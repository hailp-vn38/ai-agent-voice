//! Shared Admin HTTP helpers, JSON extraction, validation, pagination, and error mapping.

use super::{AppState, id};
use axum::{
    Json,
    body::{Body, to_bytes},
    extract::Request,
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Response},
};
use serde::{Deserialize, de::DeserializeOwned};
use serde_json::Value;
use std::time::{SystemTime, UNIX_EPOCH};

pub(super) const MAX_BODY: usize = 256 * 1024;
pub(super) const MAX_ASR_TEST_BODY: usize = 5 * 1024 * 1024;
/// Voice samples are raw PCM16 WAV: 12s mono 16 kHz is 384 KiB, so the 256 KiB
/// JSON cap would reject every valid clip. The route itself still enforces
/// `speaker.max_audio_body_bytes` (512 KiB) and the 12s duration cap.
pub(super) const MAX_SPEAKER_SAMPLE_BODY: usize = 512 * 1024;
pub(super) const MAX_QUICK_CAPTURE_BODY: usize = 4 * 1024 * 1024;
pub(super) const MAX_JOB_CREATE_BODY: usize = 64 * 1024 * 1024;
pub(super) const PAGE_DEFAULT: u32 = 50;
pub(super) const PAGE_MAX: u32 = 200;

pub(super) fn error(request: &Request, status: StatusCode, code: &'static str) -> Response {
    (
        status,
        Json(serde_json::json!({"error":{"code":code,"request_id":id(request)}})),
    )
        .into_response()
}

/// The shared security registry, used by every mutating handler to revoke the live Voice Sessions
/// that pinned the changed dependency.
pub(super) fn security(
    state: &AppState,
) -> Option<std::sync::Arc<crate::database::tool_security::ToolSecurity>> {
    state
        .database
        .as_ref()
        .map(|db| std::sync::Arc::clone(&db.tool_security))
}

pub(super) fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

fn sql_error_kind(error: &sqlx::Error) -> &'static str {
    match error {
        sqlx::Error::PoolTimedOut => "database_unavailable",
        sqlx::Error::Database(error)
            if matches!(
                error.code().as_deref(),
                Some("5" | "6" | "SQLITE_BUSY" | "SQLITE_LOCKED")
            ) =>
        {
            "database_busy"
        }
        _ => "database_unavailable",
    }
}

pub(super) fn sql_error(request: &Request, sql_error_value: &sqlx::Error) -> Response {
    error(
        request,
        StatusCode::SERVICE_UNAVAILABLE,
        sql_error_kind(sql_error_value),
    )
}

pub(super) fn mutation_sql_error(request: &Request, sql_error_value: &sqlx::Error) -> Response {
    if sql_error_kind(sql_error_value) == "database_busy" {
        sql_error(request, sql_error_value)
    } else {
        error(request, StatusCode::CONFLICT, "resource_conflict")
    }
}

#[allow(clippy::result_large_err)] // Preserves the request so callers can attach its request ID.
pub(super) async fn json<T: DeserializeOwned>(request: Request) -> Result<(Request, T), Response> {
    json_with_limit(request, MAX_BODY).await
}

