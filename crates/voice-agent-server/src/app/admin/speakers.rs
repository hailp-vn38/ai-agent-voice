//! Admin Speaker profiles and web enrollment drafts.
//!
//! A Speaker profile is a named voice with zero or more per-embedding-space voiceprints; until a
//! voiceprint exists the profile is not a usable candidate. Enrollment drafts are ephemeral,
//! bounded collection windows pinned to one exact provider revision and embedding space.
//!
//! See `docs/speaker-recognition-web-enrollment-implementation-guide (1).md` §5–§7.

use super::*;
use crate::audio::enrollment::{self, QualityProfile, Reject as SampleReject};
use crate::database::DesiredProvider;
use crate::services::provider_diagnostic::ProviderDiagnosticRequestError;
use crate::services::provider_runtime::RuntimeError;
use axum::body::to_bytes;
use axum::http::header;
use serde_json::json;
use sqlx::SqlitePool;

/// Extra grace before an expired draft's tombstone row is deleted, so a just-expired id answers
/// `enrollment_expired` briefly before it becomes `not_found`.
const TOMBSTONE_SECONDS: i64 = 300;
const MAX_DESCRIPTION: usize = 2048;
const MAX_DRAFT_ID: usize = 64;
/// Enrollment slots are sequential; a draft holds at most this many samples.
const MAX_SAMPLE_SLOT: u32 = 5;

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
struct ProviderSnapshot {
    id: i64,
    key: String,
    adapter: String,
    config_json: String,
    secret_ref: Option<String>,
    enabled: i64,
    revision: i64,
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
}

#[derive(sqlx::FromRow)]
struct DraftRow {
    id: String,
    provider_id: Option<i64>,
    provider_key: String,
    provider_revision: i64,
    loaded_provider_revision: Option<i64>,
    runtime_id: String,
    embedding_space: String,
    dims: Option<i64>,
    status: String,
    base_speaker_revision: i64,
    base_voiceprint_revision: Option<i64>,
    revision: i64,
    expires_at: i64,
}

#[derive(sqlx::FromRow)]
struct SampleRow {
    seq: i64,
    duration_ms: i64,
    speech_ms: i64,
}

const SAMPLE_COLUMNS: &str = "seq,duration_ms,speech_ms";

const DRAFT_COLUMNS: &str = "id,speaker_id,embedding_space,provider_id,provider_key,provider_revision,loaded_provider_revision,runtime_id,sample_rate,dims,status,committed_speaker_id,base_speaker_revision,base_voiceprint_revision,revision,created_at,expires_at,terminal_at";

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
#[serde(deny_unknown_fields)]
pub(super) struct DraftCreateBody {
    provider_key: String,
    expected_provider_revision: i64,
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
    #[serde(default)]
    provider_key: Option<String>,
}

#[derive(Deserialize)]
pub(super) struct SpeakerGetQuery {
    #[serde(default)]
    provider_key: Option<String>,
}

fn speaker_resource(row: &SpeakerRow, voiceprints: Vec<Value>, drafts: Vec<Value>) -> Value {
    json!({
        "key": row.key,
        "name": row.name,
        "description": row.description,
        "enabled": row.enabled != 0,
        "revision": row.revision,
        "voiceprints": voiceprints,
        "enrollment_drafts": drafts,
        "created_at": row.created_at,
        "updated_at": row.updated_at,
    })
}

fn draft_resource(draft: &DraftRow, speaker_key: &str, samples: &[SampleRow]) -> Value {
    json!({
        "id": draft.id,
        "speaker_key": speaker_key,
        "provider_key": draft.provider_key,
        "desired_provider_revision": draft.provider_revision,
        "loaded_provider_revision": draft.loaded_provider_revision,
        "runtime_id": draft.runtime_id,
        "embedding_space_id": draft.embedding_space,
        "dimension": draft.dims,
        "preprocessing": enrollment::PREPROCESSING_CONTRACT,
        "revision": draft.revision,
        "status": draft.status,
        "base_speaker_revision": draft.base_speaker_revision,
        "base_voiceprint_revision": draft.base_voiceprint_revision,
        "expires_at": draft.expires_at,
        "samples": samples.iter().map(sample_resource).collect::<Vec<_>>(),
    })
}

/// One stored sample as bounded metadata. Raw audio and the embedding vector never leave the
/// server; only numbers the wizard can render.
fn sample_resource(sample: &SampleRow) -> Value {
    json!({
        "slot": sample.seq,
        "status": "accepted",
        "quality": "good",
        "duration_ms": sample.duration_ms,
        "speech_ms": sample.speech_ms,
    })
}

async fn load_samples(pool: &SqlitePool, draft_id: &str) -> Result<Vec<SampleRow>, sqlx::Error> {
    sqlx::query_as::<_, SampleRow>(&format!(
        "SELECT {SAMPLE_COLUMNS} FROM speaker_enrollment_samples WHERE draft_id=? ORDER BY seq"
    ))
    .bind(draft_id)
    .fetch_all(pool)
    .await
}

/// Draft response with its stored samples. Loads them fresh so every caller stays consistent.
async fn loaded_draft_response(
    pool: &SqlitePool,
    draft: &DraftRow,
    speaker_key: &str,
    status: StatusCode,
) -> Result<Response, sqlx::Error> {
    let samples = load_samples(pool, &draft.id).await?;
    Ok(with_etag(
        (status, Json(draft_resource(draft, speaker_key, &samples))).into_response(),
        draft.revision,
    ))
}

