//! Owns Muse task session closure, independently of media cleanup.
use orchestrator_agent_rs::{RunStatus, RunSummary};

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum ClosureDecision {
    Keep,
    Close,
}

/// Mirror of orchestrator-agent 0.6.0 `subagent/outcome.rs::child_failure_is_retryable`.
/// The lockfile-version test requires an explicit review when that dependency changes.
fn child_failure_is_retryable(status: RunStatus, code: &str) -> bool {
    status != RunStatus::Cancelled
        && !matches!(
            code,
            "TOOL_OUTCOME_UNKNOWN"
                | "RECOVERY_BLOCKED"
                | "BUDGET_EXHAUSTED"
                | "CONTEXT_LIMIT"
                | "MODEL_NOT_CONFIGURED"
                | "PROVIDER_INVALID_REQUEST"
                | "PROVIDER_AUTH_FAILED"
                | "PROVIDER_CONFIG_CHANGED"
                | "CANCELLED"
                | "CHILD_CANCELLED"
        )
}

pub(crate) fn decide_closure(latest: Option<&RunSummary>, root_active: bool) -> ClosureDecision {
    if !root_active {
        return ClosureDecision::Close;
    }
    match latest {
        Some(run) if matches!(run.status, RunStatus::Completed | RunStatus::Cancelled) => {
            ClosureDecision::Close
        }
        Some(run)
            if run.status == RunStatus::Failed
                && !child_failure_is_retryable(
                    run.status,
                    run.error.as_ref().map_or("", |e| e.code.as_str()),
                ) =>
        {
            ClosureDecision::Close
        }
        _ => ClosureDecision::Keep,
    }
}

#[cfg(test)]
#[path = "closer_tests.rs"]
mod tests;

use crate::providers::llm::{MUSE_TASK_KEY_PREFIX, gateway::muse::pool};
use orchestrator_agent_rs::{
    CallerContext, Orchestrator, OrchestratorError, PageRequest, ToolOperationSummary,
};
use std::collections::{BTreeMap, HashSet};
use std::sync::Arc;
use std::time::Duration;

/// A journal is trusted host scope, never model arguments. Fail closed on incomplete scope.
#[derive(serde::Deserialize, PartialEq, Eq)]
struct JournalScope {
    owner_id: String,
    workspace_id: String,
    session_id: String,
    root_session_id: String,
}

pub(crate) struct SubagentMuseCloser;

impl SubagentMuseCloser {
    /// One startup/periodic pass. Operations stay durable until DELETE is acknowledged.
    pub(crate) async fn sweep(orchestrator: &Orchestrator) -> Result<(), OrchestratorError> {
        let summaries = orchestrator.list_tool_operations().await?;
        let mut groups: BTreeMap<&str, Vec<&ToolOperationSummary>> = BTreeMap::new();
        // The library ACK API marks by remote ID globally, without an account/key
        // filter. A remote ID shared by keys is inconsistent with session isolation;
        // preserve it for operator reconciliation rather than deleting live work.
        let mut remote_keys: BTreeMap<&str, HashSet<Option<&str>>> = BTreeMap::new();
        for summary in &summaries {
            let op = &summary.operation;
            if op.remote_deleted_at.is_none()
                && let Some(remote) = op.remote_session_id.as_deref()
            {
                remote_keys
                    .entry(remote)
                    .or_default()
                    .insert(op.continuation_key.as_deref());
            }
        }
        let conflicts: HashSet<&str> = remote_keys
            .into_iter()
            .filter_map(|(remote, keys)| (keys.len() > 1).then_some(remote))
            .collect();
        for summary in &summaries {
            let op = &summary.operation;
            if op.remote_deleted_at.is_none()
                && let Some(key) = op
                    .continuation_key
                    .as_deref()
                    .filter(|key| key.starts_with(MUSE_TASK_KEY_PREFIX))
            {
                groups.entry(key).or_default().push(summary);
            }
        }
        for (key, ops) in groups {
            if let Err(error) = Self::close_key(orchestrator, key, &ops, &conflicts).await {
                tracing::warn!(continuation_key = key, %error, "Muse task closure deferred");
            }
        }
        Ok(())
    }

