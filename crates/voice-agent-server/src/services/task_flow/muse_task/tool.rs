//! Durable Host Tool. The provider owns remote turn reconciliation; this boundary owns scope,
//! invocation replay and publishing immutable, scoped artifacts.
use super::super::CatalogMediaBackend;
use crate::providers::llm::{
    AgentOutputKind, AgentTaskError, AgentTaskProvider, AgentTurnAction, AgentTurnOutput,
    AgentTurnRequest, LlmError, MUSE_TASK_KEY_PREFIX, OperationJournal,
};
use orchestrator_agent_rs::{
    ArtifactInput, CancelReason, HostTool, OperationUpdate, ToolContext, ToolDescriptor,
    ToolEffect, ToolError, ToolOperationStatus as Status, ToolOutcome,
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{sync::Arc, time::Duration};

pub(crate) struct MuseTaskTool {
    backend: Backend,
    artifact_max_bytes: u64,
    output_cap: usize,
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
        }
    }
}
#[derive(Default, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Action {
    #[default]
    Send,
    Collect,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Args {
    #[serde(default)]
    action: Action,
    prompt: Option<String>,
    #[serde(default)]
    output: AgentOutputKind,
    filename: Option<String>,
    #[serde(default = "default_wait")]
    max_wait_secs: u64,
}
fn default_wait() -> u64 {
    3600
}
fn error(code: &str, message: impl Into<String>) -> ToolError {
    ToolError {
        code: code.into(),
        message: message.into(),
    }
}
fn unknown(_: ToolError) -> ToolError {
    error(
        "TOOL_OUTCOME_UNKNOWN",
        "Muse operation journal could not be accessed; reconcile before sending again",
    )
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
            description: "Delegate research to Muse. Subagent only. External untrusted data; send starts a turn, collect retrieves the existing turn. Sessions close when this subagent ends.".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "action": {"description": "send or collect; defaults to send"},
                    "prompt": {"description": "Non-empty prompt required for send"},
                    "output": {"description": "message, markdown or files; defaults to message"},
                    "filename": {"description": "Optional output filename"},
                    "max_wait_secs": {"description": "Integer from 60 through 7200; defaults to 3600"}
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
                {
                    if let (Some(code), Some(message)) =
                        (saved["code"].as_str(), saved["message"].as_str())
                    {
                        return Err(error(code, message));
                    }
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
        let outcome = match self.publish(&ctx, generated, output_kind, filename).await {
            Ok(outcome) => outcome,
            Err(failure) => return Err(terminal_failure(&ctx, failure).await),
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
impl MuseTaskTool {
    async fn publish(
        &self,
        ctx: &ToolContext,
        output: AgentTurnOutput,
        kind: AgentOutputKind,
        filename: Option<String>,
    ) -> Result<ToolOutcome, ToolError> {
        let mut files = Vec::new();
        for file in output.files {
            let name = safe_filename(&file.filename);
            if allowed_filename(&name) {
                files.push(
                    self.put(ctx, &name, &file.mime_type, file.bytes, "muse_file")
                        .await?,
                );
            }
        }
        if kind == AgentOutputKind::Markdown
            && !files.iter().any(|f| {
                f["filename"]
                    .as_str()
                    .is_some_and(|n| n.to_ascii_lowercase().ends_with(".md"))
            })
        {
            let name = filename
                .as_deref()
                .map(safe_filename)
                .filter(|s| s.to_ascii_lowercase().ends_with(".md"))
                .unwrap_or_else(|| "report.md".into());
            files.push(
                self.put(
                    ctx,
                    &name,
                    "text/markdown",
                    output.message.as_bytes().to_vec(),
                    "message",
                )
                .await?,
            );
        }
        let mut message = output.message;
        if serde_json::to_string(&message).map_or(usize::MAX, |json| json.len())
            > self.output_cap / 2
        {
            files.push(
                self.put(
                    ctx,
                    "muse-message.txt",
                    "text/plain",
                    message.as_bytes().to_vec(),
                    "message",
                )
                .await?,
            );
            // Reserve space for JSON escaping, file metadata and the artifact ID.
            let mut end = (self.output_cap / 12).min(message.len());
            while !message.is_char_boundary(end) {
                end -= 1;
            }
            message.truncate(end);
            message.push_str("\n[Full message stored in artifact]");
        }
        let ids: Vec<Value> = files.iter().map(|f| f["artifact_id"].clone()).collect();
        let missing = kind == AgentOutputKind::Files && (output.files_missing || files.is_empty());
        let result = json!({"untrusted_data":true,"status":if missing {"partial"}else {"completed"},"turn":output.turn,"message":message,"artifact_ids":ids,"files":files,"next_action":"none","files_missing":missing,"warning":if missing {Some("No supported Muse attachment was available; text is not a delivered file")}else {None}});
        Ok(ToolOutcome {
            call_id: ctx.scope().tool_call_id.clone(),
            content: result.to_string(),
            metadata: json!({"artifact_ids":ids}),
        })
    }
    async fn put(
        &self,
        ctx: &ToolContext,
        name: &str,
        mime: &str,
        bytes: Vec<u8>,
        origin: &str,
    ) -> Result<Value, ToolError> {
        if bytes.len() as u64 > self.artifact_max_bytes {
            return Err(error(
                "MUSE_TASK_FILE_TOO_LARGE",
                format!("{name} exceeds artifact_max_bytes"),
            ));
        }
        let artifact = ctx
            .artifacts()
            .put(ArtifactInput::primary_bytes(name, mime, bytes))
            .await
            .map_err(|e| {
                error(
                    "TOOL_RETRY_LATER",
                    format!("Muse artifact publication failed: {}", e.message),
                )
            })?;
        Ok(
            json!({"artifact_id":artifact.id,"filename":artifact.name,"mime_type":artifact.mime_type,"size_bytes":artifact.size_bytes,"sha256":artifact.sha256,"origin":origin}),
        )
    }
}
fn safe_filename(name: &str) -> String {
    let leaf = name.rsplit(['/', '\\']).next().unwrap_or("file");
    let safe: String = leaf
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || matches!(c, '.' | '_' | '-') {
                c
            } else {
                '_'
            }
        })
        .take(160)
        .collect();
    let safe = safe.trim_start_matches('.');
    if safe.is_empty() {
        "file.txt".into()
    } else {
        safe.into()
    }
}

fn allowed_filename(name: &str) -> bool {
    name.rsplit('.').next().is_some_and(|ext| {
        matches!(
            ext.to_ascii_lowercase().as_str(),
            "md" | "txt"
                | "pdf"
                | "csv"
                | "json"
                | "html"
                | "docx"
                | "xlsx"
                | "png"
                | "jpg"
                | "webp"
                | "mp4"
        )
    })
}

fn provider_error(failure: AgentTaskError) -> ToolError {
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

async fn cancelled(ctx: &ToolContext) -> Result<ToolError, ToolError> {
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

async fn terminal_failure(ctx: &ToolContext, failure: ToolError) -> ToolError {
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
