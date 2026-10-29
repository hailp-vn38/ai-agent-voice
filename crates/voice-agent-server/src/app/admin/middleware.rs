//! Admin HTTP middleware: request ID correlation, transport body limits, and Bearer token auth.

use super::{
    AppState, MAX_ASR_TEST_BODY, MAX_BODY, MAX_JOB_CREATE_BODY, MAX_QUICK_CAPTURE_BODY,
    MAX_SPEAKER_SAMPLE_BODY, error,
};
use axum::{
    extract::{Request, State},
    http::{HeaderValue, StatusCode, header},
    middleware::Next,
    response::Response,
};
use uuid::Uuid;

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
    let valid = request
        .headers()
        .get_all(header::AUTHORIZATION)
        .iter()
        .count()
        == 1
        && request
            .headers()
            .get(header::AUTHORIZATION)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.strip_prefix("Bearer "))
            .is_some_and(|token| {
                !token.is_empty()
                    && constant_time_eq(token.as_bytes(), state.config.api.admin_token.as_bytes())
            });
    if valid {
        next.run(request).await
    } else {
        error(&request, StatusCode::UNAUTHORIZED, "unauthorized")
    }
}

fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    let mut diff = left.len() ^ right.len();
    for index in 0..left.len().max(right.len()) {
        diff |= usize::from(*left.get(index).unwrap_or(&0) ^ *right.get(index).unwrap_or(&0));
    }
    diff == 0
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