/// Map a loaded-draft result, keeping the request borrow out of the awaited future.
fn draft_result(request: &Request, result: Result<Response, sqlx::Error>) -> Response {
    match result {
        Ok(response) => response,
        Err(error_value) => sql_error(request, &error_value),
    }
}

pub(super) fn with_etag(response: Response, revision: i64) -> Response {
    let Ok(value) = HeaderValue::from_str(&format!("\"{revision}\"")) else {
        return response;
    };
    let mut response = response;
    response.headers_mut().insert(header::ETAG, value);
    response
}

/// `GET /speaker-recognition` — bounded enrollment config and runtime capability summary.
pub(super) async fn summary(State(state): State<AppState>, request: Request) -> Response {
    let pool = match db(&state) {
        Ok(pool) => pool,
        Err(response) => return response,
    };
    let config = &state.config.speaker_recognition;
    let providers = sqlx::query_as::<_, (i64, String, String, i64)>(
        "SELECT id,key,adapter,revision FROM providers WHERE type='speaker' ORDER BY key",
    )
    .fetch_all(pool)
    .await;
    let providers = match providers {
        Ok(rows) => rows,
        Err(error_value) => return sql_error(&request, &error_value),
    };

    let mut catalog_revision = 0_i64;
    let mut provider_entries = Vec::with_capacity(providers.len());
    for (id, key, adapter, revision) in providers {
        catalog_revision = catalog_revision.max(revision);
        let ready = state
            .provider_runtime_manager
            .as_ref()
            .is_some_and(|manager| {
                manager
                    .inspect(id, revision)
                    .ready_revisions
                    .contains(&revision)
            });
        provider_entries.push(json!({
            "provider_key": key,
            "provider_revision": revision,
            "adapter": adapter,
            "state": if ready { "loaded" } else { "cold" },
        }));
    }

    let available = state.provider_runtime_manager.is_some();
    let enrollment = &config.enrollment;
    Json(json!({
        "available": available,
        "runtime_mode": if available { "managed" } else { "unavailable" },
        "providers": provider_entries,
        "enrollment": {
            "content_type": "audio/wav",
            "sample_rate": 16_000,
            "channels": 1,
            "bits_per_sample": 16,
            "min_samples": enrollment.min_samples,
            "max_samples": enrollment.max_samples,
            "min_clip_ms": enrollment.min_clip_ms,
            "max_clip_ms": enrollment.max_clip_ms,
            "min_speech_ms": enrollment.min_speech_ms,
            "max_window_ms": enrollment.max_window_ms,
            "ttl_ms": enrollment.ttl_ms,
            "max_body_bytes": enrollment.max_audio_body_bytes,
        },
        "limits": {
            "max_speakers": config.max_speakers,
            "max_voiceprint_spaces_per_speaker": config.max_voiceprint_spaces_per_speaker,
            "max_candidates_per_agent": config.max_candidates_per_agent,
        },
        "catalog_revision": catalog_revision,
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
        Some("draft") => filters.push(format!(
            "EXISTS (SELECT 1 FROM speaker_enrollment_drafts d WHERE d.speaker_id = speakers.id AND d.status = 'collecting' AND d.expires_at > {})",
            now()
        )),
        Some(_) => return error(&request, StatusCode::BAD_REQUEST, "invalid_query"),
    }
    if let Some(provider_key) = query.provider_key.as_deref() {
        if !valid_key(provider_key) {
            return error(&request, StatusCode::BAD_REQUEST, "invalid_query");
        }
        // `valid_key` guarantees `[A-Za-z0-9_-]`, so this interpolation is injection-safe.
        filters.push(format!(
            "(EXISTS (SELECT 1 FROM speaker_voiceprints v WHERE v.speaker_id = speakers.id AND v.provider_key = '{provider_key}') OR EXISTS (SELECT 1 FROM speaker_enrollment_drafts d WHERE d.speaker_id = speakers.id AND d.provider_key = '{provider_key}'))"
        ));
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

/// `GET /speakers/{key}` — Speaker detail with voiceprint and draft metadata.
pub(super) async fn get(
    State(state): State<AppState>,
    Path(key): Path<String>,
    Query(query): Query<SpeakerGetQuery>,
    request: Request,
) -> Response {
    if query
        .provider_key
        .as_deref()
        .is_some_and(|value| !valid_key(value))
    {
        return error(&request, StatusCode::BAD_REQUEST, "invalid_query");
    }
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
    let voiceprints = match load_voiceprints(pool, speaker.id, query.provider_key.as_deref()).await
    {
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
                "enrolled_at": voiceprint.enrolled_at,
            })
        })
        .collect();
    let drafts = match load_open_drafts(pool, speaker.id).await {
        Ok(rows) => rows,
        Err(error_value) => return sql_error(&request, &error_value),
    };
    let draft_values: Vec<Value> = drafts
        .iter()
        .map(|draft| {
            json!({
                "id": draft.id,
                "status": draft.status,
                "revision": draft.revision,
                "expires_at": draft.expires_at,
            })
        })
        .collect();
    with_etag(
        Json(speaker_resource(&speaker, voiceprint_values, draft_values)).into_response(),
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
    match get_speaker_by(pool, &key).await {
        Ok(speaker) => with_etag(
            Json(speaker_resource(&speaker, Vec::new(), Vec::new())).into_response(),
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
        "SELECT (SELECT COUNT(*) FROM speaker_voiceprints WHERE speaker_id=?)+(SELECT COUNT(*) FROM agent_speaker_candidates WHERE speaker_id=?)+(SELECT COUNT(*) FROM speaker_enrollment_drafts WHERE speaker_id=? AND status='collecting')",
    )
    .bind(old.id)
    .bind(old.id)
    .bind(old.id)
    .fetch_one(pool)
    .await
    {
        Ok(value) => value,
        Err(error_value) => return sql_error(&request, &error_value),
    };
    if in_use > 0 {
        return error(&request, StatusCode::CONFLICT, "resource_in_use");
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

/// `POST /speakers/{key}/enrollments` — open a draft pinned to an exact provider revision.
pub(super) async fn create_draft(
    State(state): State<AppState>,
    Path(key): Path<String>,
    request: Request,
) -> Response {
    let (request, body): (_, DraftCreateBody) = match json(request).await {
        Ok(value) => value,
        Err(response) => return response,
    };
    let expected = match expected(request.headers()) {
        Ok(value) => value,
        Err(code) => return error(&request, StatusCode::BAD_REQUEST, code),
    };
    if !valid_key(&body.provider_key) || body.expected_provider_revision <= 0 {
        return error(&request, StatusCode::BAD_REQUEST, "validation_failed");
    }
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
    if speaker.revision != expected {
        audit_conflict_action(
            pool,
            id(&request).to_owned(),
            "speaker",
            speaker.id,
            expected,
            "create_enrollment",
        )
        .await;
        return error(&request, StatusCode::CONFLICT, "revision_conflict");
    }
    let provider = match load_speaker_provider(pool, &body.provider_key).await {
        Ok(Some(provider)) => provider,
        Ok(None) => return error(&request, StatusCode::NOT_FOUND, "provider_not_found"),
        Err(error_value) => return sql_error(&request, &error_value),
    };
    if provider.enabled == 0 {
        return error(&request, StatusCode::CONFLICT, "provider_disabled");
    }
    if provider.revision != body.expected_provider_revision {
        return error(&request, StatusCode::CONFLICT, "provider_revision_conflict");
    }
    if let Some(existing) = match load_open_draft_for_speaker(pool, speaker.id).await {
        Ok(existing) => existing,
        Err(error_value) => return sql_error(&request, &error_value),
    } {
        return (
            StatusCode::CONFLICT,
            Json(json!({"error":{"code":"enrollment_in_progress","request_id":id(&request),"enrollment_id":existing.id}})),
        )
            .into_response();
    }
    let open_drafts: i64 = match sqlx::query_scalar(
        "SELECT COUNT(*) FROM speaker_enrollment_drafts WHERE status='collecting' AND expires_at>?",
    )
    .bind(now())
    .fetch_one(pool)
    .await
    {
        Ok(value) => value,
        Err(error_value) => return sql_error(&request, &error_value),
    };
    if open_drafts
        >= state
            .config
            .speaker_recognition
            .enrollment
            .max_open_enrollments as i64
    {
        return error(&request, StatusCode::CONFLICT, "enrollment_quota_exceeded");
    }
    let (embedding_space, dims) = match acquire_embedding_space(&state, &provider).await {
        Ok(pair) => pair,
        Err((status, code)) => return error(&request, status, code),
    };
    let ttl_seconds = (state.config.speaker_recognition.enrollment.ttl_ms / 1000).max(1) as i64;
    let time = now();
    let draft_id = uuid::Uuid::new_v4().to_string();
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
    let insert = sqlx::query(
        "INSERT INTO speaker_enrollment_drafts (id,speaker_id,embedding_space,provider_id,provider_key,provider_revision,loaded_provider_revision,runtime_id,sample_rate,dims,status,base_speaker_revision,revision,created_at,expires_at) VALUES (?,?,?,?,?,?,?,?,16000,?,'collecting',?,1,?,?)",
    )
    .bind(&draft_id)
    .bind(speaker.id)
    .bind(&embedding_space)
    .bind(provider.id)
    .bind(&provider.key)
    .bind(provider.revision)
    .bind(provider.revision)
    .bind(&state.runtime_id)
    .bind(dims)
    .bind(speaker.revision)
    .bind(time)
    .bind(time + ttl_seconds)
    .execute(&mut *tx)
    .await;
    if let Err(error_value) = insert {
        return mutation_sql_error(&request, &error_value);
    }
    if audit(
        &mut *tx,
        id(&request),
        "speaker_enrollment",
        Some(speaker.id),
        "create",
        Some(speaker.revision),
        Some(speaker.revision),
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
    match load_draft(pool, &draft_id).await {
        Ok(Some(draft)) => {
            let loaded =
                loaded_draft_response(pool, &draft, &speaker.key, StatusCode::CREATED).await;
            draft_result(&request, loaded)
        }
        _ => error(
            &request,
            StatusCode::SERVICE_UNAVAILABLE,
            "database_unavailable",
        ),
    }
}

/// `GET /speakers/{key}/enrollments/{id}` — resume a draft, repinning a compatible runtime.
pub(super) async fn get_draft(
    State(state): State<AppState>,
    Path((key, draft_id)): Path<(String, String)>,
    request: Request,
) -> Response {
    if draft_id.len() > MAX_DRAFT_ID {
        return error(&request, StatusCode::NOT_FOUND, "enrollment_not_found");
    }
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
    let draft = match load_draft(pool, &draft_id).await {
        Ok(Some(draft)) if draft.status == "expired" => {
            return error(&request, StatusCode::GONE, "enrollment_expired");
        }
        Ok(Some(draft)) if draft.status != "collecting" => {
            return error(&request, StatusCode::CONFLICT, "enrollment_committed");
        }
        Ok(Some(draft)) if draft.expires_at <= now() => {
            expire_draft(pool, &draft.id).await;
            return error(&request, StatusCode::GONE, "enrollment_expired");
        }
        Ok(Some(draft)) => draft,
        Ok(None) => return error(&request, StatusCode::NOT_FOUND, "enrollment_not_found"),
        Err(error_value) => return sql_error(&request, &error_value),
    };
    if draft.runtime_id == state.runtime_id {
        let loaded = loaded_draft_response(pool, &draft, &speaker.key, StatusCode::OK).await;
        return draft_result(&request, loaded);
    }
    // Compatible runtime repin: another process incarnation opened this draft, so re-pin it to this
    // one only when the exact provider revision, embedding space and dimension still agree.
    let compatible = match load_speaker_provider(pool, &draft.provider_key).await {
        Ok(Some(provider))
            if provider.enabled != 0
                && provider.revision == draft.provider_revision
                && Some(provider.id) == draft.provider_id =>
        {
            acquire_embedding_space(&state, &provider)
                .await
                .ok()
                .filter(|(space, _)| *space == draft.embedding_space)
                .map(|(_, dims)| (provider.id, dims))
        }
        Ok(_) => None,
        Err(_) => None,
    };
    let Some((provider_id, dims)) = compatible else {
        return error(
            &request,
            StatusCode::CONFLICT,
            "enrollment_runtime_incompatible",
        );
    };
    let update = sqlx::query(
        "UPDATE speaker_enrollment_drafts SET runtime_id=?,provider_id=?,dims=?,revision=revision+1 WHERE id=? AND status='collecting' AND revision=?",
    )
    .bind(&state.runtime_id)
    .bind(provider_id)
    .bind(dims)
    .bind(&draft.id)
    .bind(draft.revision)
    .execute(pool)
    .await;
    match update {
        Ok(outcome) if outcome.rows_affected() == 1 => {}
        Ok(_) => return error(&request, StatusCode::CONFLICT, "revision_conflict"),
        Err(error_value) => return sql_error(&request, &error_value),
    }
    match load_draft(pool, &draft_id).await {
        Ok(Some(draft)) => {
            let loaded = loaded_draft_response(pool, &draft, &speaker.key, StatusCode::OK).await;
            draft_result(&request, loaded)
        }
        _ => error(
            &request,
            StatusCode::SERVICE_UNAVAILABLE,
            "database_unavailable",
        ),
    }
}

/// `DELETE /speakers/{key}/enrollments/{id}` — cancel a collecting draft.
pub(super) async fn cancel_draft(
    State(state): State<AppState>,
    Path((key, draft_id)): Path<(String, String)>,
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
    let speaker = match get_speaker_by(pool, &key).await {
        Ok(speaker) => speaker,
        Err(sqlx::Error::RowNotFound) => {
            return error(&request, StatusCode::NOT_FOUND, "speaker_not_found");
        }
        Err(error_value) => return sql_error(&request, &error_value),
    };
    let draft = match load_draft(pool, &draft_id).await {
        Ok(Some(draft)) if draft.status == "expired" => {
            return error(&request, StatusCode::GONE, "enrollment_expired");
        }
        Ok(Some(draft)) if draft.status != "collecting" => {
            return error(&request, StatusCode::CONFLICT, "enrollment_committed");
        }
        Ok(Some(draft)) => draft,
        Ok(None) => return error(&request, StatusCode::NOT_FOUND, "enrollment_not_found"),
        Err(error_value) => return sql_error(&request, &error_value),
    };
    if draft.revision != expected {
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
    let result = sqlx::query(
        "DELETE FROM speaker_enrollment_drafts WHERE id=? AND status='collecting' AND revision=?",
    )
    .bind(&draft.id)
    .bind(expected)
    .execute(&mut *tx)
    .await;
    let cancelled = matches!(result, Ok(outcome) if outcome.rows_affected() == 1);
    if !cancelled {
        let _ = tx.rollback().await;
        return error(&request, StatusCode::CONFLICT, "revision_conflict");
    }
    let _ = sqlx::query("DELETE FROM speaker_enrollment_samples WHERE draft_id=?")
        .bind(&draft.id)
        .execute(&mut *tx)
        .await;
    if audit(
        &mut *tx,
        id(&request),
        "speaker_enrollment",
        Some(speaker.id),
        "cancel",
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

/// `PUT /speakers/{key}/enrollments/{id}/samples/{slot}` — bounded raw WAV upload.
///
/// The body is the recorder's PCM16 mono 16 kHz WAV. The server parses it, runs the quality gate,
/// asks the bounded worker for one embedding, and stores only bounded metadata plus the vector.
/// A rejected request never mutates the draft.
pub(super) async fn put_sample(
    State(state): State<AppState>,
    Path((key, draft_id, slot)): Path<(String, String, u32)>,
    request: Request,
) -> Response {
    let expected = match expected(request.headers()) {
        Ok(value) => value,
        Err(code) => return error(&request, StatusCode::BAD_REQUEST, code),
    };
    if !(1..=MAX_SAMPLE_SLOT).contains(&slot) {
        return error(&request, StatusCode::BAD_REQUEST, "validation_failed");
    }
    if !wav_content_type(request.headers()) {
        return error(
            &request,
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            "unsupported_audio_format",
        );
    }
    if request
        .headers()
        .get(header::CONTENT_ENCODING)
        .is_some_and(|encoding| encoding.as_bytes() != b"identity")
    {
        return error(
            &request,
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            "unsupported_content_encoding",
        );
    }
    if draft_id.len() > MAX_DRAFT_ID {
        return error(&request, StatusCode::NOT_FOUND, "enrollment_not_found");
    }
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
    let draft = match load_draft(pool, &draft_id).await {
        Ok(Some(draft)) if draft.status == "expired" => {
            return error(&request, StatusCode::GONE, "enrollment_expired");
        }
        Ok(Some(draft)) if draft.status != "collecting" => {
            return error(&request, StatusCode::CONFLICT, "enrollment_committed");
        }
        Ok(Some(draft)) if draft.expires_at <= now() => {
            expire_draft(pool, &draft.id).await;
            return error(&request, StatusCode::GONE, "enrollment_expired");
        }
        Ok(Some(draft)) => draft,
        Ok(None) => return error(&request, StatusCode::NOT_FOUND, "enrollment_not_found"),
        Err(error_value) => return sql_error(&request, &error_value),
    };
    if draft.runtime_id != state.runtime_id {
        return error(
            &request,
            StatusCode::CONFLICT,
            "enrollment_runtime_incompatible",
        );
    }
    if draft.revision != expected {
        return error(&request, StatusCode::CONFLICT, "revision_conflict");
    }
    let provider_revision = match draft.loaded_provider_revision {
        Some(revision) => revision,
        None => {
            return error(
                &request,
                StatusCode::CONFLICT,
                "enrollment_runtime_incompatible",
            );
        }
    };
    let dims = draft.dims.unwrap_or_default() as usize;

    // Split the body off but keep a Request for error responses and tracing.
    let (parts, body) = request.into_parts();
    let request = Request::from_parts(parts, Body::empty());
    let limit = state
        .config
        .speaker_recognition
        .enrollment
        .max_audio_body_bytes;
    let body = match to_bytes(body, limit).await {
        Ok(body) => body,
        Err(_) => {
            return error(&request, StatusCode::PAYLOAD_TOO_LARGE, "request_too_large");
        }
    };
    let (clip, duration_ms) = match enrollment::parse_wav(&body) {
        Ok(value) => value,
        Err(_) => {
            return error(
                &request,
                StatusCode::UNSUPPORTED_MEDIA_TYPE,
                "unsupported_audio_format",
            );
        }
    };
    let config = &state.config.speaker_recognition.enrollment;
    let profile = QualityProfile {
        min_clip_ms: config.min_clip_ms,
        max_clip_ms: config.max_clip_ms,
        min_speech_ms: config.min_speech_ms,
        max_window_ms: config.max_window_ms,
    };
    let analyzed = match enrollment::analyze(&clip, duration_ms, &profile) {
        Ok(analyzed) => analyzed,
        Err(SampleReject::Clipped) => {
            return error(
                &request,
                StatusCode::UNPROCESSABLE_ENTITY,
                "speaker_audio_clipped",
            );
        }
        Err(_) => {
            return error(
                &request,
                StatusCode::UNPROCESSABLE_ENTITY,
                "speaker_insufficient_audio",
            );
        }
    };

    let extracted = match state
        .provider_diagnostics
        .extract_speaker_embedding(&draft.provider_key, provider_revision, analyzed.window)
        .await
    {
        Ok(extracted) => extracted,
        Err(error_value) => return enrollment_error_response(&request, error_value),
    };
    if extracted.embedding_space_id != draft.embedding_space || extracted.dimension != dims {
        return error(
            &request,
            StatusCode::CONFLICT,
            "enrollment_runtime_incompatible",
        );
    }
    if enrollment::validate_embedding(&extracted.embedding, dims).is_err() {
        return error(
            &request,
            StatusCode::BAD_GATEWAY,
            "speaker_inference_failed",
        );
    }
    let mut embedding = extracted.embedding;
    if !enrollment::normalize(&mut embedding) {
        return error(
            &request,
            StatusCode::BAD_GATEWAY,
            "speaker_inference_failed",
        );
    }
    let vector = enrollment::encode_embedding(&embedding);

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
    let inserted = sqlx::query(
        "INSERT INTO speaker_enrollment_samples (draft_id,seq,duration_ms,speech_ms,vector,created_at) VALUES (?,?,?,?,?,?) \
         ON CONFLICT(draft_id,seq) DO UPDATE SET duration_ms=excluded.duration_ms,speech_ms=excluded.speech_ms,vector=excluded.vector",
    )
    .bind(&draft.id)
    .bind(i64::from(slot))
    .bind(analyzed.quality.duration_ms as i64)
    .bind(analyzed.quality.speech_ms as i64)
    .bind(&vector)
    .bind(now())
    .execute(&mut *tx)
    .await;
    let bumped = sqlx::query(
        "UPDATE speaker_enrollment_drafts SET revision=revision+1 WHERE id=? AND status='collecting' AND revision=?",
    )
    .bind(&draft.id)
    .bind(expected)
    .execute(&mut *tx)
    .await;
    let (inserted, bumped) = match (inserted, bumped) {
        (Ok(inserted), Ok(bumped)) => (inserted, bumped),
        _ => {
            let _ = tx.rollback().await;
            return error(
                &request,
                StatusCode::SERVICE_UNAVAILABLE,
                "database_unavailable",
            );
        }
    };
    if inserted.rows_affected() == 0 || bumped.rows_affected() != 1 {
        let _ = tx.rollback().await;
        return error(&request, StatusCode::CONFLICT, "revision_conflict");
    }
    if audit(
        &mut *tx,
        id(&request),
        "speaker_enrollment",
        Some(speaker.id),
        "sample_upload",
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

    match load_draft(pool, &draft.id).await {
        Ok(Some(draft)) => {
            let samples = match load_samples(pool, &draft.id).await {
                Ok(samples) => samples,
                Err(error_value) => return sql_error(&request, &error_value),
            };
            with_etag(
                (
                    StatusCode::OK,
                    Json(draft_resource(&draft, &speaker.key, &samples)),
                )
                    .into_response(),
                draft.revision,
            )
        }
        _ => error(
            &request,
            StatusCode::SERVICE_UNAVAILABLE,
            "database_unavailable",
        ),
    }
}

/// `DELETE /speakers/{key}/enrollments/{id}/samples/{slot}` — remove one stored sample.
pub(super) async fn delete_sample(
    State(state): State<AppState>,
    Path((key, draft_id, slot)): Path<(String, String, u32)>,
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
    let speaker = match get_speaker_by(pool, &key).await {
        Ok(speaker) => speaker,
        Err(sqlx::Error::RowNotFound) => {
            return error(&request, StatusCode::NOT_FOUND, "speaker_not_found");
        }
        Err(error_value) => return sql_error(&request, &error_value),
    };
    let draft = match load_draft(pool, &draft_id).await {
        Ok(Some(draft)) if draft.status == "expired" => {
            return error(&request, StatusCode::GONE, "enrollment_expired");
        }
        Ok(Some(draft)) if draft.status != "collecting" => {
            return error(&request, StatusCode::CONFLICT, "enrollment_committed");
        }
        Ok(Some(draft)) if draft.expires_at <= now() => {
            expire_draft(pool, &draft.id).await;
            return error(&request, StatusCode::GONE, "enrollment_expired");
        }
        Ok(Some(draft)) => draft,
        Ok(None) => return error(&request, StatusCode::NOT_FOUND, "enrollment_not_found"),
        Err(error_value) => return sql_error(&request, &error_value),
    };
    if draft.revision != expected {
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
    let removed = sqlx::query("DELETE FROM speaker_enrollment_samples WHERE draft_id=? AND seq=?")
        .bind(&draft.id)
        .bind(i64::from(slot))
        .execute(&mut *tx)
        .await;
    let bumped = sqlx::query(
        "UPDATE speaker_enrollment_drafts SET revision=revision+1 WHERE id=? AND status='collecting' AND revision=?",
    )
    .bind(&draft.id)
    .bind(expected)
    .execute(&mut *tx)
    .await;
    let (Ok(removed), Ok(bumped)) = (removed, bumped) else {
        let _ = tx.rollback().await;
        return error(
            &request,
            StatusCode::SERVICE_UNAVAILABLE,
            "database_unavailable",
        );
    };
    if removed.rows_affected() == 0 {
        let _ = tx.rollback().await;
        return error(&request, StatusCode::NOT_FOUND, "sample_not_found");
    }
    if bumped.rows_affected() != 1 {
        let _ = tx.rollback().await;
        return error(&request, StatusCode::CONFLICT, "revision_conflict");
    }
    if audit(
        &mut *tx,
        id(&request),
        "speaker_enrollment",
        Some(speaker.id),
        "sample_delete",
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
    match load_draft(pool, &draft.id).await {
        Ok(Some(draft)) => {
            let samples = match load_samples(pool, &draft.id).await {
                Ok(samples) => samples,
                Err(error_value) => return sql_error(&request, &error_value),
            };
            with_etag(
                (
                    StatusCode::OK,
                    Json(draft_resource(&draft, &speaker.key, &samples)),
                )
                    .into_response(),
                draft.revision,
            )
        }
        _ => error(
            &request,
            StatusCode::SERVICE_UNAVAILABLE,
            "database_unavailable",
        ),
    }
}

/// The recorder only ever sends `audio/wav`; parameters such as `codecs=1` are tolerated.
fn wav_content_type(headers: &HeaderMap) -> bool {
    headers
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split(';').next())
        .map(str::trim)
        .is_some_and(|value| value.eq_ignore_ascii_case("audio/wav"))
}

fn enrollment_error_response(
    request: &Request,
    error_value: ProviderDiagnosticRequestError,
) -> Response {
    let (status, code) = enrollment_error_status(error_value);
    error(request, status, code)
}

/// HTTP mapping for a failed enrollment extraction, kept pure so it is unit-testable.
fn enrollment_error_status(
    error_value: ProviderDiagnosticRequestError,
) -> (StatusCode, &'static str) {
    use crate::services::provider_diagnostic::ProviderDiagnosticError;
    match error_value {
        ProviderDiagnosticRequestError::RevisionConflict => {
            (StatusCode::CONFLICT, "revision_conflict")
        }
        ProviderDiagnosticRequestError::Runtime(RuntimeError::Busy)
        | ProviderDiagnosticRequestError::Diagnostic(ProviderDiagnosticError::Busy) => {
            (StatusCode::TOO_MANY_REQUESTS, "provider_runtime_busy")
        }
        ProviderDiagnosticRequestError::Runtime(RuntimeError::Timeout)
        | ProviderDiagnosticRequestError::Diagnostic(ProviderDiagnosticError::Timeout) => {
            (StatusCode::GATEWAY_TIMEOUT, "speaker_inference_timeout")
        }
        ProviderDiagnosticRequestError::Diagnostic(ProviderDiagnosticError::InvalidResponse) => {
            (StatusCode::BAD_GATEWAY, "speaker_inference_failed")
        }
        ProviderDiagnosticRequestError::Runtime(error_value) => {
            (StatusCode::SERVICE_UNAVAILABLE, error_value.code())
        }
        _ => (
            StatusCode::SERVICE_UNAVAILABLE,
            "speaker_runtime_unavailable",
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

async fn speaker_resource_value(pool: &SqlitePool, speaker: &SpeakerRow) -> Value {
    let voiceprints = load_voiceprints(pool, speaker.id, None)
        .await
        .unwrap_or_default();
    let drafts = load_open_drafts(pool, speaker.id).await.unwrap_or_default();
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
                "enrolled_at": voiceprint.enrolled_at,
            })
        })
        .collect();
    let draft_values: Vec<Value> = drafts
        .iter()
        .map(|draft| {
            json!({
                "id": draft.id,
                "status": draft.status,
                "revision": draft.revision,
                "expires_at": draft.expires_at,
            })
        })
        .collect();
    speaker_resource(speaker, voiceprint_values, draft_values)
}

async fn load_voiceprints(
    pool: &SqlitePool,
    speaker_id: i64,
    provider_key: Option<&str>,
) -> Result<Vec<VoiceprintRow>, sqlx::Error> {
    sqlx::query_as::<_, VoiceprintRow>(
        "SELECT revision,sample_count,embedding_space,provider_key,provider_revision,browser_validation_status,enrolled_at FROM speaker_voiceprints WHERE speaker_id=? AND (? IS NULL OR provider_key=?) ORDER BY embedding_space",
    )
    .bind(speaker_id)
    .bind(provider_key)
    .bind(provider_key)
    .fetch_all(pool)
    .await
}

async fn load_open_drafts(
    pool: &SqlitePool,
    speaker_id: i64,
) -> Result<Vec<DraftRow>, sqlx::Error> {
    sqlx::query_as::<_, DraftRow>(&format!(
        "SELECT {DRAFT_COLUMNS} FROM speaker_enrollment_drafts WHERE speaker_id=? AND status='collecting' AND expires_at>? ORDER BY created_at,id"
    ))
    .bind(speaker_id)
    .bind(now())
    .fetch_all(pool)
    .await
}

async fn load_open_draft_for_speaker(
    pool: &SqlitePool,
    speaker_id: i64,
) -> Result<Option<DraftRow>, sqlx::Error> {
    sqlx::query_as::<_, DraftRow>(&format!(
        "SELECT {DRAFT_COLUMNS} FROM speaker_enrollment_drafts WHERE speaker_id=? AND status='collecting' AND expires_at>? LIMIT 1"
    ))
    .bind(speaker_id)
    .bind(now())
    .fetch_optional(pool)
    .await
}

async fn load_draft(pool: &SqlitePool, id: &str) -> Result<Option<DraftRow>, sqlx::Error> {
    sqlx::query_as::<_, DraftRow>(&format!(
        "SELECT {DRAFT_COLUMNS} FROM speaker_enrollment_drafts WHERE id=?"
    ))
    .bind(id)
    .fetch_optional(pool)
    .await
}

async fn load_speaker_provider(
    pool: &SqlitePool,
    key: &str,
) -> Result<Option<ProviderSnapshot>, sqlx::Error> {
    sqlx::query_as::<_, ProviderSnapshot>(
        "SELECT id,key,adapter,config_json,secret_ref,enabled,revision FROM providers WHERE key=? AND type='speaker'",
    )
    .bind(key)
    .fetch_optional(pool)
    .await
}

/// Acquire the exact provider revision and read its embedding space identity. The lease is dropped
/// before returning so no runtime slot is held while the draft transaction runs.
async fn acquire_embedding_space(
    state: &AppState,
    provider: &ProviderSnapshot,
) -> Result<(String, i64), (StatusCode, &'static str)> {
    let manager = state.provider_runtime_manager.as_ref().ok_or((
        StatusCode::SERVICE_UNAVAILABLE,
        "provider_runtime_unavailable",
    ))?;
    let desired = DesiredProvider {
        id: provider.id,
        key: provider.key.clone(),
        kind: "speaker".to_owned(),
        adapter: provider.adapter.clone(),
        config_json: provider.config_json.clone(),
        secret_ref: provider.secret_ref.clone(),
        revision: provider.revision,
    };
    let lease = manager
        .acquire(desired)
        .await
        .map_err(|error_value| match error_value {
            RuntimeError::Busy => (StatusCode::TOO_MANY_REQUESTS, "provider_runtime_busy"),
            RuntimeError::Timeout => (StatusCode::GATEWAY_TIMEOUT, "provider_runtime_timeout"),
            RuntimeError::Configuration => (StatusCode::CONFLICT, "provider_config_invalid"),
            RuntimeError::ShuttingDown => {
                (StatusCode::SERVICE_UNAVAILABLE, "server_is_shutting_down")
            }
            _ => (
                StatusCode::SERVICE_UNAVAILABLE,
                "provider_runtime_unavailable",
            ),
        })?;
    let runtime = lease
        .runtimes()
        .and_then(|catalog| catalog.speaker(&provider.key))
        .ok_or((
            StatusCode::SERVICE_UNAVAILABLE,
            "provider_runtime_unavailable",
        ))?;
    Ok((
        runtime.embedding_space_id().to_owned(),
        runtime.dimension() as i64,
    ))
}

async fn expire_draft(pool: &SqlitePool, id: &str) {
    let _ = sqlx::query(
        "UPDATE speaker_enrollment_drafts SET status='expired',terminal_at=? WHERE id=? AND status='collecting'",
    )
    .bind(now())
    .bind(id)
    .execute(pool)
    .await;
    let _ = sqlx::query("DELETE FROM speaker_enrollment_samples WHERE draft_id=?")
        .bind(id)
        .execute(pool)
        .await;
}

/// Best-effort sweep of expired drafts and past-tombstone rows. Called at startup and on the
/// five-minute maintenance tick, never inside a request path except one draft's own expiry check.
pub(crate) async fn cleanup_expired_drafts(pool: &SqlitePool) {
    let cutoff = now();
    let _ = sqlx::query(
        "UPDATE speaker_enrollment_drafts SET status='expired',terminal_at=? WHERE status='collecting' AND expires_at<=?",
    )
    .bind(cutoff)
    .bind(cutoff)
    .execute(pool)
    .await;
    let _ = sqlx::query(
        "DELETE FROM speaker_enrollment_samples WHERE draft_id IN (SELECT id FROM speaker_enrollment_drafts WHERE status IN ('expired','committed'))",
    )
    .execute(pool)
    .await;
    let _ = sqlx::query(
        "DELETE FROM speaker_enrollment_drafts WHERE status IN ('expired','committed') AND terminal_at IS NOT NULL AND terminal_at<=?",
    )
    .bind(cutoff - TOMBSTONE_SECONDS)
    .execute(pool)
    .await;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::provider_diagnostic::{
        ProviderDiagnosticError, ProviderDiagnosticRequestError,
    };

    #[test]
    fn enrollment_extraction_failures_map_to_bounded_status_codes() {
        let cases = [
            (
                ProviderDiagnosticRequestError::RevisionConflict,
                StatusCode::CONFLICT,
                "revision_conflict",
            ),
            (
                ProviderDiagnosticRequestError::Runtime(RuntimeError::Busy),
                StatusCode::TOO_MANY_REQUESTS,
                "provider_runtime_busy",
            ),
            (
                ProviderDiagnosticRequestError::Runtime(RuntimeError::Timeout),
                StatusCode::GATEWAY_TIMEOUT,
                "speaker_inference_timeout",
            ),
            (
                ProviderDiagnosticRequestError::Diagnostic(ProviderDiagnosticError::Timeout),
                StatusCode::GATEWAY_TIMEOUT,
                "speaker_inference_timeout",
            ),
            (
                ProviderDiagnosticRequestError::Diagnostic(
                    ProviderDiagnosticError::InvalidResponse,
                ),
                StatusCode::BAD_GATEWAY,
                "speaker_inference_failed",
            ),
            (
                ProviderDiagnosticRequestError::SpeakerManagerRequired,
                StatusCode::SERVICE_UNAVAILABLE,
                "speaker_runtime_unavailable",
            ),
        ];
        for (error_value, status, code) in cases {
            assert_eq!(enrollment_error_status(error_value), (status, code));
        }
    }

    #[test]
    fn wav_content_type_tolerates_parameters_only_for_audio_wav() {
        let mut headers = HeaderMap::new();
        assert!(!wav_content_type(&headers));
        headers.insert(header::CONTENT_TYPE, "audio/wav; codecs=1".parse().unwrap());
        assert!(wav_content_type(&headers));
        headers.insert(header::CONTENT_TYPE, "audio/webm".parse().unwrap());
        assert!(!wav_content_type(&headers));
    }
}
