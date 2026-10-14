//! Child Session preflight, creation, execution and matched Root feedback.
use super::{
    super::{
        JobEngine, JobError, JobView,
        model::{DelegatedTask, LlmRunContext},
    },
    child::finish_child,
    context::last_turn_id,
    results::ChildOutcome,
    tools::{SubagentInput, agent_instruction, validate_subagent},
};
use crate::providers::llm::{ChatMessage, ToolCall};
use serde_json::json;
use tokio_util::sync::CancellationToken;

/// Returns false when rejected delegation requires another Root turn.
pub(super) async fn delegate_children(
    engine: &JobEngine,
    job: &JobView,
    root_id: &str,
    root_session: &str,
    context: &LlmRunContext,
    calls: &[ToolCall],
    cancellation: &CancellationToken,
) -> Result<bool, JobError> {
    let round = context.attempt_no;
    let job_id = &job.id;

    // Validate all semantic arguments before creating any Child or Plugin Step.
    let mut tasks = Vec::with_capacity(calls.len());
    let mut inputs = Vec::with_capacity(calls.len());
    let frozen = job
        .plugin_choices
        .as_array()
        .ok_or(JobError::DatabaseUnavailable)?;
    let allowed: Vec<String> = frozen
        .iter()
        .map(|v| {
            format!(
                "{}.{}",
                v["plugin_id"].as_str().unwrap_or(""),
                v["operation"].as_str().unwrap_or("")
            )
        })
        .collect();
    if allowed.len() > 16 {
        return Err(JobError::CapacityUnavailable);
    }
    let mut invalid: Option<&'static str> = None;
    for (index, call) in calls.iter().enumerate() {
        let input: SubagentInput =
            serde_json::from_value(call.arguments.clone()).map_err(|_| JobError::InvalidPlan)?;
        if let Err(code) = validate_subagent(&input, job) {
            invalid = Some(code);
        }
        let goal: String = input.description.chars().take(256).collect();
        tasks.push(DelegatedTask {
            task_key: format!("task_{}_{}", round, index),
            role: input.agent.clone(),
            goal,
            allowed_operations: allowed.clone(),
        });
        inputs.push(input);
    }
    if invalid.is_none() {
        let mut sessions = std::collections::HashSet::new();
        for input in &inputs {
            if let Some(session) = input.session_id.as_deref() {
                if !sessions.insert(session) {
                    invalid = Some("session_busy");
                    break;
                }
                if let Err(error) = engine
                    .database
                    .preflight_child_resume(
                        job_id,
                        root_id,
                        session,
                        &input.agent,
                        &job.snapshot.worker,
                    )
                    .await
                {
                    invalid = Some(match error {
                        JobError::CapacityUnavailable => "session_busy",
                        JobError::InvalidArguments => "session_access_denied",
                        _ => return Err(error),
                    });
                    break;
                }
            }
        }
    }
    if let Some(code) = invalid {
        engine
            .database
            .record_agent_decision(
                context,
                false,
                json!({
                    "validation_code":code,"validation_path":"$.tool_calls[0].arguments",
                    "attempt_no":round
                }),
            )
            .await?;
        let mut messages = vec![ChatMessage::AssistantToolCalls {
            calls: calls.to_vec(),
        }];
        for call in calls {
            messages.push(ChatMessage::ToolResult {
                tool_call_id: call.id.clone(),
                content: json!({"status":"failed","error_code":code}).to_string(),
            });
        }
        engine
            .database
            .append_messages(job_id, root_session, &messages)
            .await?;
        return Ok(false);
    }

    engine
        .database
        .record_agent_decision(
            context,
            true,
            json!({
                "action":"delegate","attempt_no":round
            }),
        )
        .await?;
    let turn_id = last_turn_id(engine, job_id, root_id).await?;
    let sessions: Vec<_> = inputs
        .iter()
        .map(|input| crate::database::jobs::SessionTask {
            agent_key: &input.agent,
            resume_session_id: input.session_id.as_deref(),
            pin: &job.snapshot.worker,
            system: agent_instruction(&input.agent),
            prompt: &input.prompt,
        })
        .collect();
    let children = engine
        .database
        .create_session_children(job_id, root_id, &turn_id, &tasks, &sessions, calls)
        .await?;
    let mut attached = Vec::with_capacity(children.len());
    for child in &children {
        attached.push(
            engine
                .database
                .session_for_run(job_id, &child.agent_run_id)
                .await?,
        );
    }
    if children.len() == 1 {
        finish_child(engine, job, &children[0], cancellation.clone()).await?;
    } else {
        let (a, b) = tokio::join!(
            finish_child(engine, job, &children[0], cancellation.clone()),
            finish_child(engine, job, &children[1], cancellation.clone())
        );
        a?;
        b?;
    }
    let completed = engine
        .database
        .join_children(job_id, root_id, &turn_id)
        .await?;
    let mut feedback = Vec::with_capacity(calls.len());
    for ((call, child), session) in calls.iter().zip(completed.iter()).zip(attached.iter()) {
        feedback.push(ChatMessage::ToolResult {
            tool_call_id: call.id.clone(),
            content: serde_json::to_string(&ChildOutcome::from_run(session, child)?)
                .map_err(|_| JobError::InvalidPlan)?,
        });
    }
    engine
        .database
        .append_messages(job_id, root_session, &feedback)
        .await?;
    Ok(true)
}
