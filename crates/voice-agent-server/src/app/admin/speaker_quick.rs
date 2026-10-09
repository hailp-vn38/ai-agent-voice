//! The two-operation Admin boundary for built-in Speaker enrollment.

use super::*;
use crate::audio::enrollment::{self, QualityProfile, Reject, WavReject};
use axum::{
    body::{Body, to_bytes},
    extract::Path,
    http::header,
};
use serde::Deserialize;
use serde_json::json;
use uuid::Uuid;

const CAPTURE_TTL_SECONDS: i64 = 10 * 60;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CommitBody {
    capture_id: String,
    name: String,
    #[serde(default)]
    description: Option<String>,
}

pub(super) async fn create_capture(State(state): State<AppState>, request: Request) -> Response {
    if request
        .headers()
        .get(header::CONTENT_TYPE)
        .map(|value| value.as_bytes())
        != Some(b"audio/wav")
    {
        return error(
            &request,
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            "unsupported_audio_format",
        );
    }
    let (parts, body) = request.into_parts();
    let request = Request::from_parts(parts, Body::empty());
    let body = match to_bytes(body, MAX_QUICK_CAPTURE_BODY).await {
        Ok(body) => body,
        Err(_) => return error(&request, StatusCode::PAYLOAD_TOO_LARGE, "request_too_large"),
    };
    if body.len()
        > state
            .config
            .speaker_recognition
            .enrollment
            .max_audio_body_bytes
    {
        return error(&request, StatusCode::PAYLOAD_TOO_LARGE, "request_too_large");
    }
    let database = match database(&state) {
        Ok(pool) => pool,
        Err(response) => return response,
    };
    let (pcm, duration_ms) = match enrollment::parse_wav(&body) {
        Ok(value) => value,
        Err(WavReject::UnsupportedFormat) => {
            return error(
                &request,
                StatusCode::UNSUPPORTED_MEDIA_TYPE,
                "unsupported_audio_format",
            );
        }
        Err(WavReject::TooLong) => {
            return error(
                &request,
                StatusCode::UNPROCESSABLE_ENTITY,
                "speaker_audio_too_long",
            );
        }
        Err(WavReject::Malformed) => {
            return error(&request, StatusCode::BAD_REQUEST, "invalid_audio");
        }
    };
    let config = &state.config.speaker_recognition.enrollment;
    let analyzed = match enrollment::analyze(
        &pcm,
        duration_ms,
        &QualityProfile {
            min_clip_ms: config.min_clip_ms,
            max_clip_ms: config.max_clip_ms,
            min_speech_ms: config.min_speech_ms,
            max_window_ms: config.max_window_ms,
        },
    ) {
        Ok(value) => value,
        Err(Reject::TooShort) => {
            return error(
                &request,
                StatusCode::UNPROCESSABLE_ENTITY,
                "speaker_audio_too_short",
            );
        }
        Err(Reject::TooLong) => {
            return error(
                &request,
                StatusCode::UNPROCESSABLE_ENTITY,
                "speaker_audio_too_long",
            );
        }
        Err(Reject::InsufficientAudio) => {
            return error(
                &request,
                StatusCode::UNPROCESSABLE_ENTITY,
                "speaker_insufficient_audio",
            );
        }
        Err(Reject::Clipped) => {
            return error(
                &request,
                StatusCode::UNPROCESSABLE_ENTITY,
                "speaker_audio_clipped",
            );
        }
    };
    let Some(runtime) = state.speaker_runtime.as_ref() else {
        return error(
            &request,
            StatusCode::SERVICE_UNAVAILABLE,
            "speaker_unavailable",
        );
    };
    let mut vector = match runtime.extract_builtin(analyzed.window).await {
        Ok(vector) => vector,
        Err(crate::providers::speaker::SpeakerError::Busy) => {
            return error(
                &request,
                StatusCode::TOO_MANY_REQUESTS,
                "speaker_runtime_busy",
            );
        }
        Err(_) => {
            return error(
                &request,
                StatusCode::SERVICE_UNAVAILABLE,
                "speaker_unavailable",
            );
        }
    };
    let dimension = runtime.dimension();
    let embedding_space_id = runtime.embedding_space_id().to_owned();
    if enrollment::validate_embedding(&vector, dimension).is_err() {
        return error(
            &request,
            StatusCode::SERVICE_UNAVAILABLE,
            "speaker_invalid_embedding",
        );
    }
    if !enrollment::normalize(&mut vector) {
        return error(
            &request,
            StatusCode::SERVICE_UNAVAILABLE,
            "speaker_invalid_embedding",
        );
    }
    let created_at = now();
    let id = Uuid::new_v4().to_string();
    if let Err(cause) = database
        .store_speaker_capture(crate::database::speakers::captures::NewCapture {
            id: &id,
            runtime_id: &state.runtime_id,
            embedding_space: &embedding_space_id,
            dimension,
            vector: enrollment::encode_embedding(&vector),
            created_at,
            max_open: config.max_open_enrollments,
        })
        .await
    {
        return write_error(&request, cause);
    }
    (StatusCode::CREATED, Json(json!({"status":"accepted","capture_id":id,"quality":{"duration_ms":analyzed.quality.duration_ms,"speech_ms":analyzed.quality.speech_ms},"expires_at":created_at + CAPTURE_TTL_SECONDS}))).into_response()
}

