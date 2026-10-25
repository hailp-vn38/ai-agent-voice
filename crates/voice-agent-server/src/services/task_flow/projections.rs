//! Admin JSON projections from [`TaskView`] / [`TaskDetailView`] to the Admin Web UI contract.

use orchestrator_flow_rs::{TaskDetailView, TaskStatus, TaskView};
use serde_json::{Value, json};

fn map_task_status_to_admin_job_status(status: TaskStatus) -> &'static str {
    match status {
        TaskStatus::Queued => "queued",
        TaskStatus::Running => "running",
        TaskStatus::Completed => "succeeded",
        TaskStatus::Failed | TaskStatus::Interrupted => "failed",
        TaskStatus::Cancelled => "cancelled",
    }
}

fn map_agent_run_status(status: &str) -> &'static str {
    match status {
        "running" => "running",
        "completed" => "succeeded",
        "cancelled" => "cancelled",
        "interrupted" => "interrupted",
        "failed" | "timed_out" => "failed",
        _ => "queued",
    }
}

fn map_step_status(status: &str) -> &'static str {
    match status {
        "requested" => "pending",
        "running" => "running",
        "completed" => "succeeded",
        "cancelled" => "cancelled",
        "failed" | "interrupted" => "failed",
        _ => "pending",
    }
}

/// Projects a [`TaskView`] and optional [`TaskDetailView`] into the `AdminJob` JSON contract
/// consumed by Admin Web UI (`/api/admin/jobs` and `/api/admin/jobs/{id}`).
pub fn project_task_to_admin_job(task: &TaskView, detail: Option<&TaskDetailView>) -> Value {
    let status = map_task_status_to_admin_job_status(task.status);
    let created_at_sec = (task.created_at / 1000).max(1);
    let started_at_str = task.started_at.map(|ms| (ms / 1000).max(1).to_string());
    let finished_at_str = task.finished_at.map(|ms| (ms / 1000).max(1).to_string());

    let mut agent_runs = Vec::new();
    let mut steps = Vec::new();

    if let Some(detail) = detail {
        for (idx, session) in detail.all_agents().into_iter().enumerate() {
            let run_status = session
                .latest_run
                .as_ref()
                .map(|r| map_agent_run_status(&r.status))
                .unwrap_or(if status == "succeeded" {
                    "succeeded"
                } else {
                    "running"
                });
            let llm_rounds = session.latest_run.as_ref().map(|r| r.steps).unwrap_or(0);
            let input_tokens = session.latest_run.as_ref().map(|r| r.input_tokens);
            let output_tokens = session.latest_run.as_ref().map(|r| r.output_tokens);

            let goal_excerpt = session
                .recent_messages
                .iter()
                .find(|m| m.role == "user")
                .map(|m| m.text.chars().take(240).collect::<String>())
                .unwrap_or_else(|| {
                    if session.is_root {
                        task.prompt.chars().take(240).collect()
                    } else {
                        format!("Subagent `{}`", session.agent_id)
                    }
                });

            let last_assistant = session
                .recent_messages
                .iter()
                .rev()
                .find(|m| m.role == "assistant" && !m.text.trim().is_empty())
                .map(|m| json!({ "text": m.text }));

            agent_runs.push(json!({
                "agent_run_id": session.session_id,
                "parent_agent_run_id": session.parent_session_id,
                "fork_group_id": Value::Null,
                "role": session.agent_id,
                "goal_excerpt": goal_excerpt,
                "result": last_assistant,
                "status": run_status,
                "run_index": idx,
                "llm_rounds": llm_rounds,
                "tool_calls": session.invocations.len(),
                "input_tokens": input_tokens,
                "output_tokens": output_tokens,
                "started_at": started_at_str,
                "finished_at": finished_at_str,
            }));

            for (inv_idx, inv) in session.invocations.iter().enumerate() {
                steps.push(json!({
                    "step_key": format!("{}:{}:{}", session.agent_id, inv_idx + 1, inv.tool_name),
                    "agent_run_id": session.session_id,
                    "plugin_id": "orchestrator_flow",
                    "operation": inv.tool_name,
                    "status": map_step_status(&inv.status),
                    "error_code": inv.error_code,
                    "attempt_state": inv.status,
                    "instruction": format!("{} -> {}", session.agent_id, inv.tool_name),
                    "required": true,
                    "dependencies": [],
                    "input_refs": [],
                    "arguments": inv.arguments,
                    "output_schema": {},
                    "result": inv.result,
                }));
            }
        }
    }

    let budget_used = match detail.and_then(|d| d.execution_summary.as_ref()) {
        Some(summary) => json!({
            "steps": summary.total_steps,
            "input_tokens": summary.total_input_tokens,
            "output_tokens": summary.total_output_tokens,
            "total_tokens": summary.total_tokens,
            "tool_calls": summary.total_tool_calls,
            "succeeded_tool_calls": summary.succeeded_tool_calls,
            "failed_tool_calls": summary.failed_tool_calls,
            "sources_written": summary.sources_written,
            "wiki_pages_written": summary.wiki_pages_written,
            "last_lint_healthy": summary.last_lint_healthy,
        }),
        None => json!({}),
    };

    let result = task.output_preview.as_ref().map(|text| {
        json!({
            "text": text,
            "summary": text,
            "related_wiki_links": task.related_wiki_links,
        })
    });

    json!({
        "id": task.id,
        "title": task.title,
        "goal": task.prompt,
        "status": status,
        "task_status": task.status,
        "source": task.source,
        "flow_id": task.flow_id,
        "agent_id": task.agent_id,
        "session_id": task.session_id,
        "latest_turn_id": task.latest_turn_id,
        "active_subagent": task.active_subagent,
        "active_tool": task.active_tool,
        "progress_summary": task.progress_summary,
        "output_preview": task.output_preview,
        "related_wiki_links": task.related_wiki_links,
        "created_at": created_at_sec.to_string(),
        "started_at": started_at_str,
        "finished_at": finished_at_str,
        "requester_scope": format!("{}:{}:{}", task.source.as_str(), task.owner_id, task.workspace_id),
        "plan_revision": 1,
        "retry_of_job_id": Value::Null,
        "execution_mode": "orchestrator_agent",
        "result": result,
        "budget_used": budget_used,
        "agent_runs": agent_runs,
        "steps": steps,
        "artifacts": [],
        "followups": detail.map(|d| &d.followups),
        "execution_summary": detail.and_then(|d| d.execution_summary.as_ref()),
        "error_code": task.error_code,
        "error_message": task.error_message,
    })
}

