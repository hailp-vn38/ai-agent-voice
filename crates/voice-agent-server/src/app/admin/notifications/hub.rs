use super::super::*;
use crate::services::notification_hub::{
    ExternalChannelKind, InboxListQuery, NotificationHub, NotificationHubError,
    NotificationSettingsUpdate, NotificationSseEvent, TestChannelInput,
};
use axum::response::sse::{Event, KeepAlive, Sse};
use std::{convert::Infallible, sync::Arc, time::Duration};
use tokio::sync::broadcast::error::RecvError;

#[allow(clippy::result_large_err)]
fn notification_hub(state: &AppState, req: &Request) -> Result<Arc<NotificationHub>, Response> {
    state
        .notification_hub
        .as_ref()
        .cloned()
        .ok_or_else(|| error(req, StatusCode::SERVICE_UNAVAILABLE, "database_unavailable"))
}

fn hub_error(req: &Request, err: NotificationHubError) -> Response {
    match err {
        NotificationHubError::InvalidInput(code) => error(req, StatusCode::BAD_REQUEST, code),
        NotificationHubError::NotFound => error(req, StatusCode::NOT_FOUND, "not_found"),
        NotificationHubError::RevisionConflict => {
            error(req, StatusCode::CONFLICT, "revision_conflict")
        }
        NotificationHubError::EncryptionUnavailable => error(
            req,
            StatusCode::SERVICE_UNAVAILABLE,
            "credential_encryption_unavailable",
        ),
        NotificationHubError::Busy => error(req, StatusCode::SERVICE_UNAVAILABLE, "database_busy"),
        NotificationHubError::Database(sql_err) => sql_error(req, &sql_err),
    }
}

pub(in crate::app::admin) async fn list_inbox(
    State(state): State<AppState>,
    Query(query): Query<InboxListQuery>,
    req: Request,
) -> Response {
    let hub = match notification_hub(&state, &req) {
        Ok(h) => h,
        Err(resp) => return resp,
    };
    match hub.list_inbox(&query).await {
        Ok(page) => Json(page).into_response(),
        Err(err) => hub_error(&req, err),
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(in crate::app::admin) struct MarkReadInput {
    #[serde(default)]
    pub ids: Vec<String>,
    #[serde(default)]
    pub all: bool,
}

pub(in crate::app::admin) async fn mark_read(State(state): State<AppState>, req: Request) -> Response {
    let hub = match notification_hub(&state, &req) {
        Ok(h) => h,
        Err(resp) => return resp,
    };
    let (req, input): (_, MarkReadInput) = match json(req).await {
        Ok(v) => v,
        Err(resp) => return resp,
    };
    match hub.mark_read(&input.ids, input.all).await {
        Ok(out) => Json(out).into_response(),
        Err(err) => hub_error(&req, err),
    }
}

pub(in crate::app::admin) async fn delete_item(
    State(state): State<AppState>,
    Path(item_id): Path<String>,
    req: Request,
) -> Response {
    let hub = match notification_hub(&state, &req) {
        Ok(h) => h,
        Err(resp) => return resp,
    };
    match hub.delete_inbox(Some(&item_id), false).await {
        Ok(out) => Json(out).into_response(),
        Err(err) => hub_error(&req, err),
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(in crate::app::admin) struct PurgeInput {
    #[serde(default = "default_true")]
    pub read_only: bool,
}

const fn default_true() -> bool {
    true
}

pub(in crate::app::admin) async fn purge_inbox(
    State(state): State<AppState>,
    req: Request,
) -> Response {
    let hub = match notification_hub(&state, &req) {
        Ok(h) => h,
        Err(resp) => return resp,
    };
    let (req, input): (_, PurgeInput) = match json(req).await {
        Ok(v) => v,
        Err(resp) => return resp,
    };
    match hub.delete_inbox(None, input.read_only).await {
        Ok(out) => Json(out).into_response(),
        Err(err) => hub_error(&req, err),
    }
}

pub(in crate::app::admin) async fn settings(State(state): State<AppState>, req: Request) -> Response {
    let hub = match notification_hub(&state, &req) {
        Ok(h) => h,
        Err(resp) => return resp,
    };
    match hub.get_settings().await {
        Ok(view) => Json(view).into_response(),
        Err(err) => hub_error(&req, err),
    }
}

pub(in crate::app::admin) async fn save_settings(
    State(state): State<AppState>,
    req: Request,
) -> Response {
    let hub = match notification_hub(&state, &req) {
        Ok(h) => h,
        Err(resp) => return resp,
    };
    let expected_rev = match expected(req.headers()) {
        Ok(v) => v,
        Err(code) => return error(&req, StatusCode::BAD_REQUEST, code),
    };
    let (req, input): (_, NotificationSettingsUpdate) = match json(req).await {
        Ok(v) => v,
        Err(resp) => return resp,
    };
    let req_id = id(&req);
    match hub.update_settings(expected_rev, input, req_id).await {
        Ok(view) => Json(view).into_response(),
        Err(err) => hub_error(&req, err),
    }
}

pub(in crate::app::admin) async fn test_channel(
    State(state): State<AppState>,
    Path(channel): Path<String>,
    req: Request,
) -> Response {
    let hub = match notification_hub(&state, &req) {
        Ok(h) => h,
        Err(resp) => return resp,
    };
    let Some(channel_kind) = ExternalChannelKind::parse(&channel) else {
        return error(&req, StatusCode::NOT_FOUND, "not_found");
    };
    let (req, input): (_, TestChannelInput) = match json(req).await {
        Ok(v) => v,
        Err(resp) => return resp,
    };
    match hub.test_channel(channel_kind, input).await {
        Ok(out) => Json(out).into_response(),
        Err(err) => hub_error(&req, err),
    }
}

pub(in crate::app::admin) async fn stream(State(state): State<AppState>, req: Request) -> Response {
    let hub = match notification_hub(&state, &req) {
        Ok(h) => h,
        Err(resp) => return resp,
    };
    let rx = hub.subscribe_web();
    let stopping = state.lifecycle.stopping().clone();

    let messages = futures_util::stream::unfold(
        (rx, stopping, true),
        |(mut rx, stopping, first)| async move {
            if first {
                return Some((
                    Ok::<Event, Infallible>(Event::default().event("ready").data("{}")),
                    (rx, stopping, false),
                ));
            }
            let recv = tokio::select! {
                biased;
                _ = stopping.cancelled() => return None,
                recv = rx.recv() => recv,
            };
            match recv {
                Ok(NotificationSseEvent::NotificationCreated(item)) => {
                    let data = serde_json::to_string(&item).unwrap_or_else(|_| "{}".to_owned());
                    Some((
                        Ok(Event::default().event("notification_created").data(data)),
                        (rx, stopping, false),
                    ))
                }
                Ok(NotificationSseEvent::UnreadCountChanged { unread_count }) => {
                    let data = serde_json::json!({ "unread_count": unread_count }).to_string();
                    Some((
                        Ok(Event::default().event("unread_count_changed").data(data)),
                        (rx, stopping, false),
                    ))
                }
                Err(RecvError::Lagged(_)) => Some((
                    Ok(Event::default().event("resync_required").data("{}")),
                    (rx, stopping, false),
                )),
                Err(RecvError::Closed) => None,
            }
        },
    );

    let mut response = Sse::new(messages)
        .keep_alive(
            KeepAlive::new()
                .interval(Duration::from_secs(15))
                .text("keepalive"),
        )
        .into_response();
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
}
