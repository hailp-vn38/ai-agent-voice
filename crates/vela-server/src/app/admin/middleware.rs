//! Admin HTTP middleware: request ID correlation, transport body limits, and Bearer token auth.

use super::{
    AppState, MAX_ASR_TEST_BODY, MAX_BODY, MAX_JOB_CREATE_BODY, MAX_QUICK_CAPTURE_BODY,
    MAX_SPEAKER_SAMPLE_BODY, error,
};
use axum::{
    body::Body,
    extract::{Request, State},
    http::{HeaderValue, StatusCode, header},
    middleware::Next,
    response::Response,
};
use futures_util::StreamExt;
use std::time::{Duration, SystemTime};
use uuid::Uuid;

const ADMIN_STREAM_AUTH_CHECK_INTERVAL: Duration = Duration::from_secs(5);

pub(super) async fn transport(request: Request, next: Next) -> Response {
    if request
        .headers()
        .get(header::CONTENT_ENCODING)
        .is_some_and(|value| value != "identity")
    {
        return error(
            &request,
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            "unsupported_content_encoding",
        );
    }
    let is_mutation = matches!(
        request.method(),
        &http::Method::POST | &http::Method::PATCH | &http::Method::PUT
    );
    if is_mutation {
        let max_body = if request.uri().path().ends_with("/test/asr")
            || request.uri().path().ends_with("/provider-tests/asr")
        {
            MAX_ASR_TEST_BODY + 80 * 1024
        } else if request.uri().path().ends_with("/knowledge/raw/batch") {
            super::knowledge::MAX_RAW_BATCH_BODY
        } else if request.uri().path().ends_with("/system/backup/import")
            || request.uri().path().ends_with("/system/backup/inspect")
        {
            crate::database::backup::MAX_BACKUP_BODY_BYTES + 80 * 1024
        } else if request.uri().path().ends_with("/speakers/captures") {
            MAX_QUICK_CAPTURE_BODY
        } else if request.uri().path().contains("/enrollments/")
            && request.uri().path().contains("/samples/")
        {
            MAX_SPEAKER_SAMPLE_BODY
        } else if request.method() == http::Method::POST && request.uri().path().ends_with("/jobs")
        {
            MAX_JOB_CREATE_BODY
        } else {
            MAX_BODY
        };
        if let Some(length) = request
            .headers()
            .get(header::CONTENT_LENGTH)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.parse::<usize>().ok())
            && length > max_body
        {
            return error(&request, StatusCode::PAYLOAD_TOO_LARGE, "request_too_large");
        }
    }
    next.run(request).await
}

pub(super) async fn request_id(mut request: Request, next: Next) -> Response {
    let id = Uuid::new_v4().to_string();
    tracing::Span::current().record("request_id", id.as_str());
    request.extensions_mut().insert(RequestId(id.clone()));
    let mut response = next.run(request).await;
    response.headers_mut().insert(
        "x-request-id",
        HeaderValue::from_str(&id).expect("uuid header"),
    );
    response
}

pub(super) async fn authenticate(
    State(state): State<AppState>,
    request: Request,
    next: Next,
) -> Response {
    if request.method() == http::Method::GET
        && request
            .uri()
            .path()
            .ends_with("/llm-gateway/oauth/callback")
    {
        return next.run(request).await;
    }
    let header_count = request
        .headers()
        .get_all(header::AUTHORIZATION)
        .iter()
        .count();
    if header_count > 1 {
        return error(&request, StatusCode::UNAUTHORIZED, "unauthorized");
    }
    let token = if header_count == 1 {
        request
            .headers()
            .get(header::AUTHORIZATION)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.strip_prefix("Bearer "))
            .filter(|token| !token.is_empty())
    } else {
        None
    };
    let password_change = super::auth::is_password_change_path(request.uri().path())
        && request.method() == http::Method::POST;
    if let Some(token) = token
        && state.admin_auth.accepts(token)
    {
        if state.admin_auth.status().must_change_password && !password_change {
            return error(&request, StatusCode::FORBIDDEN, "password_change_required");
        }
        return next.run(request).await;
    }
    // Password Login switched off: the Admin API admits requests without a credential, as the
    // Admin has accepted that anyone who can reach the server has full access.
    if !state.admin_auth.status().password_login_enabled {
        return next.run(request).await;
    }
    error(&request, StatusCode::UNAUTHORIZED, "unauthorized")
}

/// Wraps an Admin SSE body so revoking the request's authorization also ends its open response.
pub(super) async fn guard_admin_sse(
    State(state): State<AppState>,
    request: Request,
    next: Next,
) -> Response {
    let bearer = request
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .filter(|value| !value.is_empty())
        .map(str::to_owned);
    let expires_at = bearer
        .as_deref()
        .and_then(|value| state.admin_auth.token_expiry(value))
        .and_then(|expires_at| {
            expires_at
                .duration_since(SystemTime::now())
                .ok()
                .map(|remaining| tokio::time::Instant::now() + remaining)
        });

    let response = next.run(request).await;
    let is_sse = response
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.starts_with("text/event-stream"));
    if !is_sse {
        return response;
    }

    let (parts, body) = response.into_parts();
    let chunks = body.into_data_stream();
    let guarded = futures_util::stream::unfold(
        (chunks, state, bearer, expires_at),
        |(mut chunks, state, bearer, expires_at)| async move {
            tokio::select! {
                biased;
                _ = wait_for_admin_stream_revocation(
                    state.clone(),
                    bearer.clone(),
                    expires_at,
                    ADMIN_STREAM_AUTH_CHECK_INTERVAL,
                ) => None,
                chunk = chunks.next() => chunk.map(|chunk| {
                    (chunk, (chunks, state, bearer, expires_at))
                }),
            }
        },
    );
    Response::from_parts(parts, Body::from_stream(guarded))
}

pub(super) async fn wait_for_admin_stream_revocation(
    state: AppState,
    bearer: Option<String>,
    expires_at: Option<tokio::time::Instant>,
    check_interval: Duration,
) {
    loop {
        if !state.admin_auth.stream_access_allowed(bearer.as_deref()) {
            return;
        }
        if let Some(expires_at) = expires_at {
            tokio::select! {
                biased;
                _ = tokio::time::sleep_until(expires_at) => return,
                _ = tokio::time::sleep(check_interval) => {}
            }
        } else {
            tokio::time::sleep(check_interval).await;
        }
    }
}

#[derive(Clone)]
struct RequestId(String);

pub(super) fn id(request: &Request) -> &str {
    request
        .extensions()
        .get::<RequestId>()
        .map(|v| v.0.as_str())
        .unwrap_or("unknown")
}
