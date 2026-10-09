//! Admin Speaker profile CRUD and voiceprint metadata.

use super::*;
use axum::http::header;
use serde_json::json;

const MAX_DESCRIPTION: usize = 2048;

use crate::database::speakers::{
    SpeakerChanges, SpeakerInput, SpeakerRow, get_speaker_by, load_voiceprints,
};
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
    if query.sort.as_deref().is_some_and(|sort| {
        !matches!(
            sort,
            "key"
                | "-key"
                | "name"
                | "-name"
                | "updated_at"
                | "-updated_at"
                | "revision"
                | "-revision"
        )
    }) {
        return error(&request, StatusCode::BAD_REQUEST, "invalid_query");
    }
    let database = match database(&state) {
        Ok(pool) => pool,
        Err(response) => return response,
    };

    let (total, items) = match database
        .list_speakers(
            query.sort.as_deref(),
            query.enabled,
            query.enrollment_status.as_deref(),
            page,
            page_size,
        )
        .await
    {
        Ok(value) => value,
        Err(cause) => return write_error(&request, cause),
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
    let database = match database(&state) {
        Ok(pool) => pool,
        Err(response) => return response,
    };
    if let Err(cause) = database
        .create_speaker(
            SpeakerInput {
                key: &body.key,
                name: &body.name,
                description: body.description.as_deref(),
            },
            state.config.speaker_recognition.max_speakers,
            id(&request),
        )
        .await
    {
        return write_error(&request, cause);
    }
    match get_speaker_by(database, &body.key).await {
        Ok(speaker) => with_etag(
            (
                StatusCode::CREATED,
                Json(speaker_resource_value(database, &speaker).await),
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
    let database = match database(&state) {
        Ok(pool) => pool,
        Err(response) => return response,
    };
    let speaker = match get_speaker_by(database, &key).await {
        Ok(speaker) => speaker,
        Err(sqlx::Error::RowNotFound) => {
            return error(&request, StatusCode::NOT_FOUND, "speaker_not_found");
        }
        Err(error_value) => return sql_error(&request, &error_value),
    };
    let voiceprints = match load_voiceprints(database, speaker.id).await {
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
    let database = match database(&state) {
        Ok(pool) => pool,
        Err(response) => return response,
    };
    let old = match get_speaker_by(database, &key).await {
        Ok(speaker) => speaker,
        Err(sqlx::Error::RowNotFound) => {
            return error(&request, StatusCode::NOT_FOUND, "speaker_not_found");
        }
        Err(error_value) => return sql_error(&request, &error_value),
    };
    if old.revision != expected {
        database
            .speaker_conflict(id(&request), old.id, expected, "update")
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
    if let Err(cause) = database
        .update_speaker(
            &old,
            expected,
            SpeakerChanges {
                name: &name,
                description: description.as_deref(),
                enabled,
            },
            id(&request),
        )
        .await
    {
        return write_error(&request, cause);
    }
    // The commit published a security invalidation: revoke every session that pinned this speaker.
    if let Some(security) = security(&state) {
        security.invalidate_speaker(old.id);
    }
    match get_speaker_by(database, &key).await {
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

/// `DELETE /speakers/{key}` — remove profile and owned voiceprints after unlinking Agents.
pub(super) async fn delete(
    State(state): State<AppState>,
    Path(key): Path<String>,
    request: Request,
) -> Response {
    let expected = match expected(request.headers()) {
        Ok(value) => value,
        Err(code) => return error(&request, StatusCode::BAD_REQUEST, code),
    };
    let database = match database(&state) {
        Ok(pool) => pool,
        Err(response) => return response,
    };
    let old = match get_speaker_by(database, &key).await {
        Ok(speaker) => speaker,
        Err(sqlx::Error::RowNotFound) => {
            return error(&request, StatusCode::NOT_FOUND, "speaker_not_found");
        }
        Err(error_value) => return sql_error(&request, &error_value),
    };
    if old.revision != expected {
        database
            .speaker_conflict(id(&request), old.id, expected, "delete")
            .await;
        return error(&request, StatusCode::CONFLICT, "revision_conflict");
    }
    if let Err(cause) = database.delete_speaker(&old, expected, id(&request)).await {
        return write_error(&request, cause);
    }
    if let Some(security) = security(&state) {
        security.invalidate_speaker(old.id);
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
    let database = match database(&state) {
        Ok(pool) => pool,
        Err(response) => return response,
    };
    let old = match get_speaker_by(database, &key).await {
        Ok(speaker) => speaker,
        Err(sqlx::Error::RowNotFound) => {
            return error(&request, StatusCode::NOT_FOUND, "speaker_not_found");
        }
        Err(error_value) => return sql_error(&request, &error_value),
    };
    if old.revision != expected {
        database
            .speaker_conflict(id(&request), old.id, expected, "purge_voiceprint")
            .await;
        return error(&request, StatusCode::CONFLICT, "revision_conflict");
    }
    if let Err(cause) = database
        .purge_speaker_voiceprints(&old, expected, id(&request))
        .await
    {
        return write_error(&request, cause);
    }
    if let Some(security) = security(&state) {
        security.invalidate_speaker(old.id);
    }
    match get_speaker_by(database, &key).await {
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

pub(super) async fn speaker_resource_by_key(
    database: &crate::database::Database,
    key: &str,
) -> Result<Value, sqlx::Error> {
    let speaker = get_speaker_by(database, key).await?;
    Ok(speaker_resource_value(database, &speaker).await)
}

async fn speaker_resource_value(
    database: &crate::database::Database,
    speaker: &SpeakerRow,
) -> Value {
    let voiceprints = load_voiceprints(database, speaker.id)
        .await
        .unwrap_or_default();
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
