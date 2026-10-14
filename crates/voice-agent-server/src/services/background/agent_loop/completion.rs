//! Root completion admission, coverage and durable Job finalization.
use super::{
    super::{JobEngine, JobError, model::LlmRunContext},
    results::{RootResult, bounded_result, grounded_single_child_summary, verified_coverage},
};
use crate::providers::llm::{ChatMessage, ToolCall};
use serde_json::json;

/// Returns false when a rejected completion requires another Root turn.
pub(super) async fn finish_root(
    engine: &JobEngine,
    job_id: &str,
    root_session: &str,
    context: &LlmRunContext,
    calls: &[ToolCall],
) -> Result<bool, JobError> {
    let round = context.attempt_no;
    let result: RootResult =
        serde_json::from_value(calls[0].arguments.clone()).map_err(|_| JobError::InvalidPlan)?;
    let bounded = serde_json::to_value(&result).map_err(|_| JobError::InvalidPlan)?;
    let current = engine.get(job_id).await?;
    let mut failure = if bounded_result(&bounded).is_err() || result.summary.trim().is_empty() {
        Some("invalid_finish_result")
    } else if current.agent_runs.iter().any(|r| {
        r.parent_agent_run_id.is_some() && matches!(r.status.as_str(), "queued" | "running")
    }) {
        Some("children_still_running")
    } else if !current
        .agent_runs
        .iter()
        .any(|r| r.parent_agent_run_id.is_some())
    {
        Some("missing_child_outcome")
    } else if result
        .evidence_refs
        .iter()
        .any(|id| !current.artifacts.iter().any(|a| &a.id == id))
    {
        Some("invalid_evidence_refs")
    } else {
        None
    };
    let (mut status, coverage) = verified_coverage(&current)?;
    // A Job with an explicit single article URL cannot succeed on empty
    // required_sources merely because some partial HTML fragments were fetched.
    if failure.is_none()
        && coverage["source_complete"] == false {
        failure = Some("source_not_fully_verified");
    }
    if failure.is_none()
        && status == "failed"
        && (!current.request.required_sources.is_empty()
            || !current.request.required_capabilities.is_empty())
    {
        failure = Some("requirements_unsatisfied");
    }
    if let Some(error_code) = failure {
        engine
            .database
            .record_agent_decision(
                context,
                false,
                json!({
                    "validation_code":error_code,"validation_path":"$.tool_calls[0].arguments",
                    "attempt_no":round
                }),
            )
            .await?;
        engine
            .database
            .append_messages(
                job_id,
                root_session,
                &[
                    ChatMessage::AssistantToolCalls {
                        calls: calls.to_vec(),
                    },
                    ChatMessage::ToolResult {
                        tool_call_id: calls[0].id.clone(),
                        content: json!({"status":"failed","error_code":error_code,
                "coverage":coverage})
                        .to_string(),
                    },
                ],
            )
            .await?;
        return Ok(false);
    }
    if !result.unresolved.is_empty() && status == "succeeded" {
        status = if current.request.allow_partial && !current.artifacts.is_empty() {
            "partial_success"
        } else {
            "failed"
        };
    }
    let preserved = grounded_single_child_summary(&current, &coverage);
    let summary_source = if preserved.is_some() { "verified_child" } else { "root" };
    let final_summary = preserved.unwrap_or(result.summary);
    let value = json!({"summary":final_summary,"summary_source":summary_source,
    "findings":result.findings,"evidence_refs":result.evidence_refs,
    "unresolved":result.unresolved,"coverage":coverage});
    engine
        .database
        .record_agent_decision(
            context,
            true,
            json!({
                "action":"finish","attempt_no":round
            }),
        )
        .await?;
    engine
        .database
        .append_messages(
            job_id,
            root_session,
            &[
                ChatMessage::AssistantToolCalls {
                    calls: calls.to_vec(),
                },
                ChatMessage::ToolResult {
                    tool_call_id: calls[0].id.clone(),
                    content: json!({"status":"accepted_for_validation"}).to_string(),
                },
            ],
        )
        .await?;
    engine
        .database
        .finish_job(
            &engine.config,
            job_id,
            status,
            (status != "succeeded").then_some("requirements_unsatisfied"),
            Some(&value),
        )
        .await?;
    Ok(true)
}
