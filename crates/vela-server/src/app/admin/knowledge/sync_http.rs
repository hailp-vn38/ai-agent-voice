use super::*;
use axum::Json;
use serde::Serialize;
use wiki_core::{DisabledReason, SyncError, SyncReport, SyncStatus};

#[derive(Serialize)]
struct SyncConfigurationView<'a> {
    remote_url: &'a str,
    branch: &'a str,
    auto: bool,
    interval_min: u64,
}

#[derive(Serialize)]
struct SyncView<'a> {
    status: SyncStatus,
    configuration: SyncConfigurationView<'a>,
}

pub(in crate::app::admin) async fn knowledge_sync(
    State(state): State<AppState>,
    req: Request,
) -> Response {
    if let Err(response) = super::wiki(&state, &req) {
        return response;
    }
    let config = &state.config.wiki.sync;
    let status = if !config.enabled {
        SyncStatus::Disabled {
            reason: DisabledReason::NotConfigured,
        }
    } else {
        state
            .wiki_sync_service
            .get()
            .map(|service| service.status())
            .unwrap_or_else(|| SyncStatus::Error {
                kind: wiki_core::SyncErrorKind::Git,
                message: "sync_starting".into(),
            })
    };
    Json(SyncView {
        status,
        configuration: SyncConfigurationView {
            remote_url: if config.enabled {
                &config.remote_url
            } else {
                ""
            },
            branch: &config.branch,
            auto: config.enabled,
            interval_min: config.interval_min,
        },
    })
    .into_response()
}

pub(in crate::app::admin) async fn knowledge_sync_now(
    State(state): State<AppState>,
    req: Request,
) -> Response {
    if let Err(response) = super::wiki(&state, &req) {
        return response;
    }
    if !state.config.wiki.sync.enabled {
        return error(&req, StatusCode::CONFLICT, "sync_disabled");
    }
    let Some(service) = state.wiki_sync_service.get().cloned() else {
        return error(&req, StatusCode::SERVICE_UNAVAILABLE, "sync_unavailable");
    };
    if service.is_sync_in_progress() {
        return error(&req, StatusCode::CONFLICT, "sync_in_progress");
    }
    match service.sync_now().await {
        Ok(report) => Json::<SyncReport>(report).into_response(),
        Err(SyncError::Git(message)) if message == "sync_in_progress" => {
            error(&req, StatusCode::CONFLICT, "sync_in_progress")
        }
        Err(error_value) => sync_error(&req, error_value),
    }
}

fn sync_error(req: &Request, error_value: SyncError) -> Response {
    let code = match error_value.kind() {
        wiki_core::SyncErrorKind::GitMissing => "sync_no_git",
        wiki_core::SyncErrorKind::ReadOnly => "sync_read_only",
        wiki_core::SyncErrorKind::Auth => "sync_auth",
        wiki_core::SyncErrorKind::Network => "sync_network",
        wiki_core::SyncErrorKind::Timeout => "sync_timeout",
        wiki_core::SyncErrorKind::FileTooLarge => "sync_file_too_large",
        wiki_core::SyncErrorKind::UnsafeRemote => "sync_unsafe_remote",
        wiki_core::SyncErrorKind::Vault => "sync_vault",
        wiki_core::SyncErrorKind::Git => "sync_git",
    };
    let status = match error_value.kind() {
        wiki_core::SyncErrorKind::Auth | wiki_core::SyncErrorKind::UnsafeRemote => {
            StatusCode::BAD_GATEWAY
        }
        wiki_core::SyncErrorKind::Network | wiki_core::SyncErrorKind::Timeout => {
            StatusCode::SERVICE_UNAVAILABLE
        }
        wiki_core::SyncErrorKind::ReadOnly => StatusCode::CONFLICT,
        _ => StatusCode::BAD_REQUEST,
    };
    error(req, status, code)
}