fn push_session_events<'a>(
    session: &'a orchestrator_flow_rs::TaskAgentSessionView,
    mut child_iter: Option<&mut std::slice::Iter<'a, orchestrator_flow_rs::TaskAgentSessionView>>,
    created_at: &str,
    seq: &mut i64,
    events: &mut Vec<(i64, Value)>,
) {
    *seq += 1;
    events.push((
        *seq,
        json!({
            "sequence": *seq,
            "event_type": "agent_run_created",
            "created_at": created_at,
            "step_key": Value::Null,
            "payload": {
                "agent_run_id": session.session_id,
                "parent_agent_run_id": session.parent_session_id,
                "role": session.agent_id,
            }
        }),
    ));

    if let Some(ref run) = session.latest_run {
        *seq += 1;
        events.push((
            *seq,
            json!({
                "sequence": *seq,
                "event_type": "agent_run_started",
                "created_at": created_at,
                "step_key": Value::Null,
                "payload": {
                    "agent_run_id": session.session_id,
                    "run_id": run.run_id,
                    "turn_id": run.turn_id,
                    "attempt": run.attempt,
                }
            }),
        ));
    }

    for (inv_idx, inv) in session.invocations.iter().enumerate() {
        let step_key = format!("{}:{}:{}", session.agent_id, inv_idx + 1, inv.tool_name);
        *seq += 1;
        events.push((
            *seq,
            json!({
                "sequence": *seq,
                "event_type": "step_started",
                "created_at": created_at,
                "step_key": step_key,
                "payload": {
                    "agent_run_id": session.session_id,
                    "tool_name": inv.tool_name,
                    "arguments": inv.arguments,
                }
            }),
        ));

        // Khi Root Agent gọi `subagent`, lồng các sự kiện của Subagent vào đúng vị trí nhân quả
        // (sau `step_started` của lệnh `subagent` và trước `step_finished` của lệnh đó),
        // tránh dịch chuyển `sequence` khi Root Agent tiếp tục gọi tool khác sau khi Subagent xong.
        if inv.tool_name == "subagent"
            && let Some(ref mut children) = child_iter
            && let Some(child_session) = children.next()
        {
            push_session_events(child_session, None, created_at, seq, events);
        }

        if matches!(
            inv.status.as_str(),
            "completed" | "failed" | "cancelled" | "interrupted"
        ) {
            *seq += 1;
            events.push((
                *seq,
                json!({
                    "sequence": *seq,
                    "event_type": "step_finished",
                    "created_at": created_at,
                    "step_key": step_key,
                    "payload": {
                        "agent_run_id": session.session_id,
                        "tool_name": inv.tool_name,
                        "status": map_step_status(&inv.status),
                        "error_code": inv.error_code,
                        "result": inv.result,
                    }
                }),
            ));
        }
    }

    if let Some(ref run) = session.latest_run
        && run.status != "running"
    {
        *seq += 1;
        events.push((
            *seq,
            json!({
                "sequence": *seq,
                "event_type": "agent_run_finished",
                "created_at": created_at,
                "step_key": Value::Null,
                "payload": {
                    "agent_run_id": session.session_id,
                    "run_id": run.run_id,
                    "status": map_agent_run_status(&run.status),
                    "steps": run.steps,
                    "input_tokens": run.input_tokens,
                    "output_tokens": run.output_tokens,
                    "error_code": run.error_code,
                }
            }),
        ));
    }
}

/// Projects a [`TaskDetailView`] into sequential `JobEvent` items for `/api/admin/jobs/{id}/events`.
pub fn project_task_events(detail: &TaskDetailView, after: i64, limit: u32) -> Vec<Value> {
    let mut events = Vec::new();
    let mut seq: i64 = 0;
    let created_at = (detail.task.created_at / 1000).max(1).to_string();

    let mut child_iter = detail.subagents.iter();
    if let Some(ref root) = detail.root_agent {
        push_session_events(root, Some(&mut child_iter), &created_at, &mut seq, &mut events);
    }
    for remaining_child in child_iter {
        push_session_events(remaining_child, None, &created_at, &mut seq, &mut events);
    }

    if detail.task.status.is_terminal() {
        seq += 1;
        events.push((
            seq,
            json!({
                "sequence": seq,
                "event_type": "job_finished",
                "created_at": created_at,
                "step_key": Value::Null,
                "payload": {
                    "status": map_task_status_to_admin_job_status(detail.task.status),
                    "error_code": detail.task.error_code,
                }
            }),
        ));
    }

    events
        .into_iter()
        .filter(|(s, _)| *s > after)
        .take(limit as usize)
        .map(|(_, ev)| ev)
        .collect()
}
