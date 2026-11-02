use super::{args::error, tool::MuseTaskTool};
use crate::providers::llm::{AgentOutputKind, AgentTurnOutput};
use orchestrator_agent_rs::{ArtifactInput, ToolContext, ToolError, ToolOutcome};
use serde_json::{Value, json};

pub(super) async fn publish(
    tool: &MuseTaskTool,
    ctx: &ToolContext,
    output: AgentTurnOutput,
    kind: AgentOutputKind,
    filename: Option<String>,
) -> Result<ToolOutcome, ToolError> {
    let mut files = Vec::new();
    for file in output.files {
        let name = safe_filename(&file.filename);
        if crate::providers::llm::supported_agent_task_file(&name) {
            files.push(put(tool, ctx, &name, &file.mime_type, file.bytes, "muse_file").await?);
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
                output.message.as_bytes().to_vec(),
                "message",
            )
            .await?,
        );
    }
    let mut message = output.message;
    if serde_json::to_string(&message).map_or(usize::MAX, |json| json.len()) > tool.output_cap / 2 {
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
        // Reserve space for JSON escaping, file metadata and the artifact ID.
        let mut end = (tool.output_cap / 12).min(message.len());
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
