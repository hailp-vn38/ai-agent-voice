//! Native tool definitions and Root call validation.
use super::{super::JobView, results::RootResult};
use crate::providers::llm::{ToolCall, ToolDefinition};
use serde_json::{json, Value};

pub(super) fn child_completion_tool() -> ToolDefinition {
    ToolDefinition {
        name: "submit_result".into(),
        description: "Finish this Sub-Agent. Set evidence_refs to artifact IDs supplied by prior plugin Tool Results, never to URLs or step keys. Use [] when no artifacts exist."
            .into(),
        parameters: json!({"type":"object","properties":{
            "status":{"type":"string","enum":["completed"]},
            "summary":{"type":"string"},
            "findings":{"type":"array","items":{"type":"string"}},
            "evidence_refs":{"type":"array","items":{"type":"string"}},
            "unresolved":{"type":"array","items":{"type":"string"}}
        },"required":["status","summary","findings","evidence_refs","unresolved"],
        "additionalProperties":false}),
    }
}
pub(super) const ROOT_INSTRUCTION: &str = "You are the Agent Job Root Orchestrator. Use the native subagent tool to delegate specific research/document/general tasks. Write each Child's complete task prompt yourself. Inspect subagent tool results before continuing. Resume an existing Child using its sessionID when more work is needed. For a URL article summary, delegate full article extraction (prefer core.web.extract mode=article, max_chars <=6000) and keep the source’s caveats, negations, unresolved decisions and uncertainty verbatim. Check verified_source_urls returned by subagent; when a required URL is not verified, resume that Child with a targeted prompt instead of claiming completion. Only call finish_job after enough evidence exists. finish_job accepts summary, findings and unresolved ONLY. Do not send evidence_refs, artifact IDs, step keys or source URLs in finish_job arguments: Rust attaches authorized Job artifact IDs and verified source URLs from the persisted ledger. Do not invent artifact IDs. Do not reply with prose instead of calling a tool. Never set background=true; only the pinned worker model may be selected.";

#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct SubagentInput {
    pub(super) agent: String,
    pub(super) description: String,
    pub(super) prompt: String,
    #[serde(default)]
    pub(super) model: Option<String>,
    #[serde(default, rename = "sessionID")]
    pub(super) session_id: Option<String>,
    #[serde(default)]
    pub(super) background: bool,
}

pub(super) fn root_tools() -> Vec<ToolDefinition> {
    vec![
        ToolDefinition {
            name: "subagent".into(),
            description: "Delegate a complete, concrete task prompt to a specialized Child Session. sessionID resumes a previous Child; background must be false.".into(),
            parameters: json!({"type":"object","properties":{
                "agent":{"type":"string","enum":["research","document","general"]},
                "description":{"type":"string"},
                "prompt":{"type":"string"},
                "model":{"type":["string","null"]},
                "sessionID":{"type":["string","null"]},
                "background":{"type":"boolean"}
            },"required":["agent","description","prompt","model","sessionID","background"],
            "additionalProperties":false}),
        },
        ToolDefinition {
            name: "finish_job".into(),
            description: "Finish the Job with summary, findings, unresolved ONLY. Do not send evidence_refs or URLs as artifact IDs. Rust derives artifact IDs and source verification from persisted Job records.".into(),
            parameters: json!({"type":"object","properties":{
                "summary":{"type":"string"},
                "findings":{"type":"array","items":{"type":"string"}},
                "unresolved":{"type":"array","items":{"type":"string"}}
            },"required":["summary","findings","unresolved"],
            "additionalProperties":false}),
        },
    ]
}
/// Explain the exact violated finish_job contract in a *matched* Tool Result.
/// Never turn an untrusted URL/step_key into a valid artifact reference.
pub(super) fn root_call_validation_feedback(call: &ToolCall) -> Value {
    if call.name == "finish_job" && call.arguments.get("evidence_refs").is_some() {
        json!({
            "status": "failed",
            "error_code": "finish_job_server_owned_evidence",
            "validation_path": "$.arguments.evidence_refs",
            "guidance": "Remove evidence_refs from finish_job arguments. Send only summary, findings and unresolved. Rust derives Job-owned artifact IDs and verified URLs; source URLs are NOT artifact IDs."
        })
    } else {
        json!({
            "status": "failed",
            "error_code": "agent_tool_protocol_error",
            "guidance": "Match the provided native tool schema exactly; only permitted tools and fields may be used."
        })
    }
}

pub(super) fn agent_instruction(role: &str) -> &'static str {
    match role {
        "research" => {
            "You are a Research Sub-Agent. Investigate the delegated task, verify evidence, use only approved tools, and report concise findings. For a news URL summary, prefer core.web.extract with mode=article and max_chars=6000 on the first call. If truncated=true, continue next_offset using the same mode and URL. Preserve explicit negations, tentative statements, and claims not yet officially decided. Never treat navigation fragments as a complete article. You cannot delegate."
        }
        "document" => {
            "You are a Document Sub-Agent. Analyze or prepare content as instructed, use only approved tools, and report the result and unresolved gaps. You cannot delegate."
        }
        _ => {
            "You are a General Sub-Agent. Complete the delegated task using only approved tools, report verified results and unresolved gaps. You cannot delegate."
        }
    }
}

pub(super) fn validate_subagent(input: &SubagentInput, job: &JobView) -> Result<(), &'static str> {
    if !matches!(input.agent.as_str(), "research" | "document" | "general")
        || input.description.trim().is_empty()
        || input.description.len() > 256
        || input.prompt.trim().is_empty()
        || input.prompt.len() > 16000
    {
        return Err("invalid_subagent_prompt");
    }
    if input.background {
        return Err("background_subagent_not_enabled");
    }
    if input
        .model
        .as_deref()
        .is_some_and(|model| model != job.snapshot.worker.reference)
    {
        return Err("model_not_allowed");
    }
    if input
        .session_id
        .as_deref()
        .is_some_and(|id| id.is_empty() || id.len() > 128)
    {
        return Err("invalid_session_id");
    }
    Ok(())
}
pub(super) fn root_call_batch_valid(
    calls: &[crate::providers::llm::ToolCall],
    seen: &std::collections::HashSet<String>,
) -> bool {
    if calls.is_empty() || calls.len() > 2 {
        return false;
    }
    let tools = root_tools();
    let mut local = std::collections::HashSet::new();
    let mut finished = false;
    for call in calls {
        if call.id.is_empty()
            || call.id.len() > 128
            || !local.insert(call.id.as_str())
            || seen.contains(&call.id)
            || !call.arguments.is_object()
            || call.arguments.to_string().len() > 16384
        {
            return false;
        }
        let Some(tool) = tools.iter().find(|tool| tool.name == call.name) else {
            return false;
        };
        if jsonschema::validator_for(&tool.parameters)
            .map_or(true, |schema| !schema.is_valid(&call.arguments))
        {
            return false;
        }
        match call.name.as_str() {
            "subagent" => {
                if serde_json::from_value::<SubagentInput>(call.arguments.clone()).is_err() {
                    return false;
                }
            }
            "finish_job" => {
                if serde_json::from_value::<RootResult>(call.arguments.clone()).is_err() {
                    return false;
                }
                finished = true;
            }
            _ => return false,
        }
    }
    !finished || calls.len() == 1
}
