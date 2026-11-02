use super::{args::error, tool::MuseTaskTool};
use crate::providers::llm::{AgentOutputKind, AgentTurnOutput};
use orchestrator_agent_rs::{ArtifactInput, ToolContext, ToolError, ToolOutcome};
use serde_json::{Value, json};
use std::{collections::VecDeque, sync::Mutex};

const DELIVERED_TURNS_KEPT: usize = 32;

/// The latest published turn of recent sessions, so collecting a turn that was already delivered
/// does not publish its artifacts again. In-memory: after a restart a collect publishes once more.
#[derive(Default)]
pub(super) struct DeliveredTurns(Mutex<VecDeque<DeliveredTurn>>);

struct DeliveredTurn {
    session_id: String,
    turn: u32,
    kind: AgentOutputKind,
    filename: Option<String>,
    content: String,
    metadata: Value,
}

impl DeliveredTurns {
    pub(super) fn find(
        &self,
        ctx: &ToolContext,
        turn: u32,
        kind: AgentOutputKind,
        filename: &Option<String>,
    ) -> Option<ToolOutcome> {
        let turns = self.0.lock().unwrap_or_else(|p| p.into_inner());
        let found = turns.iter().find(|d| {
            d.session_id == ctx.scope().session_id
                && d.turn == turn
                && d.kind == kind
                && d.filename == *filename
        })?;
        Some(ToolOutcome {
            call_id: ctx.scope().tool_call_id.clone(),
            content: found.content.clone(),
            metadata: found.metadata.clone(),
        })
    }

    pub(super) fn remember(
        &self,
        ctx: &ToolContext,
        turn: u32,
        kind: AgentOutputKind,
        filename: Option<String>,
        outcome: &ToolOutcome,
    ) {
        let mut turns = self.0.lock().unwrap_or_else(|p| p.into_inner());
        turns.retain(|d| d.session_id != ctx.scope().session_id);
        if turns.len() >= DELIVERED_TURNS_KEPT {
            turns.pop_front();
        }
        turns.push_back(DeliveredTurn {
            session_id: ctx.scope().session_id.clone(),
            turn,
            kind,
            filename,
            content: outcome.content.clone(),
            metadata: outcome.metadata.clone(),
        });
    }
}

pub(super) async fn publish(
    tool: &MuseTaskTool,
    ctx: &ToolContext,
    output: AgentTurnOutput,
    kind: AgentOutputKind,
    filename: Option<String>,
) -> Result<ToolOutcome, ToolError> {
    let mut files = Vec::new();
    let mut message = output.message;
    for file in output.files {
        let name = safe_filename(&file.filename);
        if crate::providers::llm::supported_agent_task_file(&name) {
            let published = put(tool, ctx, &name, &file.mime_type, file.bytes, "muse_file").await?;
            // A sandbox link only resolves inside the Muse VM; point it at the delivered copy.
            if let Some(id) = published["artifact_id"].as_str() {
                message = message.replace(
                    &format!("sandbox://{}", file.source_path),
                    &format!("artifact:{id}"),
                );
            }
            files.push(published);
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
            put(
                tool,
                ctx,
                &name,
                "text/markdown",
                message.as_bytes().to_vec(),
                "message",
            )
            .await?,
        );
    }
    let mut message_artifact_id = None;
    if serde_json::to_string(&message).map_or(usize::MAX, |json| json.len()) > tool.output_cap / 2 {
        if !files.iter().any(|f| f["origin"] == "message") {
            files.push(
                put(
                    tool,
                    ctx,
                    "muse-message.txt",
                    "text/plain",
                    message.as_bytes().to_vec(),
                    "message",
                )
                .await?,
            );
        }
        let stored = files
            .iter()
            .find(|f| f["origin"] == "message")
            .map(|f| f["artifact_id"].clone());
        // Reserve space for JSON escaping, file metadata and the artifact ID.
        let mut end = (tool.output_cap / 12).min(message.len());
        while !message.is_char_boundary(end) {
            end -= 1;
        }
        message.truncate(end);
        message.push_str(&format!(
            "\n[Preview only. Muse finished the complete message: read it with artifact_read(artifact_id={}). Do not ask Muse to rewrite or shorten it.]",
            stored.as_ref().and_then(Value::as_str).unwrap_or_default()
        ));
        message_artifact_id = stored;
    }
    let ids: Vec<Value> = files.iter().map(|f| f["artifact_id"].clone()).collect();
    let missing = output.files_missing || (kind == AgentOutputKind::Files && files.is_empty());
    let result = json!({"untrusted_data":true,"status":if missing {"partial"}else {"completed"},"turn":output.turn,"completed_at":chrono::Local::now().to_rfc3339(),"message":message,"message_truncated":message_artifact_id.is_some(),"message_artifact_id":message_artifact_id,"artifact_ids":ids,"files":files,"next_action":"none","files_missing":missing,"warning":if missing {Some("A Muse file was not available; text is not a delivered file")}else {None}});
    Ok(ToolOutcome {
        call_id: ctx.scope().tool_call_id.clone(),
        content: result.to_string(),
        metadata: json!({"artifact_ids":ids}),
    })
}
async fn put(
    tool: &MuseTaskTool,
    ctx: &ToolContext,
    name: &str,
    mime: &str,
    bytes: Vec<u8>,
    origin: &str,
) -> Result<Value, ToolError> {
    if bytes.len() as u64 > tool.artifact_max_bytes {
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
