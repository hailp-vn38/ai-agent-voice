mod artifacts;
mod attachments;
mod jobs;
mod settings;
mod skills;
#[cfg(test)]
mod tests;

pub(super) use artifacts::{artifact, list_artifacts};
pub(super) use jobs::{
    cancel, create, delete, events, followup, get, history, list, retry, stream,
};
pub(super) use settings::{create_agent, delete_agent, save_settings, settings};
pub(super) use skills::{delete_skill, get_skill, save_skill};

use super::*;
use orchestrator_flow_rs::{ErrorCode, TaskError, TaskManager};
use std::sync::Arc;

#[allow(clippy::result_large_err)]
pub(self) fn task_manager(state: &AppState, req: &Request) -> Result<Arc<TaskManager>, Response> {
    state
        .task_manager
        .as_ref()
        .cloned()
        .ok_or_else(|| error(req, StatusCode::SERVICE_UNAVAILABLE, "runtime_unavailable"))
}

#[allow(clippy::result_large_err)]
pub(self) fn task_manager_for_submit(
    state: &AppState,
    req: &Request,
) -> Result<Arc<TaskManager>, Response> {
    let disabled = state
        .admin_task_control
        .lock()
        .expect("admin_task_control mutex poisoned")
        .background_enabled_override
        == Some(false);
    if disabled {
        return Err(error(
            req,
            StatusCode::SERVICE_UNAVAILABLE,
            "runtime_unavailable",
        ));
    }
    task_manager(state, req)
}

pub(self) fn task_failure(request: &Request, cause: TaskError) -> Response {
    if cause.code() == ErrorCode::OrchestratorError {
        let msg = cause.to_string();
        if msg.contains("WORKSPACE_QUOTA_EXCEEDED") {
            return error(
                request,
                StatusCode::INSUFFICIENT_STORAGE,
                "workspace_quota_exceeded",
            );
        }
        if msg.contains("ARTIFACT_TOO_LARGE") {
            return error(request, StatusCode::PAYLOAD_TOO_LARGE, "artifact_too_large");
        }
    }
    let status = match cause.code() {
        ErrorCode::InvalidInput | ErrorCode::InvalidConfig => StatusCode::BAD_REQUEST,
        ErrorCode::NotFound => StatusCode::NOT_FOUND,
        ErrorCode::PermissionDenied => StatusCode::FORBIDDEN,
        ErrorCode::StoreAlreadyOwned | ErrorCode::SessionBusy => StatusCode::CONFLICT,
        ErrorCode::NotReady | ErrorCode::ShuttingDown => StatusCode::SERVICE_UNAVAILABLE,
        ErrorCode::StoreError | ErrorCode::OrchestratorError => StatusCode::SERVICE_UNAVAILABLE,
    };
    error(request, status, cause.code().as_str())
}