    async fn close_key(
        orchestrator: &Orchestrator,
        key: &str,
        ops: &[&ToolOperationSummary],
        conflicts: &HashSet<&str>,
    ) -> Result<(), OrchestratorError> {
        let mut scope = None;
        for summary in ops {
            let parsed = summary
                .operation
                .request_json
                .as_deref()
                .and_then(|json| serde_json::from_str::<JournalScope>(json).ok());
            let Some(parsed) = parsed.filter(|scope| {
                !scope.owner_id.is_empty()
                    && !scope.workspace_id.is_empty()
                    && scope.session_id == key[MUSE_TASK_KEY_PREFIX.len()..]
                    && scope.root_session_id == summary.root_session_id
            }) else {
                tracing::warn!(
                    operation_id = summary.operation.id,
                    "Skipping Muse task key with invalid journal scope"
                );
                return Ok(());
            };
            if scope.as_ref().is_some_and(|scope| *scope != parsed) {
                tracing::warn!(
                    continuation_key = key,
                    "Skipping Muse task key with conflicting journal scopes"
                );
                return Ok(());
            }
            scope = Some(parsed);
        }
        let Some(scope) = scope else {
            return Ok(());
        };
        let caller = CallerContext::new(scope.owner_id, scope.workspace_id);
        let latest = latest_run(orchestrator, &caller, &scope.session_id).await?;
        if decide_closure(latest.as_ref(), ops.iter().any(|s| s.root_has_active_work))
            == ClosureDecision::Keep
        {
            return Ok(());
        }
        // In 0.6.0 an already-archived child can return PERMISSION_DENIED from
        // archive_session's admission rejection helper. Confirm public metadata
        // instead of interpreting that error as either success or permanent failure.
        if !orchestrator
            .get_session(&caller, &scope.session_id)
            .await?
            .archived
        {
            if let Err(error) = orchestrator
                .archive_session(&caller, &scope.session_id)
                .await
            {
                if !orchestrator
                    .get_session(&caller, &scope.session_id)
                    .await?
                    .archived
                {
                    return Err(error);
                }
            }
        }
        // Archive blocks admission, but an already-admitted attempt can still run/resume.
        let latest = latest_run(orchestrator, &caller, &scope.session_id).await?;
        if latest.as_ref().is_some_and(|run| {
            matches!(
                run.status,
                RunStatus::Running | RunStatus::WaitingRetry | RunStatus::WaitingUser
            )
        }) {
            return Ok(());
        }
        let mut released = HashSet::new();
        let mut acknowledged: BTreeMap<&str, bool> = BTreeMap::new();
        for summary in ops {
            let op = &summary.operation;
            let Some(remote) = op.remote_session_id.as_deref() else {
                continue;
            };
            if conflicts.contains(remote) {
                tracing::warn!(
                    continuation_key = key,
                    "Muse remote ID shared by continuation keys; cleanup deferred"
                );
                continue;
            }
            let Some(account) = op.account_key.as_deref() else {
                acknowledged.insert(remote, false);
                continue;
            };
            if !released.insert((account, remote)) {
                continue;
            }
            let ack = pool::release_recorded_job_session(op).await.is_some();
            acknowledged
                .entry(remote)
                .and_modify(|all| *all &= ack)
                .or_insert(ack);
        }
        for (remote, all_acknowledged) in acknowledged {
            // Distinct accounts may have recorded the same remote ID. Do not mark
            // every durable row until every pair has acknowledged deletion.
            if all_acknowledged {
                orchestrator.mark_remote_session_deleted(remote).await?;
            }
        }
        Ok(())
    }

    pub(crate) fn spawn(orchestrator: &Arc<Orchestrator>) {
        let weak = Arc::downgrade(orchestrator);
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(10));
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                // First tick is immediate: close finished children restored at startup.
                interval.tick().await;
                let Some(orchestrator) = weak.upgrade() else {
                    break;
                };
                if let Err(error) = Self::sweep(&orchestrator).await {
                    tracing::warn!(%error, "Muse task closure scan deferred");
                }
            }
        });
    }
}

async fn latest_run(
    orchestrator: &Orchestrator,
    caller: &CallerContext,
    session: &str,
) -> Result<Option<RunSummary>, OrchestratorError> {
    let mut request = PageRequest::first(100);
    let mut latest = None;
    loop {
        let page = orchestrator
            .list_runs(caller, session, None, request)
            .await?;
        if let Some(run) = page.items.into_iter().last() {
            latest = Some(run);
        }
        match page.next_cursor {
            Some(cursor) => request = PageRequest::after(cursor, 100),
            None => return Ok(latest),
        }
    }
}
