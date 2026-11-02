use super::args::{error, unknown};
use crate::providers::llm::AgentTaskError;
use orchestrator_agent_rs::{
    CancelReason, OperationUpdate, ToolContext, ToolError, ToolOperationStatus as Status,
};
use serde_json::json;

pub(super) fn provider_error(failure: AgentTaskError) -> ToolError {
    match failure {
        AgentTaskError::RetryLater(message) => error("TOOL_RETRY_LATER", message),
        AgentTaskError::OutcomeUnknown => error(
            "TOOL_OUTCOME_UNKNOWN",
            "Remote outcome is unknown; reconcile before another send",
        ),
        AgentTaskError::Busy => error(
            "MUSE_TASK_BUSY",
            json!({"next_action":"collect","message":"Previous Muse turn is still running"})
                .to_string(),
        ),
        AgentTaskError::NoSession => error("MUSE_TASK_NO_SESSION", "Use send before collect"),
        AgentTaskError::SessionLost => error(
            "MUSE_SESSION_LOST",
            "Pinned Muse session was lost; do not resend automatically",
        ),
        AgentTaskError::NeedsReauth => error(
            "PROVIDER_AUTH_FAILED",
            "Muse account requires reauthentication",
        ),
        AgentTaskError::Rejected(message) => error("MUSE_TASK_REJECTED", message),
        AgentTaskError::Timeout { partial } => error(
            "MUSE_TASK_TIMEOUT",
            json!({"partial_text":partial,"next_action":"collect"}).to_string(),
        ),
        AgentTaskError::Stalled { partial } => error(
            "MUSE_TASK_STALLED",
            json!({"partial_text":partial,"next_action":"collect"}).to_string(),
        ),
    }
}

pub(super) async fn cancelled(ctx: &ToolContext) -> Result<ToolError, ToolError> {
    if ctx.cancel_reason() == Some(CancelReason::Shutdown) {
        return Ok(error(
            "TOOL_RETRY_LATER",
            "Server is shutting down; keep the Muse operation for recovery",
        ));
    }
    ctx.operation()
        .advance(OperationUpdate {
            status: Some(Status::Cancelled),
            ..Default::default()
        })
        .await
        .map_err(unknown)?;
    Ok(error("CANCELLED", "Muse task invocation was cancelled"))
}

pub(super) async fn terminal_failure(ctx: &ToolContext, failure: ToolError) -> ToolError {
    let status = match failure.code.as_str() {
        "TOOL_RETRY_LATER" | "TOOL_OUTCOME_UNKNOWN" => return failure,
        "MUSE_TASK_TIMEOUT" => Status::Expired,
        "CANCELLED" => Status::Cancelled,
        _ => Status::Failed,
    };
    match ctx
        .operation()
        .advance(OperationUpdate {
            status: Some(status),
            result_json: Some(json!({"code":failure.code,"message":failure.message}).to_string()),
            ..Default::default()
        })
        .await
    {
        Ok(()) => failure,
        Err(failure) => unknown(failure),
    }
}