pub(super) async fn create_speaker_from_capture(
    State(state): State<AppState>,
    request: Request,
) -> Response {
    let (request, body): (_, CommitBody) = match json(request).await {
        Ok(value) => value,
        Err(response) => return response,
    };
    if body.capture_id.len() > 64
        || !valid_text(&body.name, 128, false)
        || body
            .description
            .as_deref()
            .is_some_and(|value| !valid_text(value, 2048, true))
    {
        return error(&request, StatusCode::BAD_REQUEST, "validation_failed");
    }
    let database = match database(&state) {
        Ok(pool) => pool,
        Err(response) => return response,
    };
    let committed = match database
        .promote_speaker_capture(
            crate::database::speakers::captures::CapturePromotion {
                capture_id: &body.capture_id,
                name: &body.name,
                description: body.description.as_deref(),
                runtime_id: &state.runtime_id,
                embedding: state
                    .speaker_runtime
                    .as_ref()
                    .map(|runtime| (runtime.embedding_space_id(), runtime.dimension())),
                max_speakers: state.config.speaker_recognition.max_speakers,
            },
            id(&request),
        )
        .await
    {
        Ok(value) => value,
        Err(cause) => return write_error(&request, cause),
    };
    if !committed.created {
        return match super::speakers::speaker_resource_by_key(database, &committed.key).await {
            Ok(value) => (StatusCode::OK, Json(json!({"speaker": value}))).into_response(),
            Err(cause) => sql_error(&request, &cause),
        };
    }
    let speaker_id = committed.speaker_id;
    let key = committed.key;
    if let Some(security) = security(&state) {
        security.invalidate_speaker(speaker_id);
    }
    match super::speakers::speaker_resource_by_key(database, &key).await {
        Ok(value) => (StatusCode::CREATED, Json(json!({"speaker": value}))).into_response(),
        Err(cause) => sql_error(&request, &cause),
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ReplaceVoiceprintBody {
    capture_id: String,
}

/// PUT /speakers/{key}/voiceprint — one captured embedding replaces the current
/// model-space vector without changing the Speaker identity or Agent links.
pub(super) async fn replace_voiceprint(
    State(state): State<AppState>,
    Path(key): Path<String>,
    request: Request,
) -> Response {
    let expected_revision = match expected(request.headers()) {
        Ok(value) => value,
        Err(code) => return error(&request, StatusCode::BAD_REQUEST, code),
    };
    let (request, body): (_, ReplaceVoiceprintBody) = match json(request).await {
        Ok(value) => value,
        Err(response) => return response,
    };
    if body.capture_id.is_empty() || body.capture_id.len() > 64 {
        return error(&request, StatusCode::BAD_REQUEST, "validation_failed");
    }
    let database = match database(&state) {
        Ok(pool) => pool,
        Err(response) => return response,
    };
    let Some(runtime) = state.speaker_runtime.as_ref() else {
        return error(
            &request,
            StatusCode::SERVICE_UNAVAILABLE,
            "speaker_unavailable",
        );
    };
    if let Err(cause) = database
        .replace_speaker_voiceprint(
            &key,
            &body.capture_id,
            expected_revision,
            (
                &state.runtime_id,
                runtime.embedding_space_id(),
                runtime.dimension(),
            ),
            id(&request),
        )
        .await
    {
        return write_error(&request, cause);
    }
    match super::speakers::speaker_resource_by_key(database, &key).await {
        Ok(value) => (StatusCode::OK, Json(serde_json::json!({"speaker": value}))).into_response(),
        Err(cause) => sql_error(&request, &cause),
    }
}

#[cfg(test)]
mod tests {
    use sqlx::SqlitePool;

    #[tokio::test]
    async fn reservation_keeps_the_capture_accepted_until_a_speaker_is_linked() {
        let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
        sqlx::query(
            "CREATE TABLE speaker_quick_captures (
                id TEXT PRIMARY KEY, status TEXT NOT NULL, vector BLOB, speaker_id INTEGER,
                expires_at INTEGER NOT NULL, committed_at INTEGER,
                CHECK ((status='accepted' AND vector IS NOT NULL AND speaker_id IS NULL AND committed_at IS NULL)
                    OR (status='committed' AND vector IS NULL AND speaker_id IS NOT NULL AND committed_at IS NOT NULL))
            )",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query("INSERT INTO speaker_quick_captures (id,status,vector,expires_at) VALUES ('capture','accepted',X'01',1)")
            .execute(&pool)
            .await
            .unwrap();

        let reserved = sqlx::query("UPDATE speaker_quick_captures SET expires_at=expires_at WHERE id='capture' AND status='accepted'")
            .execute(&pool)
            .await;
        assert!(
            reserved.is_ok(),
            "reservation must retain a valid accepted row"
        );
        let committed = sqlx::query("UPDATE speaker_quick_captures SET status='committed',vector=NULL,speaker_id=1,committed_at=2 WHERE id='capture' AND status='accepted'")
            .execute(&pool)
            .await
            .unwrap();
        assert_eq!(committed.rows_affected(), 1);
    }

    #[tokio::test]
    async fn quick_voiceprint_insert_matches_its_columns() {
        let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
        for ddl in [
            "CREATE TABLE providers (id INTEGER PRIMARY KEY, key TEXT, revision INTEGER, type TEXT, enabled INTEGER)",
            "CREATE TABLE speakers (id INTEGER PRIMARY KEY, key TEXT, name TEXT, description TEXT, enabled INTEGER, revision INTEGER, created_at INTEGER, updated_at INTEGER)",
            "CREATE TABLE speaker_voiceprints (speaker_id INTEGER, embedding_space TEXT, revision INTEGER, sample_count INTEGER, browser_validation_status TEXT, provider_id INTEGER, provider_key TEXT, provider_revision INTEGER, dims INTEGER, vector BLOB, calibration_revision TEXT, enrolled_at INTEGER, updated_at INTEGER)",
        ] {
            sqlx::query(ddl).execute(&pool).await.unwrap();
        }
        sqlx::query("INSERT INTO providers VALUES (1, 'speaker', 1, 'speaker', 1)")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO speakers VALUES (1, 'spk_test', 'Owner', NULL, 1, 1, 1, 1)")
            .execute(&pool)
            .await
            .unwrap();
        let inserted = sqlx::query("INSERT INTO speaker_voiceprints (speaker_id,embedding_space,revision,sample_count,browser_validation_status,provider_id,provider_key,provider_revision,dims,vector,calibration_revision,enrolled_at,updated_at) SELECT ?,?,1,1,'pending',p.id,p.key,?,?,?, 'quick-v1',?,? FROM providers p WHERE p.id=?")
            .bind(1).bind("space").bind(1).bind(3).bind(vec![1_u8; 12]).bind(1).bind(1).bind(1)
            .execute(&pool).await.unwrap();
        assert_eq!(inserted.rows_affected(), 1);
    }
}
