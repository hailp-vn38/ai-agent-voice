use super::AppState;
use crate::{
    database::{DeviceRegistration, EnrollmentClaim, EnrollmentCreateError, EnrollmentRequest},
    protocol::{Firmware, OtaActivation, OtaResponse, OtaWebsocket, ServerTime},
};
use axum::{
    body::to_bytes,
    extract::{Request, State},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
};
use serde_json::{Value, json};
use std::time::{SystemTime, UNIX_EPOCH};
use url::Url;
use uuid::Uuid;

const OTA_BODY_MAX: usize = 32 * 1024;
const ACTIVATE_BODY_MAX: usize = 8 * 1024;

pub(crate) async fn handler(State(state): State<AppState>, request: Request) -> Response {
    let headers = request.headers().clone();
    let identity = match identity(&headers) {
        Ok(value) => value,
        Err(response) => return response,
    };
    let metadata_json = if request.method() == axum::http::Method::POST {
        match ota_metadata(request, &identity.client_id).await {
            Ok(value) => value,
            Err(response) => return response,
        }
    } else {
        metadata(&identity.client_id, None, None).to_string()
    };
    let Some(database) = state.database.as_ref() else {
        return ota_error(StatusCode::SERVICE_UNAVAILABLE, "database_unavailable");
    };
    let now = unix_seconds();
    match database.device_registration(&identity.device_id).await {
        Ok(DeviceRegistration::Registered) => ota_response(&state, &headers, None, true),
        Ok(DeviceRegistration::Blocked) => ota_error(StatusCode::FORBIDDEN, "device_not_admitted"),
        Ok(DeviceRegistration::Unknown) => {
            if !state.config.database.devices.enrollment.enabled {
                return ota_error(StatusCode::FORBIDDEN, "device_not_admitted");
            }
            if !state.admission_gate().is_open() {
                return ota_error(StatusCode::SERVICE_UNAVAILABLE, "server_shutting_down");
            }
            let request = EnrollmentRequest {
                device_id: identity.device_id,
                client_id: identity.client_id,
                metadata_json,
                now,
                ttl_seconds: state.config.database.devices.enrollment.code_ttl_seconds,
                candidates: (0..8).map(|_| candidate()).collect(),
            };
            match database
                .get_or_create_enrollment(
                    request,
                    state.config.database.devices.enrollment.max_pending,
                )
                .await
            {
                Ok(EnrollmentClaim::Pending(pending)) => {
                    if state.config.database.devices.enrollment.transport
                        == crate::config::EnrollmentTransport::Websocket
                    {
                        return ota_response(&state, &headers, None, true);
                    }
                    let remaining = pending.expires_at.saturating_sub(now).max(0) as u64;
                    ota_response(
                        &state,
                        &headers,
                        Some(OtaActivation {
                            code: pending.code,
                            message: "Nhập mã này trong mục Thêm thiết bị trên web.",
                            challenge: pending.challenge,
                            timeout_ms: remaining.saturating_mul(1_000),
                        }),
                        false,
                    )
                }
                Ok(EnrollmentClaim::Registered) => ota_response(&state, &headers, None, true),
                Ok(EnrollmentClaim::Blocked) => {
                    ota_error(StatusCode::FORBIDDEN, "device_not_admitted")
                }
                Err(EnrollmentCreateError::Capacity) => ota_error(
                    StatusCode::SERVICE_UNAVAILABLE,
                    "enrollment_capacity_exceeded",
                ),
                Err(EnrollmentCreateError::CodeUnavailable) => ota_error(
                    StatusCode::SERVICE_UNAVAILABLE,
                    "enrollment_code_unavailable",
                ),
                Err(EnrollmentCreateError::Database) => {
                    ota_error(StatusCode::SERVICE_UNAVAILABLE, "database_unavailable")
                }
            }
        }
        Err(_) => ota_error(StatusCode::SERVICE_UNAVAILABLE, "database_unavailable"),
    }
}

