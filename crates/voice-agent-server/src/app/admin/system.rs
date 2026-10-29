//! Safe aggregate Admin status; it deliberately excludes paths, configs, secrets and failures.
use super::*;
use crate::{
    database::AuditLogFilter,
    services::system_logs::{LogFilter, LogStreamItem},
};
use axum::response::sse::{Event, KeepAlive, Sse};
use std::{convert::Infallible, time::Duration};

pub(super) async fn get_system(State(state): State<AppState>, request: Request) -> Response {
    if state.database.is_none() {
        return error(
            &request,
            StatusCode::SERVICE_UNAVAILABLE,
            "database_unavailable",
        );
    }
    match state.system_status().summary().await {
        Ok(summary) => Json(summary).into_response(),
        Err(cause) => sql_error(&request, &cause),
    }
}

pub(super) async fn get_resources(State(state): State<AppState>, request: Request) -> Response {
    let Some(snapshot) = state.system_metrics.latest() else {
        return error(
            &request,
            StatusCode::SERVICE_UNAVAILABLE,
            "system_metrics_unavailable",
        );
    };
    let mut response = Json(snapshot).into_response();
    response.headers_mut().insert(
        axum::http::header::CACHE_CONTROL,
        axum::http::HeaderValue::from_static("no-store"),
    );
    response
}

pub(super) async fn get_logs(State(state): State<AppState>, request: Request) -> Response {
    let Query(filter) = match Query::<LogFilter>::try_from_uri(request.uri()) {
        Ok(q) => q,
        Err(_) => return error(&request, StatusCode::BAD_REQUEST, "invalid_query"),
    };
    let page = match state.system_logs.query(&filter) {
        Ok(page) => page,
        Err(code) => return error(&request, StatusCode::BAD_REQUEST, code),
    };
    let mut response = Json(page).into_response();
    response.headers_mut().insert(
        axum::http::header::CACHE_CONTROL,
        axum::http::HeaderValue::from_static("no-store"),
    );
    response
}

pub(super) async fn stream_logs(State(state): State<AppState>, request: Request) -> Response {
    let Query(filter) = match Query::<LogFilter>::try_from_uri(request.uri()) {
        Ok(q) => q,
        Err(_) => return error(&request, StatusCode::BAD_REQUEST, "invalid_query"),
    };
    let sub = match state.system_logs.subscribe(&filter) {
        Ok(sub) => sub,
        Err(code) => return error(&request, StatusCode::BAD_REQUEST, code),
    };
    let stopping = state.lifecycle.stopping().clone();

    let messages = futures_util::stream::unfold(
        (sub, stopping, true),
        |(mut sub, stopping, first)| async move {
            if first {
                let ready_payload = serde_json::json!({
                    "latest_seq": sub.latest_seq_at_subscribe,
                    "capacity": sub.capacity,
                })
                .to_string();
                return Some((
                    Ok::<Event, Infallible>(Event::default().event("ready").data(ready_payload)),
                    (sub, stopping, false),
                ));
            }
            let item = tokio::select! {
                biased;
                _ = stopping.cancelled() => return None,
                item = sub.recv() => item,
            };
            match item {
                Some(LogStreamItem::Entry(entry)) => {
                    let data =
                        serde_json::to_string(entry.as_ref()).unwrap_or_else(|_| "{}".into());
                    Some((
                        Ok(Event::default().event("log_entry").data(data)),
                        (sub, stopping, false),
                    ))
                }
                Some(LogStreamItem::Lagged { missed, latest_seq }) => {
                    let data = serde_json::json!({
                        "missed": missed,
                        "latest_seq": latest_seq,
                    })
                    .to_string();
                    Some((
                        Ok(Event::default().event("lagged").data(data)),
                        (sub, stopping, false),
                    ))
                }
                None => None,
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
    response.headers_mut().insert(
        axum::http::header::CACHE_CONTROL,
        axum::http::HeaderValue::from_static("no-store"),
    );
    response
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct AuditLogsQuery {
    #[serde(default)]
    page: Option<u32>,
    #[serde(default)]
    page_size: Option<u32>,
    #[serde(default)]
    resource_type: Option<String>,
    #[serde(default)]
    outcome: Option<String>,
}

pub(super) async fn get_audit_logs(State(state): State<AppState>, request: Request) -> Response {
    let Some(db) = state.database.as_ref() else {
        return error(
            &request,
            StatusCode::SERVICE_UNAVAILABLE,
            "database_unavailable",
        );
    };
    let Query(query) = match Query::<AuditLogsQuery>::try_from_uri(request.uri()) {
        Ok(q) => q,
        Err(_) => return error(&request, StatusCode::BAD_REQUEST, "invalid_query"),
    };
    let (page, page_size) = match page_bounds(query.page, query.page_size) {
        Ok(bounds) => bounds,
        Err(code) => return error(&request, StatusCode::BAD_REQUEST, code),
    };
    let clean_opt = |value: Option<String>| -> Result<Option<String>, &'static str> {
        let Some(raw) = value else {
            return Ok(None);
        };
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            return Ok(None);
        }
        if !valid_identity(trimmed) {
            return Err("invalid_query");
        }
        Ok(Some(trimmed.to_owned()))
    };
    let filter = AuditLogFilter {
        page,
        page_size,
        resource_type: match clean_opt(query.resource_type) {
            Ok(v) => v,
            Err(code) => return error(&request, StatusCode::BAD_REQUEST, code),
        },
        outcome: match clean_opt(query.outcome) {
            Ok(v) => v,
            Err(code) => return error(&request, StatusCode::BAD_REQUEST, code),
        },
    };
    match db.list_audit_events(&filter).await {
        Ok(page) => {
            let mut response = Json(page).into_response();
            response.headers_mut().insert(
                axum::http::header::CACHE_CONTROL,
                axum::http::HeaderValue::from_static("no-store"),
            );
            response
        }
        Err(cause) => sql_error(&request, &cause),
    }
}
