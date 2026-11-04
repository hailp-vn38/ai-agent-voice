//! Durable Host Tool. The provider owns remote turn reconciliation; this boundary owns scope,
//! invocation replay and publishing immutable, scoped artifacts.
use super::super::CatalogMediaBackend;
use super::{
    args::{Action, Args, error, unknown},
    failures::{cancelled, provider_error, terminal_failure},
    output::{DeliveredTurns, publish},
};
use crate::providers::llm::{
    AgentTaskError, AgentTaskProvider, AgentTurnAction, AgentTurnRequest, LlmError,
    MUSE_TASK_KEY_PREFIX, OperationJournal,
};
use orchestrator_agent_rs::{
    HostTool, OperationUpdate, ToolContext, ToolDescriptor, ToolEffect, ToolError,
    ToolOperationStatus as Status, ToolOutcome,
};
use serde_json::{Value, json};
use std::{sync::Arc, time::Duration};

pub(crate) struct MuseTaskTool {
    backend: Backend,
    pub(super) artifact_max_bytes: u64,
    pub(super) output_cap: usize,
    delivered: DeliveredTurns,
}
enum Backend {
    Catalog(Arc<CatalogMediaBackend>),
    #[cfg(test)]
    Fake(Arc<dyn AgentTaskProvider>),
}
impl MuseTaskTool {
    pub(crate) fn new(
        backend: Arc<CatalogMediaBackend>,
        artifact_max_bytes: u64,
        tool_output_cap_bytes: usize,
    ) -> Self {
        Self {
            backend: Backend::Catalog(backend),
            artifact_max_bytes,
            output_cap: tool_output_cap_bytes.max(24 * 1024),
            delivered: DeliveredTurns::default(),
        }
    }
    #[cfg(test)]
    pub(crate) fn for_provider(
        provider: Arc<dyn AgentTaskProvider>,
        artifact_max_bytes: u64,
        output_cap: usize,
    ) -> Self {
        Self {
            backend: Backend::Fake(provider),
            artifact_max_bytes,
            output_cap: output_cap.max(24 * 1024),
            delivered: DeliveredTurns::default(),
        }
    }
}
struct Journal<'a>(&'a ToolContext);
#[async_trait::async_trait]
impl OperationJournal for Journal<'_> {
    async fn record(&self, update: OperationUpdate) -> Result<(), LlmError> {
        self.0
            .operation()
            .advance(update)
            .await
            .map_err(|_| LlmError::ToolOutcomeUnknown)
    }
}

