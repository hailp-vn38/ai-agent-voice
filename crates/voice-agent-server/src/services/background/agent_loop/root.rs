//! Foreground Root transcript and bounded orchestration rounds.
use super::{
    super::{JobEngine, JobError, JobView, model::LlmRunContext},
    completion::finish_root,
    context::run_context,
    delegation::delegate_children,
    tools::{ROOT_INSTRUCTION, root_call_batch_valid, root_call_validation_feedback, root_tools},
};
use crate::providers::llm::{BackgroundMode, BackgroundTurn, ChatMessage, LlmRequest};
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;

async fn root_turn(
    engine: &JobEngine,
    job: &JobView,
    root_id: &str,
    session_id: &str,
    attempt: u32,
    cancellation: &CancellationToken,
) -> Result<(BackgroundTurn, LlmRunContext), JobError> {
    let context = run_context(&job.id, root_id, "planner", attempt);
    let messages = engine.database.read_messages(&job.id, session_id).await?;
    let models = engine.models.as_ref().ok_or(JobError::RuntimeUnavailable)?;
    engine
        .database
        .reserve_model_call(&context, &job.snapshot.orchestrator)
        .await?;
    let deadline = job.deadline_at.min(
        super::super::engine::now()?
            .saturating_add(i64::from(job.snapshot.limits.step_timeout_seconds)),
    );
    let response = models
        .turn_traced(
            &context,
            &job.snapshot.orchestrator,
            LlmRequest {
                messages,
                tools: root_tools(),
            },
            BackgroundMode::ToolsParallel,
            deadline,
            cancellation.clone(),
        )
        .await;
    engine
        .database
        .record_llm_activity(
            &context,
            "llm_run_finished",
            json!({
                "status":if response.is_ok(){"completed"}else{"failed"},
                "error_code":response.as_ref().err().map(|e|e.code()),
                "duration_ms":context.started_at.elapsed().as_millis() as u64,
            }),
        )
        .await?;
    response.map(|turn| (turn.turn, context))
}
/// Native foreground Root loop. The result of every Child is a paired tool
/// observation, never a fabricated joined_children user prompt.
pub(in super::super) async fn run_root(
    engine: &JobEngine,
    job_id: &str,
    cancellation: CancellationToken,
) -> Result<(), JobError> {
    let root = engine.database.create_root_run(job_id).await?;
    let job = engine.get(job_id).await?;
    // Give Root the requirements and only the frozen, credential-free tool
    // catalog. Root plans prompts; the Child executor remains authoritative.
    let operations: Vec<Value> = job
        .plugin_choices
        .as_array()
        .ok_or(JobError::DatabaseUnavailable)?
        .iter()
        .map(|choice| {
            json!({
                "plugin_id":choice["plugin_id"],
                "operation":choice["operation"],
                "provides":choice["provides"],
                "sources":choice["sources"]
            })
        })
        .collect();
    let root_context = json!({
        "goal":job.request.goal,
        "output_format":job.request.output_format,
        "allow_partial":job.request.allow_partial,
        "required_sources":job.request.required_sources,
        "required_capabilities":job.request.required_capabilities,
        "frozen_operations":operations
    })
    .to_string();
    if root_context.len() > 16000 {
        return Err(JobError::CapacityUnavailable);
    }
    let root_session = engine
        .database
        .create_root_session(
            job_id,
            &root.agent_run_id,
            &job.snapshot.orchestrator,
            ROOT_INSTRUCTION,
            &root_context,
        )
        .await?;
    let mut seen = std::collections::HashSet::new();
    let max_rounds = job
        .snapshot
        .limits
        .max_child_runs
        .saturating_add(job.snapshot.limits.max_plan_repairs)
        .saturating_add(8);
    let mut repairs = 0u32;
    for round in 1..=max_rounds {
        if cancellation.is_cancelled() {
            return Err(JobError::Cancelled);
        }
        let (turn, context) = root_turn(
            engine,
            &job,
            &root.agent_run_id,
            &root_session,
            round,
            &cancellation,
        )
        .await?;
        let calls = match turn {
            BackgroundTurn::ToolCalls(calls) => calls,
            BackgroundTurn::Text(text) => {
                engine.database.record_agent_decision(&context,false,json!({
                    "validation_code":"expected_tool_call","validation_path":"$.tool_calls","attempt_no":round
                })).await?;
                engine.database.append_messages(job_id,&root_session,&[
                    ChatMessage::AssistantText{content:text.chars().take(2000).collect()},
                    ChatMessage::User{content:"Use subagent or finish_job tools; plain text cannot complete the Job.".into()}
                ]).await?;
                repairs += 1;
                if repairs > job.snapshot.limits.max_plan_repairs {
                    return Err(JobError::InvalidPlan);
                }
                continue;
            }
        };
        if !root_call_batch_valid(&calls, &seen) {
            let detailed = calls.first().map(root_call_validation_feedback);
            let validation_code = detailed
                .as_ref()
                .and_then(|value| value["error_code"].as_str())
                .unwrap_or("agent_tool_protocol_error");
            engine.database.record_agent_decision(&context,false,json!({
                "validation_code":validation_code,"validation_path":"$.tool_calls","attempt_no":round
            })).await?;
            let mut ids = std::collections::HashSet::new();
            let matchable = !calls.is_empty()
                && calls.iter().all(|call| {
                    !call.id.is_empty()
                        && call.id.len() <= 128
                        && !seen.contains(&call.id)
                        && ids.insert(call.id.clone())
                });
            if matchable {
                let mut messages = vec![ChatMessage::AssistantToolCalls {
                    calls: calls.clone(),
                }];
                for call in &calls {
                    messages.push(ChatMessage::ToolResult {
                        tool_call_id: call.id.clone(),
                        content: root_call_validation_feedback(call).to_string(),
                    });
                    seen.insert(call.id.clone());
                }
                engine
                    .database
                    .append_messages(job_id, &root_session, &messages)
                    .await?;
            } else {
                // Unusable IDs cannot form valid provider-native call/result pairs.
                engine
                    .database
                    .append_messages(
                        job_id,
                        &root_session,
                        &[ChatMessage::User {
                            content: "Invalid tool call batch; use valid, unique native tool IDs."
                                .into(),
                        }],
                    )
                    .await?;
            }
            repairs += 1;
            if repairs > job.snapshot.limits.max_plan_repairs {
                return Err(JobError::InvalidPlan);
            }
            continue;
        }
        for call in &calls {
            seen.insert(call.id.clone());
        }
        let finish = calls[0].name == "finish_job";
        if finish {
            if finish_root(engine, job_id, &root_session, &context, &calls).await? {
                return Ok(());
            }
            repairs += 1;
            if repairs > job.snapshot.limits.max_plan_repairs {
                return Err(JobError::InvalidPlan);
            }
            continue;
        }

        if !delegate_children(
            engine,
            &job,
            &root.agent_run_id,
            &root_session,
            &context,
            &calls,
            &cancellation,
        )
        .await?
        {
            repairs += 1;
            if repairs > job.snapshot.limits.max_plan_repairs {
                return Err(JobError::InvalidPlan);
            }
            continue;
        }
        repairs = 0;
    }
    Err(JobError::CapacityUnavailable)
}
