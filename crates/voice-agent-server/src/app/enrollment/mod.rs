//! Enrollment Session is control-plane transport, separate from the Voice Session actor.
mod socket;

use super::{AppState, ota};
use crate::{
    config::EnrollmentTransport,
    database::{DeviceRegistration, EnrollmentClaim, EnrollmentPending, EnrollmentRequest},
    services::device_enrollment::EnrollmentRuntime,
};
use axum::{http::StatusCode, response::{IntoResponse, Response}};
use std::sync::Arc;
use tokio::sync::OwnedSemaphorePermit;

pub(super) enum Route {
    Voice,
    Pending(PendingConnection),
}

pub(super) struct PendingConnection {
    device_id: String,
    pending: EnrollmentPending,
    runtime: Arc<EnrollmentRuntime>,
    _permit: OwnedSemaphorePermit,
}

/// Called after wire identity and transport bearer validation, before provider resolution.
pub(super) async fn route(
    state: &AppState,
    device_id: &str,
    client_id: &str,
) -> Result<Route, Response> {
    let config = &state.config.database.devices.enrollment;
    if !config.enabled || config.transport != EnrollmentTransport::Websocket {
        return Ok(Route::Voice);
    }
    let database = state.database.as_ref().ok_or_else(|| unavailable("database_unavailable"))?;
    match database.device_registration(device_id).await {
        Ok(DeviceRegistration::Registered) => return Ok(Route::Voice),
        Ok(DeviceRegistration::Blocked) => return Err(denied()),
        Err(_) => return Err(unavailable("database_unavailable")),
        Ok(DeviceRegistration::Unknown) => {}
    }
    if !state.admission_gate().is_open() {
        return Err(unavailable("server_shutting_down"));
    }
    let runtime = state.enrollment_runtime.clone().ok_or_else(|| unavailable("enrollment_prompt_unavailable"))?;
    let permit = runtime.try_connection().ok_or_else(|| unavailable("enrollment_capacity_exceeded"))?;
    let request = EnrollmentRequest {
        device_id: device_id.to_owned(),
        client_id: client_id.to_owned(),
        metadata_json: serde_json::json!({"source":"websocket","client_id":client_id}).to_string(),
        now: ota::unix_seconds(),
        ttl_seconds: config.code_ttl_seconds,
        candidates: (0..8).map(|_| ota::candidate()).collect(),
    };
    match database.get_or_create_enrollment(request, config.max_pending).await {
        Ok(EnrollmentClaim::Registered) => Ok(Route::Voice),
        Ok(EnrollmentClaim::Blocked) => Err(denied()),
        Ok(EnrollmentClaim::Pending(pending)) => Ok(Route::Pending(PendingConnection {
            device_id: device_id.to_owned(), pending, runtime, _permit: permit,
        })),
        Err(error) => Err((StatusCode::SERVICE_UNAVAILABLE, error.to_string()).into_response()),
    }
}

fn unavailable(reason: &'static str) -> Response {
    (StatusCode::SERVICE_UNAVAILABLE, reason).into_response()
}
fn denied() -> Response {
    (StatusCode::FORBIDDEN, "device not admitted").into_response()
}

pub(super) use socket::run;