#[allow(clippy::result_large_err)]
pub(super) async fn json_with_limit<T: DeserializeOwned>(
    request: Request,
    max_body: usize,
) -> Result<(Request, T), Response> {
    if request
        .headers()
        .get(header::CONTENT_ENCODING)
        .is_some_and(|v| v != "identity")
    {
        return Err(error(
            &request,
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            "unsupported_content_encoding",
        ));
    }
    let content_type = request
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    let mut content_parts = content_type.split(';');
    let media_type_ok = content_parts
        .next()
        .is_some_and(|v| v.trim().eq_ignore_ascii_case("application/json"));
    let parameters_ok = content_parts.all(|parameter| {
        let parameter = parameter.trim();
        !parameter.is_empty()
            && parameter
                .split_once('=')
                .is_some_and(|(name, value)| !name.trim().is_empty() && !value.trim().is_empty())
    });
    if !media_type_ok || !parameters_ok {
        return Err(error(
            &request,
            StatusCode::BAD_REQUEST,
            "invalid_content_type",
        ));
    }
    let (parts, body) = request.into_parts();
    let bytes = match to_bytes(body, max_body).await {
        Ok(bytes) => bytes,
        Err(_) => {
            let request = Request::from_parts(parts, Body::empty());
            return Err(error(
                &request,
                StatusCode::PAYLOAD_TOO_LARGE,
                "request_too_large",
            ));
        }
    };
    let request = Request::from_parts(parts, Body::empty());
    match serde_json::from_slice(&bytes) {
        Ok(payload) => Ok((request, payload)),
        Err(_) => Err(error(&request, StatusCode::BAD_REQUEST, "invalid_json")),
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(untagged)]
pub enum Patch<T> {
    Clear(Option<T>),
    #[default]
    Absent,
}

impl<T> Patch<T> {
    pub fn value(self) -> Option<Option<T>> {
        match self {
            Self::Absent => None,
            Self::Clear(v) => Some(v),
        }
    }
}

#[derive(Deserialize)]
pub(super) struct PageQuery {
    #[serde(default)]
    pub(super) page: Option<u32>,
    #[serde(default)]
    pub(super) enabled: Option<bool>,
    #[serde(default)]
    pub(super) sort: Option<String>,
    #[serde(default)]
    pub(super) page_size: Option<u32>,
}

/// The shared page window. It takes the two bounds rather than a resource query so every list
/// surface can declare only the filters it actually has.
pub(super) fn page_bounds(
    page: Option<u32>,
    page_size: Option<u32>,
) -> Result<(u32, u32), &'static str> {
    let page = page.unwrap_or(1);
    let page_size = page_size.unwrap_or(PAGE_DEFAULT);
    if page == 0 || page > 10_000 || !(1..=PAGE_MAX).contains(&page_size) {
        Err("invalid_query")
    } else {
        Ok((page, page_size))
    }
}

pub(super) fn valid_key(value: &str) -> bool {
    value.len() <= 64
        && value.as_bytes().first().is_some_and(u8::is_ascii_lowercase)
        && value
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
}

pub(super) fn valid_text(value: &str, max: usize, allow_empty: bool) -> bool {
    value.len() <= max && (allow_empty || !value.trim().is_empty())
}

pub(super) fn valid_identity(value: &str) -> bool {
    !value.is_empty() && value.len() <= 128 && value.bytes().all(|b| b > 0x1f && b != 0x7f)
}

pub(super) fn valid_metadata(value: &Value) -> bool {
    serde_json::to_vec(value).is_ok_and(|v| v.len() <= 16 * 1024) && json_shape(value, 0, &mut 0)
}

fn json_shape(value: &Value, depth: u8, nodes: &mut u16) -> bool {
    if depth > 8 {
        return false;
    }
    match value {
        Value::Array(items) => items.iter().all(|item| {
            *nodes += 1;
            *nodes <= 256 && json_shape(item, depth + 1, nodes)
        }),
        Value::Object(items) => items.values().all(|item| {
            *nodes += 1;
            *nodes <= 256 && json_shape(item, depth + 1, nodes)
        }),
        _ => true,
    }
}

pub(super) fn expected(headers: &HeaderMap) -> Result<i64, &'static str> {
    let value = headers
        .get(header::IF_MATCH)
        .and_then(|v| v.to_str().ok())
        .ok_or("invalid_if_match")?;
    value
        .strip_prefix('"')
        .and_then(|v| v.strip_suffix('"'))
        .and_then(|v| v.parse().ok())
        .filter(|v: &i64| *v > 0)
        .ok_or("invalid_if_match")
}

#[allow(clippy::result_large_err)]
pub(super) fn database(state: &AppState) -> Result<&crate::database::Database, Response> {
    state.database.as_deref().ok_or_else(|| {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({"error":{"code":"database_unavailable"}})),
        )
            .into_response()
    })
}

pub(super) fn write_error(
    request: &Request,
    cause: crate::database::writes::WriteError,
) -> Response {
    use crate::database::writes::WriteError;
    match cause {
        WriteError::Sql(cause) => sql_error(request, &cause),
        WriteError::Mutation(cause) => mutation_sql_error(request, &cause),
        WriteError::Unavailable => error(
            request,
            StatusCode::SERVICE_UNAVAILABLE,
            "database_unavailable",
        ),
        WriteError::NotFound => error(request, StatusCode::NOT_FOUND, "not_found"),
        WriteError::Invalid(code) => error(request, StatusCode::BAD_REQUEST, code),
        WriteError::Gone(code) => error(request, StatusCode::GONE, code),
        WriteError::Missing(code) => error(request, StatusCode::NOT_FOUND, code),
        WriteError::UnavailableCode(code) => error(request, StatusCode::SERVICE_UNAVAILABLE, code),
        WriteError::Conflict(code) => error(request, StatusCode::CONFLICT, code),
    }
}
