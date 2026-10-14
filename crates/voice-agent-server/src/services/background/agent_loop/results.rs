//! Bounded result contracts, artifact evidence and terminal coverage.
use super::super::{JobError, JobView, model::AgentRunView};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::HashMap;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct RootResult {
    pub(super) summary: String,
    pub(super) findings: Vec<String>,
    pub(super) evidence_refs: Vec<String>,
    pub(super) unresolved: Vec<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ChildResult {
    pub(super) status: String,
    pub(super) summary: String,
    pub(super) findings: Vec<String>,
    pub(super) evidence_refs: Vec<String>,
    pub(super) unresolved: Vec<String>,
}

pub(super) fn bounded_result(value: &Value) -> Result<(), JobError> {
    if value.to_string().len() > 4096 || super::super::validation::forbidden(value) {
        Err(JobError::InvalidPlan)
    } else {
        Ok(())
    }
}
pub(super) fn verified_coverage(job: &JobView) -> Result<(&'static str, Value), JobError> {
    let mut results = HashMap::new();
    let mut failures = HashMap::new();
    for step in &job.steps {
        if step.agent_run_id.is_none() {
            continue;
        }
        match step.status.as_str() {
            "succeeded" => {
                if let Some(result) = step.result.clone() {
                    results.insert(step.step_key.clone(), result);
                }
            }
            _ => {
                failures.insert(
                    step.step_key.clone(),
                    step.error_code
                        .clone()
                        .unwrap_or_else(|| "step_failed".to_owned()),
                );
            }
        }
    }
    let (mut status, coverage) = super::super::coverage::terminal_result(job, &results, &failures)?;
    if job
        .agent_runs
        .iter()
        .any(|run| run.parent_agent_run_id.is_some() && run.status != "succeeded")
    {
        status = if job.request.allow_partial && !job.artifacts.is_empty() {
            "partial_success"
        } else {
            "failed"
        };
    }
    Ok((
        status,
        json!({
            "coverage_basis":coverage["coverage_basis"],
            "missing_sources":coverage["missing_sources"],
            "missing_capabilities":coverage["missing_capabilities"]
        }),
    ))
}
/// Only immutable artifacts owned by this Child Run can be cited as evidence.
pub(super) fn child_evidence_allowed(proposed: &[String], allowed: &[String]) -> bool {
    proposed.iter().all(|id| allowed.contains(id))
}

/// Match the Tool Call with a bounded observation and the exact server-owned
/// artifact IDs from this persisted Step. Never ask the LLM to invent IDs.
pub(super) fn plugin_tool_result(
    step_key: &str,
    status: &str,
    error: Option<&str>,
    result: Option<&str>,
    evidence_refs: &[String],
) -> Value {
    json!({
        "status":status,
        "error_code":error,
        "step_key":step_key,
        "evidence_refs":evidence_refs,
        "result_excerpt":result.map(|s| s.chars().take(3500).collect::<String>()),
    })
}
#[derive(Serialize)]
pub(super) struct ChildOutcome<'a> {
    #[serde(rename = "sessionID")]
    pub(super) session_id: &'a str,
    pub(super) status: &'a str,
    pub(super) output: String,
    pub(super) findings: Vec<String>,
    pub(super) artifact_refs: Vec<String>,
    pub(super) unresolved: Vec<String>,
    pub(super) error_code: Option<&'a str>,
}
impl<'a> ChildOutcome<'a> {
    pub(super) fn from_run(session_id: &'a str, child: &'a AgentRunView) -> Result<Self, JobError> {
        let result: Option<ChildResult> = child
            .result
            .clone()
            .map(serde_json::from_value)
            .transpose()
            .map_err(|_| JobError::DatabaseUnavailable)?;
        let result = result.unwrap_or(ChildResult {
            status: child.status.clone(),
            summary: String::new(),
            findings: Vec::new(),
            evidence_refs: Vec::new(),
            unresolved: Vec::new(),
        });
        Ok(Self {
            session_id,
            status: if child.status == "succeeded" {
                "completed"
            } else {
                &child.status
            },
            output: result.summary,
            findings: result.findings,
            artifact_refs: result.evidence_refs,
            unresolved: result.unresolved,
            error_code: child.error_code.as_deref(),
        })
    }
}