pub(crate) async fn activate(State(state): State<AppState>, request: Request) -> Response {
    if !state.config.database.devices.enrollment.enabled {
        return StatusCode::NOT_FOUND.into_response();
    }
    let headers = request.headers().clone();
    let identity = match identity(&headers) {
        Ok(value) => value,
        Err(response) => return response,
    };
    if let Err(response) = compatible_activate_body(request).await {
        return response;
    }
    let Some(database) = state.database.as_ref() else {
        return ota_error(StatusCode::SERVICE_UNAVAILABLE, "database_unavailable");
    };
    match database.device_registration(&identity.device_id).await {
        Ok(DeviceRegistration::Registered) => empty(StatusCode::OK, &headers, None),
        Ok(DeviceRegistration::Blocked) => ota_error(StatusCode::FORBIDDEN, "device_not_admitted"),
        Ok(DeviceRegistration::Unknown) => match database
            .activation_pending(&identity.device_id, unix_seconds())
            .await
        {
            Ok(Some(true)) => empty(
                StatusCode::ACCEPTED,
                &headers,
                Some((header::RETRY_AFTER, HeaderValue::from_static("3"))),
            ),
            Ok(Some(false)) => ota_error(StatusCode::GONE, "enrollment_code_expired"),
            Ok(None) => ota_error(StatusCode::NOT_FOUND, "enrollment_not_found"),
            Err(_) => ota_error(StatusCode::SERVICE_UNAVAILABLE, "database_unavailable"),
        },
        Err(_) => ota_error(StatusCode::SERVICE_UNAVAILABLE, "database_unavailable"),
    }
}

pub(crate) async fn options(request_headers: HeaderMap) -> Response {
    let mut response = StatusCode::NO_CONTENT.into_response();
    let headers = response.headers_mut();
    headers.insert(
        header::ALLOW,
        HeaderValue::from_static("GET, POST, OPTIONS"),
    );
    headers.insert(
        header::ACCESS_CONTROL_ALLOW_METHODS,
        HeaderValue::from_static("POST, OPTIONS"),
    );
    headers.insert(
        header::ACCESS_CONTROL_ALLOW_HEADERS,
        HeaderValue::from_static("Content-Type, Device-Id, Client-Id, Authorization"),
    );
    apply_cors(headers, &request_headers);
    response
}

fn ota_response(
    state: &AppState,
    request_headers: &HeaderMap,
    activation: Option<OtaActivation>,
    registered: bool,
) -> Response {
    let mut response = axum::Json(OtaResponse {
        server_time: ServerTime {
            timestamp: unix_millis(),
            timezone_offset: 420,
        },
        firmware: Firmware {
            version: "",
            url: "",
        },
        websocket: registered.then(|| OtaWebsocket {
            url: state.config.server.public_ws_url.to_string(),
            token: state.config.auth.token.clone(),
        }),
        activation,
    })
    .into_response();
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    apply_cors(response.headers_mut(), request_headers);
    response
}
fn empty(
    status: StatusCode,
    request_headers: &HeaderMap,
    extra: Option<(axum::http::HeaderName, HeaderValue)>,
) -> Response {
    let mut response = (status, axum::Json(json!({}))).into_response();
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    if let Some((name, value)) = extra {
        response.headers_mut().insert(name, value);
    }
    apply_cors(response.headers_mut(), request_headers);
    response
}
fn ota_error(status: StatusCode, code: &'static str) -> Response {
    (status, axum::Json(json!({"error":{"code":code}}))).into_response()
}

