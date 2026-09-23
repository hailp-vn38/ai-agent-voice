use super::AppState;
use crate::protocol::{Firmware, OtaResponse, OtaWebsocket, ServerTime};
use axum::{
    Json,
    extract::State,
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
};
use std::time::{SystemTime, UNIX_EPOCH};
use url::Url;

pub(crate) async fn handler(State(state): State<AppState>, request_headers: HeaderMap) -> Response {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64;
    let mut response = Json(OtaResponse {
        server_time: ServerTime {
            timestamp,
            timezone_offset: 420,
        },
        firmware: Firmware {
            version: "",
            url: "",
        },
        websocket: OtaWebsocket {
            url: state.config.server.public_ws_url.to_string(),
            token: state.config.auth.token.clone(),
        },
    })
    .into_response();
    apply_cors(response.headers_mut(), &request_headers);
    response
}

/// Answers browser and device preflight requests without creating a voice session.
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

/// Only local browser tooling may read OTA's optional bearer token cross-origin.
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
    let is_loopback = host == "localhost"
        || host
            .parse::<std::net::IpAddr>()
            .is_ok_and(|address| address.is_loopback());
    if !is_loopback {
        return;
    }
    response_headers.insert(header::ACCESS_CONTROL_ALLOW_ORIGIN, origin.clone());
    response_headers.insert(header::VARY, HeaderValue::from_static("Origin"));
}
