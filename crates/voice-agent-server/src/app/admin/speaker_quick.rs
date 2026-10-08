//! The two-operation Admin boundary for quick Speaker enrollment.

use super::*;
use crate::audio::enrollment::{self, QualityProfile, Reject, WavReject};
use crate::services::provider_diagnostic::ProviderDiagnosticRequestError;
use axum::{
    body::{Body, to_bytes},
    extract::Path,
    http::header,
};
use serde::Deserialize;
use serde_json::json;
use uuid::Uuid;

const CAPTURE_TTL_SECONDS: i64 = 10 * 60;
const TOMBSTONE_SECONDS: i64 = 24 * 60 * 60;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CommitBody {
    capture_id: String,
    name: String,
    #[serde(default)]
    description: Option<String>,
}

#[derive(sqlx::FromRow)]
struct ProviderRow {
    id: i64,
    revision: i64,
    enabled: i64,
}

#[derive(sqlx::FromRow)]
struct CaptureRow {
    provider_id: Option<i64>,
    provider_revision: i64,
    runtime_id: String,
    embedding_space: String,
    dims: i64,
    vector: Option<Vec<u8>>,
    status: String,
    speaker_id: Option<i64>,
    expires_at: i64,
}

pub(super) async fn create_capture(
    State(state): State<AppState>,
    Path(provider_key): Path<String>,
    request: Request,
) -> Response {
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
    let revision = match expected(request.headers()) {
        Ok(value) => value,
        Err(code) => return error(&request, StatusCode::BAD_REQUEST, code),
    };
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
    let pool = match db(&state) {
        Ok(pool) => pool,
        Err(response) => return response,
    };
    let provider = match provider(pool, &provider_key).await {
        Ok(Some(provider)) if provider.enabled != 0 && provider.revision == revision => provider,
        Ok(Some(provider)) if provider.enabled == 0 => {
            return error(&request, StatusCode::CONFLICT, "provider_disabled");
        }
        Ok(Some(_)) => return error(&request, StatusCode::CONFLICT, "provider_revision_conflict"),
        Ok(None) => return error(&request, StatusCode::NOT_FOUND, "provider_not_found"),
        Err(cause) => return sql_error(&request, &cause),
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
    let result = match state
        .provider_diagnostics
        .extract_speaker_embedding(&provider_key, revision, analyzed.window)
        .await
    {
        Ok(value) => value,
        Err(cause) => return diagnostic_error(&request, cause),
    };
    if enrollment::validate_embedding(&result.embedding, result.dimension).is_err() {
        return error(
            &request,
            StatusCode::SERVICE_UNAVAILABLE,
            "speaker_invalid_embedding",
        );
    }
    let mut vector = result.embedding;
    if !enrollment::normalize(&mut vector) {
        return error(
            &request,
            StatusCode::SERVICE_UNAVAILABLE,
            "speaker_invalid_embedding",
        );
    }
    let created_at = now();
    let id = Uuid::new_v4().to_string();
    let mut tx = match pool.begin().await {
        Ok(tx) => tx,
        Err(cause) => return sql_error(&request, &cause),
    };
    let active: i64 = match sqlx::query_scalar("SELECT (SELECT COUNT(*) FROM speaker_enrollment_drafts WHERE status='collecting' AND expires_at>?) + (SELECT COUNT(*) FROM speaker_quick_captures WHERE status='accepted' AND expires_at>?)")
        .bind(created_at).bind(created_at).fetch_one(&mut *tx).await { Ok(value) => value, Err(cause) => return sql_error(&request, &cause) };
    if active >= config.max_open_enrollments as i64 {
        return error(&request, StatusCode::CONFLICT, "enrollment_quota_exceeded");
    }
    let written = sqlx::query("INSERT INTO speaker_quick_captures (id,provider_id,provider_revision,runtime_id,embedding_space,dims,vector,status,created_at,expires_at) VALUES (?,?,?,?,?,?,?,'accepted',?,?)")
        .bind(&id).bind(provider.id).bind(revision).bind(&state.runtime_id).bind(&result.embedding_space_id).bind(result.dimension as i64).bind(enrollment::encode_embedding(&vector)).bind(created_at).bind(created_at + CAPTURE_TTL_SECONDS).execute(&mut *tx).await;
    if written.is_err() || tx.commit().await.is_err() {
        return error(
            &request,
            StatusCode::SERVICE_UNAVAILABLE,
            "database_unavailable",
        );
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
    let pool = match db(&state) {
        Ok(pool) => pool,
        Err(response) => return response,
    };
    let mut tx = match pool.begin().await {
        Ok(tx) => tx,
        Err(cause) => return sql_error(&request, &cause),
    };
    let capture = match sqlx::query_as::<_, CaptureRow>("SELECT provider_id,provider_revision,runtime_id,embedding_space,dims,vector,status,speaker_id,expires_at FROM speaker_quick_captures WHERE id=?").bind(&body.capture_id).fetch_optional(&mut *tx).await {
        Ok(Some(value)) => value, Ok(None) => return error(&request, StatusCode::NOT_FOUND, "capture_not_found"), Err(cause) => return sql_error(&request, &cause),
    };
    if capture.status == "committed" {
        let speaker_id = capture.speaker_id.expect("committed capture has speaker");
        let speaker = match sqlx::query_as::<_, (String,)>("SELECT key FROM speakers WHERE id=?")
            .bind(speaker_id)
            .fetch_optional(&mut *tx)
            .await
        {
            Ok(Some(value)) => value.0,
            _ => return error(&request, StatusCode::GONE, "capture_consumed"),
        };
        if tx.commit().await.is_err() {
            return error(
                &request,
                StatusCode::SERVICE_UNAVAILABLE,
                "database_unavailable",
            );
        }
        return match super::speakers::speaker_resource_by_key(pool, &speaker).await {
            Ok(value) => (StatusCode::OK, Json(json!({"speaker": value}))).into_response(),
            Err(cause) => sql_error(&request, &cause),
        };
    }
    if capture.status != "accepted" || capture.expires_at <= now() {
        return error(&request, StatusCode::GONE, "capture_expired");
    }
    if capture.runtime_id != state.runtime_id {
        return error(
            &request,
            StatusCode::CONFLICT,
            "capture_runtime_incompatible",
        );
    }
    let provider_id = match capture.provider_id {
        Some(value) => value,
        None => return error(&request, StatusCode::CONFLICT, "provider_changed"),
    };
    let current: Option<(i64,)> = match sqlx::query_as(
        "SELECT revision FROM providers WHERE id=? AND type='speaker' AND enabled=1",
    )
    .bind(provider_id)
    .fetch_optional(&mut *tx)
    .await
    {
        Ok(value) => value,
        Err(cause) => return sql_error(&request, &cause),
    };
    if current.is_none_or(|value| value.0 != capture.provider_revision) {
        return error(&request, StatusCode::CONFLICT, "provider_revision_conflict");
    }
    let count: i64 = match sqlx::query_scalar("SELECT COUNT(*) FROM speakers")
        .fetch_one(&mut *tx)
        .await
    {
        Ok(value) => value,
        Err(cause) => return sql_error(&request, &cause),
    };
    if count >= state.config.speaker_recognition.max_speakers as i64 {
        return error(&request, StatusCode::CONFLICT, "speaker_quota_exceeded");
    }
    let key = format!("spk_{}", Uuid::new_v4().simple());
    let time = now();
    // This no-op update obtains SQLite's write reservation without violating the committed-row
    // constraint before the new Speaker ID exists.
    let reserved = sqlx::query(
        "UPDATE speaker_quick_captures SET expires_at=expires_at WHERE id=? AND status='accepted'",
    )
    .bind(&body.capture_id)
    .execute(&mut *tx)
    .await;
    if !matches!(reserved, Ok(ref result) if result.rows_affected() == 1) {
        return error(&request, StatusCode::CONFLICT, "capture_busy");
    }
    let speaker_id = match sqlx::query("INSERT INTO speakers (key,name,description,enabled,revision,created_at,updated_at) VALUES (?,?,?,1,1,?,?)").bind(&key).bind(&body.name).bind(body.description.as_deref()).bind(time).bind(time).execute(&mut *tx).await { Ok(result) => result.last_insert_rowid(), Err(cause) => return mutation_sql_error(&request, &cause) };
    let vector = capture.vector.expect("accepted capture has vector");
    let voiceprint = sqlx::query("INSERT INTO speaker_voiceprints (speaker_id,embedding_space,revision,sample_count,browser_validation_status,provider_id,provider_key,provider_revision,dims,vector,calibration_revision,enrolled_at,updated_at) SELECT ?,?,1,1,'pending',p.id,p.key,?,?,?, 'quick-v1',?,? FROM providers p WHERE p.id=?")
        .bind(speaker_id).bind(&capture.embedding_space).bind(capture.provider_revision).bind(capture.dims).bind(vector).bind(time).bind(time).bind(provider_id).execute(&mut *tx).await;
    let committed = sqlx::query("UPDATE speaker_quick_captures SET status='committed',vector=NULL,speaker_id=?,committed_at=?,expires_at=? WHERE id=? AND status='accepted'")
        .bind(speaker_id)
        .bind(time)
        .bind(time + TOMBSTONE_SECONDS)
        .bind(&body.capture_id)
        .execute(&mut *tx)
        .await;
    if voiceprint.is_err()
        || !matches!(committed, Ok(ref result) if result.rows_affected() == 1)
        || super::speakers::publish_catalog_revision(&mut tx)
            .await
            .is_err()
        || audit(
            &mut *tx,
            id(&request),
            "speaker",
            Some(speaker_id),
            "quick_enrollment",
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
    if let Some(security) = security(&state) {
        security.invalidate_speaker(speaker_id);
    }
    match super::speakers::speaker_resource_by_key(pool, &key).await {
        Ok(value) => (StatusCode::CREATED, Json(json!({"speaker": value}))).into_response(),
        Err(cause) => sql_error(&request, &cause),
    }
}

async fn provider(pool: &SqlitePool, key: &str) -> Result<Option<ProviderRow>, sqlx::Error> {
    sqlx::query_as("SELECT id,revision,enabled FROM providers WHERE key=? AND type='speaker'")
        .bind(key)
        .fetch_optional(pool)
        .await
}

pub(super) async fn cleanup_expired_captures(pool: &SqlitePool) {
    let time = now();
    let _ = sqlx::query("DELETE FROM speaker_quick_captures WHERE (status='accepted' AND expires_at<=?) OR (status='committed' AND expires_at<=?)")
        .bind(time)
        .bind(time)
        .execute(pool)
        .await;
}

fn diagnostic_error(request: &Request, cause: ProviderDiagnosticRequestError) -> Response {
    match cause {
        ProviderDiagnosticRequestError::NotFound => {
            error(request, StatusCode::NOT_FOUND, "provider_not_found")
        }
        ProviderDiagnosticRequestError::Disabled => {
            error(request, StatusCode::CONFLICT, "provider_disabled")
        }
        ProviderDiagnosticRequestError::RevisionConflict => {
            error(request, StatusCode::CONFLICT, "provider_revision_conflict")
        }
        ProviderDiagnosticRequestError::Runtime(
            crate::services::provider_runtime::RuntimeError::Busy,
        ) => error(
            request,
            StatusCode::TOO_MANY_REQUESTS,
            "provider_runtime_busy",
        ),
        ProviderDiagnosticRequestError::Runtime(
            crate::services::provider_runtime::RuntimeError::Timeout,
        ) => error(
            request,
            StatusCode::GATEWAY_TIMEOUT,
            "speaker_inference_timeout",
        ),
        _ => error(
            request,
            StatusCode::SERVICE_UNAVAILABLE,
            "speaker_runtime_unavailable",
        ),
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
