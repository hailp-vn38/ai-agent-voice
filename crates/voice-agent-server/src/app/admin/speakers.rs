//! Admin Speaker profile CRUD and voiceprint metadata.

use super::*;
use axum::http::header;
use serde_json::json;
use sqlx::SqlitePool;

const MAX_DESCRIPTION: usize = 2048;

#[derive(sqlx::FromRow)]
struct SpeakerRow {
    id: i64,
    key: String,
    name: String,
    description: Option<String>,
    enabled: i64,
    revision: i64,
    created_at: i64,
    updated_at: i64,
}
#[derive(sqlx::FromRow)]
struct VoiceprintRow {
    revision: i64,
    sample_count: i64,
    embedding_space: String,
    provider_key: String,
    provider_revision: i64,
    browser_validation_status: String,
    enrolled_at: i64,
    calibration_revision: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct SpeakerCreateBody {
    key: String,
    name: String,
    #[serde(default)]
    description: Option<String>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct SpeakerPatchBody {
    #[serde(default)]
    name: Patch<String>,
    #[serde(default)]
    description: Patch<String>,
    #[serde(default)]
    enabled: Patch<bool>,
}
#[derive(Deserialize)]
pub(super) struct SpeakerListQuery {
    #[serde(default)]
    page: Option<u32>,
    #[serde(default)]
    page_size: Option<u32>,
    #[serde(default)]
    sort: Option<String>,
    #[serde(default)]
    enabled: Option<bool>,
    #[serde(default)]
    enrollment_status: Option<String>,
}
fn speaker_resource(row: &SpeakerRow, voiceprints: Vec<Value>) -> Value {
    json!({"key": row.key, "name": row.name, "description": row.description, "enabled": row.enabled != 0, "revision": row.revision, "voiceprints": voiceprints, "created_at": row.created_at, "updated_at": row.updated_at})
}

pub(super) fn with_etag(response: Response, revision: i64) -> Response {
    let Ok(value) = HeaderValue::from_str(&format!("\"{revision}\"")) else {
        return response;
    };
    let mut response = response;
    response.headers_mut().insert(header::ETAG, value);
    response
}

/// GET /speaker-recognition — the server-owned CAM++ extractor and capture limits.
pub(super) async fn summary(State(state): State<AppState>, _request: Request) -> Response {
    let config = &state.config.speaker_recognition;
    let enrollment = &config.enrollment;
    let runtime = state.speaker_runtime.as_ref();
    Json(json!({
        "available": runtime.is_some(),
        "embedding_space_id": runtime.map(|runtime| runtime.embedding_space_id()),
        "dimension": runtime.map(|runtime| runtime.dimension()),
        "enrollment": {
            "content_type": "audio/wav",
            "sample_rate": 16_000,
            "channels": 1,
            "bits_per_sample": 16,
            "min_clip_ms": enrollment.min_clip_ms,
            "max_clip_ms": enrollment.max_clip_ms,
            "min_speech_ms": enrollment.min_speech_ms,
            "max_window_ms": enrollment.max_window_ms,
            "max_body_bytes": enrollment.max_audio_body_bytes
        },
        "limits": {
            "max_speakers": config.max_speakers,
            "max_candidates_per_agent": config.max_candidates_per_agent
        }
    }))
    .into_response()
}

/// `GET /speakers` — paginated Speaker list.
pub(super) async fn list(
    State(state): State<AppState>,
    Query(query): Query<SpeakerListQuery>,
    request: Request,
) -> Response {
    let (page, page_size) = match page_bounds(query.page, query.page_size) {
        Ok(value) => value,
        Err(code) => return error(&request, StatusCode::BAD_REQUEST, code),
    };
    let order = match query.sort.as_deref().unwrap_or("key") {
        "key" => "key ASC",
        "-key" => "key DESC",
        "name" => "name ASC",
        "-name" => "name DESC",
        "updated_at" => "updated_at DESC, id DESC",
        "-updated_at" => "updated_at ASC, id ASC",
        "revision" => "revision DESC, id DESC",
        "-revision" => "revision ASC, id ASC",
        _ => return error(&request, StatusCode::BAD_REQUEST, "invalid_query"),
    };
    let pool = match db(&state) {
        Ok(pool) => pool,
        Err(response) => return response,
    };

    let mut filters: Vec<String> = Vec::new();
    if let Some(enabled) = query.enabled {
        filters.push(format!("enabled = {}", i64::from(enabled)));
    }
    match query.enrollment_status.as_deref() {
        None => {}
        Some("enrolled") => filters.push(
            "EXISTS (SELECT 1 FROM speaker_voiceprints v WHERE v.speaker_id = speakers.id)".into(),
        ),
        Some("unenrolled") => filters.push(
            "NOT EXISTS (SELECT 1 FROM speaker_voiceprints v WHERE v.speaker_id = speakers.id)"
                .into(),
        ),
        Some("draft") => return error(&request, StatusCode::BAD_REQUEST, "invalid_query"),
        Some(_) => return error(&request, StatusCode::BAD_REQUEST, "invalid_query"),
    }
    let where_clause = if filters.is_empty() {
        String::new()
    } else {
        format!(" WHERE {}", filters.join(" AND "))
    };
    let total =
        match sqlx::query_scalar::<_, i64>(&format!("SELECT COUNT(*) FROM speakers{where_clause}"))
            .fetch_one(pool)
            .await
        {
            Ok(total) => total,
            Err(error_value) => return sql_error(&request, &error_value),
        };
    let rows = sqlx::query_as::<_, SpeakerRow>(&format!(
        "SELECT id,key,name,description,enabled,revision,created_at,updated_at FROM speakers{where_clause} ORDER BY {order} LIMIT ? OFFSET ?"
    ))
    .bind(i64::from(page_size))
    .bind(i64::from((page - 1) * page_size))
    .fetch_all(pool)
    .await;
    let items = match rows {
        Ok(rows) => rows,
        Err(error_value) => return sql_error(&request, &error_value),
    };
    let items: Vec<Value> = items
        .iter()
        .map(|row| {
            json!({
                "key": row.key,
                "name": row.name,
                "description": row.description,
                "enabled": row.enabled != 0,
                "revision": row.revision,
                "updated_at": row.updated_at,
            })
        })
        .collect();
    Json(json!({
        "items": items,
        "page": page,
        "page_size": page_size,
        "total": total,
    }))
    .into_response()
}

/// `POST /speakers` — create an un-enrolled Speaker profile.
pub(super) async fn create(State(state): State<AppState>, request: Request) -> Response {
    let (request, body): (_, SpeakerCreateBody) = match json(request).await {
        Ok(value) => value,
        Err(response) => return response,
    };
    if !valid_key(&body.key)
        || !valid_text(&body.name, 128, false)
        || body
            .description
            .as_deref()
            .is_some_and(|value| !valid_text(value, MAX_DESCRIPTION, true))
    {
        return error(&request, StatusCode::BAD_REQUEST, "validation_failed");
    }
    let pool = match db(&state) {
        Ok(pool) => pool,
        Err(response) => return response,
    };
    let count: i64 = match sqlx::query_scalar("SELECT COUNT(*) FROM speakers")
        .fetch_one(pool)
        .await
    {
        Ok(count) => count,
        Err(error_value) => return sql_error(&request, &error_value),
    };
    if count >= state.config.speaker_recognition.max_speakers as i64 {
        return error(&request, StatusCode::CONFLICT, "speaker_quota_exceeded");
    }
    let time = now();
    let mut tx = match pool.begin().await {
        Ok(tx) => tx,
        Err(_) => {
            return error(
                &request,
                StatusCode::SERVICE_UNAVAILABLE,
                "database_unavailable",
            );
        }
    };
    let result = sqlx::query(
        "INSERT INTO speakers (key,name,description,enabled,revision,created_at,updated_at) VALUES (?,?,?,1,1,?,?)",
    )
    .bind(&body.key)
    .bind(&body.name)
    .bind(body.description.as_deref())
    .bind(time)
    .bind(time)
    .execute(&mut *tx)
    .await;
    let resource_id = match result {
        Ok(value) => value.last_insert_rowid(),
        Err(sqlx::Error::Database(error_value)) if error_value.is_unique_violation() => {
            return error(&request, StatusCode::CONFLICT, "speaker_key_conflict");
        }
        Err(error_value) => return mutation_sql_error(&request, &error_value),
    };
    if audit(
        &mut *tx,
        id(&request),
        "speaker",
        Some(resource_id),
        "create",
        None,
        Some(1),
        AuditOutcome::Success,
        1,
    )
    .await
    .is_err()
        || tx.commit().await.is_err()
    {
        return error(
            &request,
            StatusCode::SERVICE_UNAVAILABLE,
            "database_unavailable",
        );
    }
    match get_speaker_by(pool, &body.key).await {
        Ok(speaker) => with_etag(
            (
                StatusCode::CREATED,
                Json(speaker_resource_value(pool, &speaker).await),
            )
                .into_response(),
            speaker.revision,
        ),
        Err(_) => error(
            &request,
            StatusCode::SERVICE_UNAVAILABLE,
            "database_unavailable",
        ),
    }
}

/// `GET /speakers/{key}` — Speaker detail with voiceprint metadata.
pub(super) async fn get(
    State(state): State<AppState>,
    Path(key): Path<String>,
    request: Request,
) -> Response {
    let pool = match db(&state) {
        Ok(pool) => pool,
        Err(response) => return response,
    };
    let speaker = match get_speaker_by(pool, &key).await {
        Ok(speaker) => speaker,
        Err(sqlx::Error::RowNotFound) => {
            return error(&request, StatusCode::NOT_FOUND, "speaker_not_found");
        }
        Err(error_value) => return sql_error(&request, &error_value),
    };
    let voiceprints = match load_voiceprints(pool, speaker.id).await {
        Ok(rows) => rows,
        Err(error_value) => return sql_error(&request, &error_value),
    };
    let voiceprint_values: Vec<Value> = voiceprints
        .iter()
        .map(|voiceprint| {
            json!({
                "revision": voiceprint.revision,
                "sample_count": voiceprint.sample_count,
                "embedding_space_id": voiceprint.embedding_space,
                "enrolled_with_provider_key": voiceprint.provider_key,
                "enrolled_with_provider_revision": voiceprint.provider_revision,
                "browser_validation_status": voiceprint.browser_validation_status,
                "calibration_revision": voiceprint.calibration_revision,
                "enrolled_at": voiceprint.enrolled_at,
            })
        })
        .collect();
    with_etag(
        Json(speaker_resource(&speaker, voiceprint_values)).into_response(),
        speaker.revision,
    )
}

/// `PATCH /speakers/{key}` — CAS update of name/description/enabled.
pub(super) async fn patch(
    State(state): State<AppState>,
    Path(key): Path<String>,
    request: Request,
) -> Response {
    let (request, body): (_, SpeakerPatchBody) = match json(request).await {
        Ok(value) => value,
        Err(response) => return response,
    };
    let expected = match expected(request.headers()) {
        Ok(value) => value,
        Err(code) => return error(&request, StatusCode::BAD_REQUEST, code),
    };
    let name_patch = body.name.value();
    let description_patch = body.description.value();
    let enabled_patch = body.enabled.value();
    if name_patch.is_none() && description_patch.is_none() && enabled_patch.is_none() {
        return error(&request, StatusCode::BAD_REQUEST, "validation_failed");
    }
    let pool = match db(&state) {
        Ok(pool) => pool,
        Err(response) => return response,
    };
    let old = match get_speaker_by(pool, &key).await {
        Ok(speaker) => speaker,
        Err(sqlx::Error::RowNotFound) => {
            return error(&request, StatusCode::NOT_FOUND, "speaker_not_found");
        }
        Err(error_value) => return sql_error(&request, &error_value),
    };
    if old.revision != expected {
        audit_conflict_action(
            pool,
            id(&request).to_owned(),
            "speaker",
            old.id,
            expected,
            "update",
        )
        .await;
        return error(&request, StatusCode::CONFLICT, "revision_conflict");
    }
    let name = match name_patch {
        Some(Some(value)) if valid_text(&value, 128, false) => value,
        Some(_) => return error(&request, StatusCode::BAD_REQUEST, "validation_failed"),
        None => old.name.clone(),
    };
    let description = description_patch.unwrap_or(old.description.clone());
    if description
        .as_ref()
        .is_some_and(|value| !valid_text(value, MAX_DESCRIPTION, true))
    {
        return error(&request, StatusCode::BAD_REQUEST, "validation_failed");
    }
    let enabled = match enabled_patch {
        Some(Some(value)) => i64::from(value),
        Some(None) => return error(&request, StatusCode::BAD_REQUEST, "validation_failed"),
        None => old.enabled,
    };
    let mut tx = match pool.begin().await {
        Ok(tx) => tx,
        Err(_) => {
            return error(
                &request,
                StatusCode::SERVICE_UNAVAILABLE,
                "database_unavailable",
            );
        }
    };
    let update = sqlx::query(
        "UPDATE speakers SET name=?,description=?,enabled=?,revision=revision+1,updated_at=? WHERE id=? AND revision=?",
    )
    .bind(name)
    .bind(description)
    .bind(enabled)
    .bind(now())
    .bind(old.id)
    .bind(expected)
    .execute(&mut *tx)
    .await;
    let updated = matches!(update, Ok(result) if result.rows_affected() == 1);
    if !updated {
        let _ = tx.rollback().await;
        audit_conflict_action(
            pool,
            id(&request).to_owned(),
            "speaker",
            old.id,
            expected,
            "update",
        )
        .await;
        return error(&request, StatusCode::CONFLICT, "revision_conflict");
    }
    // Disabling a speaker revokes its authority: publish the security invalidation.
    if enabled == 0 && old.enabled != 0 && publish_catalog_revision(&mut tx).await.is_err() {
        let _ = tx.rollback().await;
        return error(
            &request,
            StatusCode::SERVICE_UNAVAILABLE,
            "database_unavailable",
        );
    }
    if audit(
        &mut *tx,
        id(&request),
        "speaker",
        Some(old.id),
        "update",
        Some(expected),
        Some(expected + 1),
        AuditOutcome::Success,
        1,
    )
    .await
    .is_err()
        || tx.commit().await.is_err()
    {
        return error(
            &request,
            StatusCode::SERVICE_UNAVAILABLE,
            "database_unavailable",
        );
    }
    // The commit published a security invalidation: revoke every session that pinned this speaker.
    if let Some(security) = security(&state) {
        security.invalidate_speaker(old.id);
    }
    match get_speaker_by(pool, &key).await {
        Ok(speaker) => with_etag(
            Json(speaker_resource(&speaker, Vec::new())).into_response(),
            speaker.revision,
        ),
        Err(_) => error(
            &request,
            StatusCode::SERVICE_UNAVAILABLE,
            "database_unavailable",
        ),
    }
}

/// `DELETE /speakers/{key}` — conditional hard delete; refused while referenced.
pub(super) async fn delete(
    State(state): State<AppState>,
    Path(key): Path<String>,
    request: Request,
) -> Response {
    let expected = match expected(request.headers()) {
        Ok(value) => value,
        Err(code) => return error(&request, StatusCode::BAD_REQUEST, code),
    };
    let pool = match db(&state) {
        Ok(pool) => pool,
        Err(response) => return response,
    };
    let old = match get_speaker_by(pool, &key).await {
        Ok(speaker) => speaker,
        Err(sqlx::Error::RowNotFound) => {
            return error(&request, StatusCode::NOT_FOUND, "speaker_not_found");
        }
        Err(error_value) => return sql_error(&request, &error_value),
    };
    if old.revision != expected {
        audit_conflict_action(
            pool,
            id(&request).to_owned(),
            "speaker",
            old.id,
            expected,
            "delete",
        )
        .await;
        return error(&request, StatusCode::CONFLICT, "revision_conflict");
    }
    let in_use: i64 = match sqlx::query_scalar(
        "SELECT (SELECT COUNT(*) FROM speaker_voiceprints WHERE speaker_id=?)+(SELECT COUNT(*) FROM agent_speaker_candidates WHERE speaker_id=?)",
    )
    .bind(old.id)
    .bind(old.id)
    .fetch_one(pool)
    .await
    {
        Ok(value) => value,
        Err(error_value) => return sql_error(&request, &error_value),
    };
    if in_use > 0 {
        return error(&request, StatusCode::CONFLICT, "speaker_in_use");
    }
    let mut tx = match pool.begin().await {
        Ok(tx) => tx,
        Err(_) => {
            return error(
                &request,
                StatusCode::SERVICE_UNAVAILABLE,
                "database_unavailable",
            );
        }
    };
    let result = sqlx::query("DELETE FROM speakers WHERE id=? AND revision=?")
        .bind(old.id)
        .bind(expected)
        .execute(&mut *tx)
        .await;
    let deleted = matches!(result, Ok(outcome) if outcome.rows_affected() == 1);
    if !deleted {
        let _ = tx.rollback().await;
        return error(&request, StatusCode::CONFLICT, "revision_conflict");
    }
    if audit(
        &mut *tx,
        id(&request),
        "speaker",
        Some(old.id),
        "delete",
        Some(expected),
        None,
        AuditOutcome::Success,
        1,
    )
    .await
    .is_err()
        || tx.commit().await.is_err()
    {
        return error(
            &request,
            StatusCode::SERVICE_UNAVAILABLE,
            "database_unavailable",
        );
    }
    StatusCode::NO_CONTENT.into_response()
}

/// The only confirmation value that permits an all-space voiceprint purge. Naming the action
/// without confirming it is a rejected request, never a second way to ask for it.
const PURGE_SPEAKER_VOICEPRINT: &str = "PURGE_SPEAKER_VOICEPRINT";

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct PurgeSpeakerVoiceprint {
    #[serde(default)]
    confirm: Option<String>,
}

/// `POST /speakers/{key}/voiceprint/purge` — explicit, confirmed removal of every voiceprint space,
/// voiceprint for one speaker. The profile, its grants and its audit
/// trail survive; so does the transcript history, which is never touched by a biometric purge.
pub(super) async fn purge(
    State(state): State<AppState>,
    Path(key): Path<String>,
    request: Request,
) -> Response {
    let (request, body): (_, PurgeSpeakerVoiceprint) = match json(request).await {
        Ok(value) => value,
        Err(response) => return response,
    };
    if body.confirm.as_deref() != Some(PURGE_SPEAKER_VOICEPRINT) {
        return error(&request, StatusCode::BAD_REQUEST, "confirmation_required");
    }
    let expected = match expected(request.headers()) {
        Ok(value) => value,
        Err(code) => return error(&request, StatusCode::BAD_REQUEST, code),
    };
    let pool = match db(&state) {
        Ok(pool) => pool,
        Err(response) => return response,
    };
    let old = match get_speaker_by(pool, &key).await {
        Ok(speaker) => speaker,
        Err(sqlx::Error::RowNotFound) => {
            return error(&request, StatusCode::NOT_FOUND, "speaker_not_found");
        }
        Err(error_value) => return sql_error(&request, &error_value),
    };
    if old.revision != expected {
        audit_conflict_action(
            pool,
            id(&request).to_owned(),
            "speaker",
            old.id,
            expected,
            "purge_voiceprint",
        )
        .await;
        return error(&request, StatusCode::CONFLICT, "revision_conflict");
    }
    let mut tx = match pool.begin().await {
        Ok(tx) => tx,
        Err(_) => {
            return error(
                &request,
                StatusCode::SERVICE_UNAVAILABLE,
                "database_unavailable",
            );
        }
    };
    let result = async {
        sqlx::query("DELETE FROM speaker_voiceprints WHERE speaker_id=?")
            .bind(old.id)
            .execute(&mut *tx)
            .await?;
        sqlx::query("UPDATE speakers SET revision=revision+1 WHERE id=?")
            .bind(old.id)
            .execute(&mut *tx)
            .await?;
        publish_catalog_revision(&mut tx).await?;
        Ok::<(), sqlx::Error>(())
    }
    .await;
    if let Err(error_value) = result {
        let _ = tx.rollback().await;
        return sql_error(&request, &error_value);
    }
    if audit(
        &mut *tx,
        id(&request),
        "speaker",
        Some(old.id),
        "purge_voiceprint",
        Some(expected),
        Some(expected + 1),
        AuditOutcome::Success,
        1,
    )
    .await
    .is_err()
        || tx.commit().await.is_err()
    {
        return error(
            &request,
            StatusCode::SERVICE_UNAVAILABLE,
            "database_unavailable",
        );
    }
    if let Some(security) = security(&state) {
        security.invalidate_speaker(old.id);
    }
    match get_speaker_by(pool, &key).await {
        Ok(speaker) => with_etag(
            Json(speaker_resource(&speaker, Vec::new())).into_response(),
            speaker.revision,
        ),
        Err(_) => error(
            &request,
            StatusCode::SERVICE_UNAVAILABLE,
            "database_unavailable",
        ),
    }
}

async fn get_speaker_by(pool: &SqlitePool, key: &str) -> Result<SpeakerRow, sqlx::Error> {
    sqlx::query_as::<_, SpeakerRow>(
        "SELECT id,key,name,description,enabled,revision,created_at,updated_at FROM speakers WHERE key=?",
    )
    .bind(key)
    .fetch_one(pool)
    .await
}

pub(super) async fn speaker_resource_by_key(
    pool: &SqlitePool,
    key: &str,
) -> Result<Value, sqlx::Error> {
    let speaker = get_speaker_by(pool, key).await?;
    Ok(speaker_resource_value(pool, &speaker).await)
}

async fn speaker_resource_value(pool: &SqlitePool, speaker: &SpeakerRow) -> Value {
    let voiceprints = load_voiceprints(pool, speaker.id).await.unwrap_or_default();
    let voiceprint_values: Vec<Value> = voiceprints
        .iter()
        .map(|voiceprint| {
            json!({
                "revision": voiceprint.revision,
                "sample_count": voiceprint.sample_count,
                "embedding_space_id": voiceprint.embedding_space,
                "enrolled_with_provider_key": voiceprint.provider_key,
                "enrolled_with_provider_revision": voiceprint.provider_revision,
                "browser_validation_status": voiceprint.browser_validation_status,
                "calibration_revision": voiceprint.calibration_revision,
                "enrolled_at": voiceprint.enrolled_at,
            })
        })
        .collect();
    speaker_resource(speaker, voiceprint_values)
}

async fn load_voiceprints(
    pool: &SqlitePool,
    speaker_id: i64,
) -> Result<Vec<VoiceprintRow>, sqlx::Error> {
    sqlx::query_as::<_, VoiceprintRow>(
        "SELECT revision,sample_count,embedding_space,provider_key,provider_revision,browser_validation_status,enrolled_at,calibration_revision FROM speaker_voiceprints WHERE speaker_id=? ORDER BY embedding_space",
    )
    .bind(speaker_id)
    .fetch_all(pool)
    .await
}

pub(super) async fn publish_catalog_revision(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
) -> Result<i64, sqlx::Error> {
    sqlx::query_scalar::<_, i64>(
        "UPDATE speaker_catalog SET revision=revision+1 WHERE id=1 RETURNING revision",
    )
    .fetch_one(&mut **tx)
    .await
}
