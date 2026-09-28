use super::AppState;
use crate::providers::{VisionError, VisionRequest};
use axum::{
    Json,
    extract::{Multipart, State},
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Response},
};
use std::{sync::Arc, time::Instant};

#[derive(serde::Serialize)]
struct Success {
    success: bool,
    action: &'static str,
    response: String,
}
#[derive(serde::Serialize)]
struct Failure {
    success: bool,
    message: &'static str,
}
fn failure(status: StatusCode, message: &'static str) -> Response {
    (
        status,
        Json(Failure {
            success: false,
            message,
        }),
    )
        .into_response()
}

pub async fn get_handler(State(state): State<AppState>) -> Response {
    let Some(url) = state.config.vision.public_url.as_ref() else {
        return "MCP Vision interface is ready: /mcp/vision/explain".into_response();
    };
    format!("MCP Vision interface is ready: {url}").into_response()
}

/// Browser and firmware preflight compatibility; this must never invoke Vision inference.
pub async fn options_handler() -> Response {
    (
        StatusCode::NO_CONTENT,
        [
            ("access-control-allow-origin", "*"),
            ("access-control-allow-methods", "GET, POST, OPTIONS"),
            (
                "access-control-allow-headers",
                "Authorization, Device-Id, Client-Id, Content-Type",
            ),
        ],
    )
        .into_response()
}

/// Apply the same CORS policy to successful and error responses. Browsers validate the actual
/// multipart POST response as well as its OPTIONS preflight.
pub async fn cors_response(mut response: Response) -> Response {
    let headers = response.headers_mut();
    headers.insert(
        "access-control-allow-origin",
        "*".parse().expect("valid header"),
    );
    headers.insert(
        "access-control-allow-methods",
        "GET, POST, OPTIONS".parse().expect("valid header"),
    );
    headers.insert(
        "access-control-allow-headers",
        "Authorization, Device-Id, Client-Id, Content-Type"
            .parse()
            .expect("valid header"),
    );
    response
}
pub async fn post_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    mut multipart: Multipart,
) -> Response {
    if !authorized(&state, &headers) {
        return failure(StatusCode::UNAUTHORIZED, "unauthorized");
    }
    let (Some(device_id), Some(client_id)) = (
        required_header(&headers, "device-id"),
        required_header(&headers, "client-id"),
    ) else {
        return failure(
            StatusCode::BAD_REQUEST,
            "device-id and client-id are required",
        );
    };
    let mut question = None;
    let mut image = None;
    loop {
        let field = match multipart.next_field().await {
            Ok(Some(field)) => field,
            Ok(None) => break,
            Err(_) => return failure(StatusCode::BAD_REQUEST, "invalid multipart body"),
        };
        match field.name() {
            Some("question") => {
                if question.is_none() {
                    question = match field.text().await {
                        Ok(question) => Some(question),
                        Err(_) => {
                            return failure(StatusCode::BAD_REQUEST, "invalid question field");
                        }
                    };
                }
            }
            Some("image") if image.is_none() => {
                image = match field.bytes().await {
                    Ok(bytes) => Some(bytes.to_vec()),
                    Err(_) => return failure(StatusCode::BAD_REQUEST, "invalid image field"),
                };
            }
            _ => {}
        }
    }
    let Some(question) = question else {
        return failure(StatusCode::BAD_REQUEST, "question is required");
    };
    if question.trim().is_empty() || question.len() > state.config.vision.max_question_bytes {
        return failure(StatusCode::BAD_REQUEST, "question is invalid");
    }
    let Some(image) = image else {
        return failure(StatusCode::BAD_REQUEST, "image is required");
    };
    if image.is_empty() {
        return failure(StatusCode::BAD_REQUEST, "image is empty");
    }
    if image.len() > state.config.vision.max_image_bytes {
        return failure(StatusCode::PAYLOAD_TOO_LARGE, "image exceeds limit");
    }
    let Some(mime_type) = detect_image_mime(&image) else {
        return failure(
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            "unsupported image format",
        );
    };
    let image_bytes = image.len();
    tracing::info!(
        device_id,
        client_id,
        image_bytes,
        mime_type,
        question_bytes = question.len(),
        "vision request accepted"
    );
    let Some(runtime) = state.bound_vision_runtime() else {
        return failure(StatusCode::SERVICE_UNAVAILABLE, "vision is not configured");
    };
    let started = Instant::now();
    let result = runtime
        .analyze(VisionRequest {
            question: Arc::<str>::from(question),
            image: Arc::<[u8]>::from(image),
            mime_type: Arc::<str>::from(mime_type),
        })
        .await;
    match result {
        Ok(response) => {
            tracing::info!(vision_provider_instance = ?state.config.effective_agent.providers.vision, vision_adapter = runtime.provider().adapter(), device_id, client_id, image_bytes, mime_type, elapsed_ms = started.elapsed().as_millis(), "vision request completed");
            Json(Success {
                success: true,
                action: "RESPONSE",
                response: response.text,
            })
            .into_response()
        }
        Err(VisionError::Timeout) => {
            failure(StatusCode::GATEWAY_TIMEOUT, "vision provider timed out")
        }
        Err(VisionError::Overloaded) => failure(
            StatusCode::SERVICE_UNAVAILABLE,
            "vision provider is overloaded",
        ),
        Err(_) => failure(StatusCode::BAD_GATEWAY, "vision provider failed"),
    }
}
fn authorized(state: &AppState, headers: &HeaderMap) -> bool {
    state.config.auth.token.is_empty()
        || headers
            .get(header::AUTHORIZATION)
            .and_then(|value| value.to_str().ok())
            == Some(&format!("Bearer {}", state.config.auth.token))
}
fn required_header<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .filter(|value| !value.is_empty())
}
pub fn detect_image_mime(bytes: &[u8]) -> Option<&'static str> {
    match bytes {
        [0xff, 0xd8, 0xff, ..] => Some("image/jpeg"),
        [0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, ..] => Some("image/png"),
        [b'G', b'I', b'F', b'8', b'7' | b'9', b'a', ..] => Some("image/gif"),
        [b'B', b'M', ..] => Some("image/bmp"),
        [b'I', b'I', 0x2a, 0, ..] | [b'M', b'M', 0, 0x2a, ..] => Some("image/tiff"),
        [
            b'R',
            b'I',
            b'F',
            b'F',
            _,
            _,
            _,
            _,
            b'W',
            b'E',
            b'B',
            b'P',
            ..,
        ] => Some("image/webp"),
        _ => None,
    }
}
