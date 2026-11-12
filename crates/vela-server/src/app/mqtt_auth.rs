//! HTTP endpoints Mosquitto (mosquitto-go-auth `http` backend, `params_mode json`,
//! `response_mode status`) calls to authenticate clients and authorize topics.
//!
//! Answers are status-only: 200 allows, 403 denies, with an empty body so no reason leaks.
//! Any failure (malformed body, database error) denies. Routes exist only when the feature is on.

use axum::{
    Json,
    extract::{State, rejection::JsonRejection},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde::Deserialize;

use crate::{
    app::state::AppState,
    database::DeviceRegistration,
    services::device_push::{DevicePush, TopicAccess},
};

#[derive(Deserialize)]
pub(crate) struct UserCheck {
    username: String,
    #[serde(default)]
    password: String,
}

#[derive(Deserialize)]
pub(crate) struct AclCheck {
    username: String,
    topic: String,
    acc: i64,
}

fn verdict(allowed: bool) -> Response {
    if allowed {
        StatusCode::OK
    } else {
        StatusCode::FORBIDDEN
    }
    .into_response()
}

async fn device_registered(state: &AppState, device_id: &str) -> bool {
    let Some(database) = state.database.as_ref() else {
        return false;
    };
    matches!(
        database.device_registration(device_id).await,
        Ok(DeviceRegistration::Registered)
    )
}

pub(crate) async fn user(
    State(state): State<AppState>,
    body: Result<Json<UserCheck>, JsonRejection>,
) -> Response {
    let (Some(push), Ok(Json(check))) = (state.device_push.as_ref(), body) else {
        return verdict(false);
    };
    if DevicePush::is_service_username(&check.username) {
        return verdict(push.verify_service_password(&check.password));
    }
    verdict(
        push.verify_device_password(&check.username, &check.password)
            && device_registered(&state, &check.username).await,
    )
}

/// Superuser bypasses ACLs, so nobody is ever one.
pub(crate) async fn superuser() -> Response {
    verdict(false)
}

pub(crate) async fn acl(
    State(state): State<AppState>,
    body: Result<Json<AclCheck>, JsonRejection>,
) -> Response {
    let (Some(_), Ok(Json(check))) = (state.device_push.as_ref(), body) else {
        return verdict(false);
    };
    let Some(access) = TopicAccess::from_code(check.acc) else {
        return verdict(false);
    };
    if DevicePush::is_service_username(&check.username) {
        return verdict(DevicePush::service_topic_allowed(&check.topic, access));
    }
    verdict(
        DevicePush::device_topic_allowed(&check.username, &check.topic, access)
            && device_registered(&state, &check.username).await,
    )
}