struct Identity {
    device_id: String,
    client_id: String,
}
#[allow(clippy::result_large_err)] // The OTA handler returns protocol errors without translation.
fn identity(headers: &HeaderMap) -> Result<Identity, Response> {
    fn single(headers: &HeaderMap, name: &str) -> Option<String> {
        let values: Vec<_> = headers.get_all(name).iter().collect();
        (values.len() == 1)
            .then(|| values[0].to_str().ok().map(str::to_owned))
            .flatten()
    }
    let device_id = single(headers, "device-id").filter(|value| valid_identity(value));
    let client_id = single(headers, "client-id").filter(|value| valid_identity(value));
    match (device_id, client_id) {
        (Some(device_id), Some(client_id)) => Ok(Identity {
            device_id,
            client_id,
        }),
        _ => Err(ota_error(
            StatusCode::BAD_REQUEST,
            "invalid_device_identity",
        )),
    }
}
fn valid_identity(value: &str) -> bool {
    !value.is_empty() && value.len() <= 128 && value.bytes().all(|byte| byte > 0x1f && byte != 0x7f)
}
#[allow(clippy::result_large_err)] // The OTA handler returns protocol errors without translation.
async fn ota_metadata(request: Request, client_id: &str) -> Result<String, Response> {
    let content_type = request
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("");
    if !content_type
        .split(';')
        .next()
        .is_some_and(|value| value.trim().eq_ignore_ascii_case("application/json"))
    {
        return Err(ota_error(
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            "unsupported_media_type",
        ));
    }
    let bytes = to_bytes(request.into_body(), OTA_BODY_MAX)
        .await
        .map_err(|_| ota_error(StatusCode::PAYLOAD_TOO_LARGE, "request_too_large"))?;
    let value = if bytes.is_empty() {
        Value::Object(Default::default())
    } else {
        serde_json::from_slice::<Value>(&bytes)
            .map_err(|_| ota_error(StatusCode::BAD_REQUEST, "invalid_json"))?
    };
    let object = value
        .as_object()
        .ok_or_else(|| ota_error(StatusCode::BAD_REQUEST, "validation_failed"))?;
    let firmware = object
        .get("application")
        .and_then(Value::as_object)
        .and_then(|application| application.get("version"))
        .and_then(Value::as_str)
        .filter(|value| value.len() <= 256)
        .map(str::to_owned);
    let board = object.get("board").and_then(Value::as_object);
    let kind = board
        .and_then(|board| board.get("type"))
        .and_then(Value::as_str)
        .filter(|value| value.len() <= 256)
        .map(str::to_owned);
    let name = board
        .and_then(|board| board.get("name"))
        .and_then(Value::as_str)
        .filter(|value| value.len() <= 256)
        .map(str::to_owned);
    Ok(metadata(client_id, firmware, kind.zip(name)).to_string())
}
fn metadata(client_id: &str, firmware: Option<String>, board: Option<(String, String)>) -> Value {
    let mut output = serde_json::Map::from_iter([
        (String::from("source"), json!("activation_code")),
        (String::from("client_id"), json!(client_id)),
    ]);
    if let Some(version) = firmware {
        output.insert("firmware".into(), json!({"version":version}));
    }
    if let Some((kind, name)) = board {
        output.insert("board".into(), json!({"type":kind,"name":name}));
    }
    Value::Object(output)
}
#[allow(clippy::result_large_err)] // The activation handler returns protocol errors without translation.
async fn compatible_activate_body(request: Request) -> Result<(), Response> {
    let bytes = to_bytes(request.into_body(), ACTIVATE_BODY_MAX)
        .await
        .map_err(|_| ota_error(StatusCode::PAYLOAD_TOO_LARGE, "request_too_large"))?;
    if bytes.is_empty() {
        return Ok(());
    }
    serde_json::from_slice::<Value>(&bytes)
        .ok()
        .and_then(|value| value.is_object().then_some(()))
        .ok_or_else(|| ota_error(StatusCode::BAD_REQUEST, "invalid_json"))
}
pub(super) fn unix_seconds() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
        .min(i64::MAX as u64) as i64
}
fn unix_millis() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(i64::MAX as u128) as i64
}
pub(super) fn candidate() -> (String, String) {
    loop {
        let uuid = Uuid::new_v4();
        let bytes = uuid.as_bytes();
        let random = u64::from_be_bytes(bytes[8..16].try_into().expect("UUID tail is eight bytes"))
            & ((1_u64 << 62) - 1);
        let limit = ((1_u64 << 62) / 1_000_000) * 1_000_000;
        if random < limit {
            return (
                format!("{:06}", random % 1_000_000),
                format!("{}{}", uuid.simple(), Uuid::new_v4().simple()),
            );
        }
    }
}
fn apply_cors(response_headers: &mut HeaderMap, request_headers: &HeaderMap) {
    let Some(origin) = request_headers.get(header::ORIGIN) else {
        return;
    };
    let Ok(origin_text) = origin.to_str() else {
        return;
    };
    let Ok(origin_url) = Url::parse(origin_text) else {
        return;
    };
    let Some(host) = origin_url.host_str() else {
        return;
    };
    if !(host == "localhost"
        || host
            .parse::<std::net::IpAddr>()
            .is_ok_and(|address| address.is_loopback()))
    {
        return;
    }
    response_headers.insert(header::ACCESS_CONTROL_ALLOW_ORIGIN, origin.clone());
    response_headers.insert(header::VARY, HeaderValue::from_static("Origin"));
}
