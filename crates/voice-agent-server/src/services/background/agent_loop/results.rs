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
    pub(super) unresolved: Vec<String>,
}
/// Artifact IDs belong to the Job ledger, never to a Root model proposal.
/// This list is built only from server-issued JobArtifact records, so a URL
/// extracted via core.web.extract does not become a fabricated artifact ID.
pub(super) fn server_evidence_refs(job: &JobView) -> Vec<String> {
    let mut ids: Vec<String> = job
        .artifacts
        .iter()
        .map(|artifact| artifact.id.clone())
        .collect();
    ids.sort();
    ids.dedup();
    ids
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
#[cfg(test)]
pub(super) fn verified_coverage(job: &JobView) -> Result<(&'static str, Value), JobError> {
    let verified = super::super::coverage::completed_web_sources(job, None);
    verified_coverage_with_sources(job, verified)
}

/// Accept only URL observations previously grouped by authoritative Session IDs.
pub(super) fn verified_coverage_with_sources(
    job: &JobView,
    verified: Vec<String>,
) -> Result<(&'static str, Value), JobError> {
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
    let requested = super::super::coverage::requested_summary_url(&job.request);
    let verified_goal = requested.as_ref().is_some_and(|url| verified.contains(url));
    let source_complete = requested.is_none() || verified_goal;
    if requested.is_some() && !verified_goal && status == "succeeded" {
        status = "failed";
    }
    let coverage_basis = if requested.is_some() {
        if verified_goal {
            "goal_url_verified"
        } else {
            "goal_url_unverified"
        }
    } else {
        coverage["coverage_basis"]
            .as_str()
            .unwrap_or("goal_only_unverified")
    };
    Ok((
        status,
        json!({
            "coverage_basis":coverage_basis,
            "missing_sources":coverage["missing_sources"],
            "missing_capabilities":coverage["missing_capabilities"],
            "requested_url":requested,
            "verified_source_urls":verified,
            "source_complete":source_complete
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
    // Return useful ARTICLE text instead of truncating a JSON object in the
    // middle of its first 3500 characters. Keep the observed text below the
    // durable Session message limit; explicitly tell the Child if text was cut.
    if let Some(value) = result.and_then(|raw| serde_json::from_str::<Value>(raw).ok())
        && value["url"].is_string()
        && value["text"].is_string()
        && value["status"].is_number()
    {
        let full_text = value["text"].as_str().unwrap_or("");
        let observed: String = full_text
            .chars()
            .take(super::super::coverage::MAX_VISIBLE_WEB_CHARS)
            .collect();
        return json!({
            "status":status,"error_code":error,"step_key":step_key,
            "evidence_refs":evidence_refs,
            "source_url":value["url"],"title":value["title"],
            "http_status":value["status"],
            "result_excerpt":observed,
            "truncated":value["truncated"],
            "next_offset":value["next_offset"],
            "body_limit_reached":value["body_limit_reached"],
            "observation_truncated":full_text.chars().count()>super::super::coverage::MAX_VISIBLE_WEB_CHARS,
            "guidance":"For an article summary, prefer mode=article and chunks <=6000 characters. If truncated, use next_offset; if observation_truncated, re-read with smaller max_chars."
        });
    }
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
    /// URLs verified from full persisted web.extract results, never LLM claims.
    pub(super) verified_source_urls: Vec<String>,
    pub(super) unresolved: Vec<String>,
    pub(super) error_code: Option<&'a str>,
}
impl<'a> ChildOutcome<'a> {
    pub(super) fn from_run(
        session_id: &'a str,
        child: &'a AgentRunView,
        verified_source_urls: Vec<String>,
    ) -> Result<Self, JobError> {
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
            verified_source_urls,
            unresolved: result.unresolved,
            error_code: child.error_code.as_deref(),
        })
    }
}

/// For a single-URL summary, the one Child's complete source-grounded answer
/// is the user-facing summary. Avoid a second LLM rewrite that drops crucial
/// negations (e.g. "no official decision to postpone") or adds new claims.
/// Root still authorizes finish_job; Rust owns this deterministic presentation.
pub(super) fn grounded_single_child_summary(job: &JobView, coverage: &Value) -> Option<String> {
    let url = super::super::coverage::requested_summary_url(&job.request)?;
    if coverage["source_complete"] != true
        || !coverage["verified_source_urls"]
            .as_array()?
            .iter()
            .any(|s| s.as_str() == Some(url.as_str()))
    {
        return None;
    }
    let children: Vec<_> = job
        .agent_runs
        .iter()
        .filter(|run| run.parent_agent_run_id.is_some())
        .collect();
    if children.len() != 1 || children[0].status != "succeeded" {
        return None;
    }
    let result: ChildResult = serde_json::from_value(children[0].result.clone()?).ok()?;
    let summary = result.summary.trim();
    if summary.is_empty() || summary.chars().count() > 2000 {
        None
    } else {
        Some(summary.to_owned())
    }
}
