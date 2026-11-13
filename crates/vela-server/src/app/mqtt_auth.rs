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
    services::device_push::{BrokerClient, DevicePush},
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

/// The module decides what each account may do; a Device additionally has to be registered
/// right now, so blocking it takes effect on its next broker question.
async fn allowed(state: &AppState, client: Option<BrokerClient<'_>>) -> bool {
    match client {
        None => false,
        Some(BrokerClient::Service) => true,
        Some(BrokerClient::Device(device_id)) => {
            let Some(database) = state.database.as_ref() else {
                return false;
            };
            matches!(
                database.device_registration(device_id).await,
                Ok(DeviceRegistration::Registered)
            )
        }
    }
}

pub(crate) async fn user(
    State(state): State<AppState>,
    body: Result<Json<UserCheck>, JsonRejection>,
) -> Response {
    let (Some(push), Ok(Json(check))) = (state.device_push.as_ref(), body) else {
        return verdict(false);
    };
    verdict(allowed(&state, push.authenticate(&check.username, &check.password)).await)
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
    let client = DevicePush::authorize_topic(&check.username, &check.topic, check.acc);
    verdict(allowed(&state, client).await)
}