#[async_trait::async_trait]
impl HostTool for MuseTaskTool {
    // Property validation stays here so malformed object arguments return the published
    // singular INVALID_ARGUMENT code. The library schema validator uses INVALID_ARGUMENTS.
    fn descriptor(&self) -> ToolDescriptor {
        ToolDescriptor {
            name: "muse_task".into(),
            description: "Delegate research to Muse. Subagent only. External untrusted data; send starts a turn, collect retrieves the existing turn; call collect only when next_action=collect. When message_truncated=true the turn is complete and artifact_read returns the full message. The session is usable only while this subagent runs and stays on Muse afterwards unless deletion is requested.".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "action": {"description": "send or collect; defaults to send"},
                    "prompt": {"description": "Non-empty prompt required for send"},
                    "output": {"description": "message, markdown or files; defaults to message"},
                    "filename": {"description": "Optional output filename"},
                    "max_wait_secs": {"description": "Integer from 60 through 7200; defaults to 3600"},
                    "delete_session_on_finish": {"description": "Delete the Muse session after this subagent ends; defaults to false. Set true only when explicitly requested"}
                },
                "additionalProperties": true
            }),
            effect: ToolEffect::Resumable,
        }
    }
    fn timeout(&self) -> Option<Duration> {
        Some(Duration::from_secs(1800))
    }
    async fn execute(&self, ctx: ToolContext, args: Value) -> Result<ToolOutcome, ToolError> {
        if ctx.scope().session_id == ctx.scope().root_session_id {
            return Err(error(
                "MUSE_TASK_SUBAGENT_ONLY",
                "muse_task is available only inside a subagent",
            ));
        }
        let parsed: Args = serde_json::from_value(args.clone())
            .map_err(|e| error("INVALID_ARGUMENT", e.to_string()))?;
        if !(60..=7200).contains(&parsed.max_wait_secs) {
            return Err(error(
                "INVALID_ARGUMENT",
                "max_wait_secs must be between 60 and 7200",
            ));
        }
        if matches!(parsed.action, Action::Send)
            && parsed.prompt.as_deref().is_none_or(|p| p.trim().is_empty())
        {
            return Err(error(
                "INVALID_ARGUMENT",
                "send requires a non-empty prompt",
            ));
        }
        if parsed
            .filename
            .as_deref()
            .is_some_and(|s| s.trim().is_empty())
        {
            return Err(error("INVALID_ARGUMENT", "filename must not be empty"));
        }
        let handle = ctx.operation();
        let mut op = match handle.current().await.map_err(unknown)? {
            Some(op) => op,
            None => {
                let op = handle
                    .begin(
                        handle
                            .now_secs()
                            .saturating_add(parsed.max_wait_secs as i64),
                        "muse_task",
                    )
                    .await
                    .map_err(unknown)?;
                handle
                    .advance(OperationUpdate {
                        continuation_key: Some(format!(
                            "{MUSE_TASK_KEY_PREFIX}{}",
                            ctx.scope().session_id
                        )),
                        request_json: Some(
                            json!({
                                "owner_id": ctx.scope().owner_id,
                                "workspace_id": ctx.scope().workspace_id,
                                "session_id": ctx.scope().session_id,
                                "root_session_id": ctx.scope().root_session_id,
                                "delete_session_on_finish": parsed.delete_session_on_finish,
                                "pending_turn": args
                            })
                            .to_string(),
                        ),
                        ..Default::default()
                    })
                    .await
                    .map_err(unknown)?;
                handle.current().await.map_err(unknown)?.unwrap_or(op)
            }
        };
        if op.status == Status::Committed {
            let saved: Value =
                serde_json::from_str(op.result_json.as_deref().ok_or_else(|| {
                    error(
                        "TOOL_OUTCOME_UNKNOWN",
                        "Committed Muse operation has no result",
                    )
                })?)
                .map_err(|_| error("TOOL_OUTCOME_UNKNOWN", "Committed Muse result is corrupt"))?;
            return Ok(ToolOutcome {
                call_id: ctx.scope().tool_call_id.clone(),
                content: saved["content"]
                    .as_str()
                    .ok_or_else(|| {
                        error(
                            "TOOL_OUTCOME_UNKNOWN",
                            "Committed Muse result has no content",
                        )
                    })?
                    .into(),
                metadata: saved["metadata"].clone(),
            });
        }
        match op.status {
            Status::Cancelled => {
                return Err(error("CANCELLED", "Muse task invocation was cancelled"));
            }
            Status::Failed | Status::Expired => {
                if let Some(saved) = op
                    .result_json
                    .as_deref()
                    .and_then(|s| serde_json::from_str::<Value>(s).ok())
                    && let (Some(code), Some(message)) =
                        (saved["code"].as_str(), saved["message"].as_str())
                {
                    return Err(error(code, message));
                }
                return Err(if op.status == Status::Expired {
                    provider_error(AgentTaskError::Timeout { partial: None })
                } else {
                    error(
                        "MUSE_TASK_REJECTED",
                        "Muse task invocation previously failed",
                    )
                });
            }
            _ => {}
        }
        if ctx.cancel_reason().is_some() {
            return Err(cancelled(&ctx).await?);
        }
        if handle.now_secs() >= op.deadline_at {
            let failure = provider_error(AgentTaskError::Timeout { partial: None });
            handle
                .advance(OperationUpdate {
                    status: Some(Status::Expired),
                    result_json: Some(
                        json!({"code":failure.code,"message":failure.message}).to_string(),
                    ),
                    ..Default::default()
                })
                .await
                .map_err(unknown)?;
            return Err(failure);
        }
        let output_kind = parsed.output;
        let filename = parsed.filename.clone();
        let collecting = matches!(parsed.action, Action::Collect);
        let action = match parsed.action {
            Action::Send => AgentTurnAction::Send {
                prompt: parsed.prompt.unwrap_or_default(),
                output: output_kind,
                filename: filename.clone(),
            },
            Action::Collect => AgentTurnAction::Collect {
                output: output_kind,
                filename: filename.clone(),
            },
        };
        let key = format!("{MUSE_TASK_KEY_PREFIX}{}", ctx.scope().session_id);
        if op.continuation_key.as_deref() != Some(&key) {
            return Err(error(
                "TOOL_OUTCOME_UNKNOWN",
                "Muse continuation key does not match subagent scope",
            ));
        }
        let request = AgentTurnRequest {
            session_key: key,
            action,
            operation: op.clone(),
            wait_slice: Duration::from_secs(1700),
        };
        let resolved = match &self.backend {
            Backend::Catalog(catalog) => Some(
                catalog
                    .agent_task_provider()
                    .map_err(|e| error("MUSE_TASK_REJECTED", e))?,
            ),
            #[cfg(test)]
            Backend::Fake(_) => None,
        };
        let provider: &dyn AgentTaskProvider = match &self.backend {
            Backend::Catalog(_) => {
                resolved
                    .as_ref()
                    .and_then(|p| p.agent_task())
                    .ok_or_else(|| {
                        error("MUSE_TASK_REJECTED", "provider does not support agent_task")
                    })?
            }
            #[cfg(test)]
            Backend::Fake(p) => p.as_ref(),
        };
        let journal = Journal(&ctx);
        let progress = |message: &str| ctx.progress(message);
        let remaining =
            Duration::from_secs(op.deadline_at.saturating_sub(handle.now_secs()).max(0) as u64);
        let generated = tokio::select! {
            biased;
            _ = ctx.cancel_token().cancelled() => return Err(cancelled(&ctx).await?),
            _ = tokio::time::sleep(remaining) => {
                return Err(terminal_failure(&ctx, provider_error(AgentTaskError::Timeout { partial: None })).await);
            }
            output = provider.run_turn(&request, &journal, &progress) => match output {
                Ok(output) => output,
                Err(failure) => return Err(terminal_failure(&ctx, provider_error(failure)).await),
            },
        };
        // Downloading/committed are separate: a restart between publication and commit may leave
        // an unreferenced artifact, but no partially published result is returned to the model.
        op = handle
            .current()
            .await
            .map_err(unknown)?
            .ok_or_else(|| error("TOOL_OUTCOME_UNKNOWN", "Muse operation disappeared"))?;
        handle
            .advance(OperationUpdate {
                status: Some(Status::Downloading),
                ..Default::default()
            })
            .await
            .map_err(unknown)?;
        let turn = generated.turn;
        let delivered = collecting
            .then(|| self.delivered.find(&ctx, turn, output_kind, &filename))
            .flatten();
        let outcome = match delivered {
            Some(outcome) => outcome,
            None => match publish(self, &ctx, generated, output_kind, filename.clone()).await {
                Ok(outcome) => {
                    self.delivered
                        .remember(&ctx, turn, output_kind, filename, &outcome);
                    outcome
                }
                Err(failure) => return Err(terminal_failure(&ctx, failure).await),
            },
        };
        handle
            .advance(OperationUpdate {
                status: Some(Status::Committed),
                batch_index: Some(
                    serde_json::from_str::<Value>(&outcome.content)
                        .ok()
                        .and_then(|v| v["turn"].as_u64())
                        .unwrap_or(op.batch_index as u64) as u32,
                ),
                artifact_ids_json: Some(outcome.metadata["artifact_ids"].to_string()),
                result_json: Some(
                    json!({"content":outcome.content,"metadata":outcome.metadata}).to_string(),
                ),
                ..Default::default()
            })
            .await
            .map_err(unknown)?;
        Ok(outcome)
    }
}
