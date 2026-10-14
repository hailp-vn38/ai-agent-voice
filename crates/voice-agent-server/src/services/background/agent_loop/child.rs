//! Bounded Child execution, plugin calls and durable completion.
use super::{
    super::{JobEngine, JobError, JobView, model::AgentRunView},
    context::{last_turn_id, run_context},
    results::{ChildResult, bounded_result, child_evidence_allowed, plugin_tool_result},
    tools::child_completion_tool,
};
use crate::providers::llm::{BackgroundMode, BackgroundTurn, ChatMessage, LlmRequest};
use serde_json::json;
use tokio_util::sync::CancellationToken;

async fn run_child(
    engine: &JobEngine,
    job: &JobView,
    child: &AgentRunView,
    cancellation: CancellationToken,
) -> Result<ChildResult, JobError> {
    engine
        .database
        .start_child_run(&job.id, &child.agent_run_id)
        .await?;
    let choices = job
        .plugin_choices
        .as_array()
        .ok_or(JobError::DatabaseUnavailable)?;
    let mut pinned = Vec::new();
    for allowed in &child.allowed_operations {
        let raw = choices
            .iter()
            .find(|c| {
                format!(
                    "{}.{}",
                    c["plugin_id"].as_str().unwrap_or(""),
                    c["operation"].as_str().unwrap_or("")
                ) == *allowed
            })
            .ok_or(JobError::CapabilityUnavailable)?;
        let choice = super::super::plugins::decode_choice(raw).map_err(JobError::from_plugin)?;
        let contract = engine
            .plugins
            .operation_contract(&choice)
            .await
            .map_err(JobError::from_plugin)?;
        pinned.push((choice, contract));
    }
    let (mut tools, bindings) = super::super::plugins::compile_child_tools(&pinned)?;
    tools.push(child_completion_tool());
    if serde_json::to_string(
        &tools
            .iter()
            .map(|t| (&t.name, &t.description, &t.parameters))
            .collect::<Vec<_>>(),
    )
    .map_or(true, |s| s.len() > 16384)
    {
        return Err(JobError::CapacityUnavailable);
    }
    let session_id = engine
        .database
        .session_for_run(&job.id, &child.agent_run_id)
        .await?;
    let mut messages = engine.database.read_messages(&job.id, &session_id).await?;
    let mut attempt = 0u32;
    let mut repairs = 0u32;
    let mut used_call_ids = std::collections::HashSet::new();
    for msg in &messages {
        if let ChatMessage::AssistantToolCalls { calls } = msg {
            for call in calls {
                used_call_ids.insert(call.id.clone());
            }
        }
    }
    for _ in 0..job.snapshot.limits.max_worker_rounds {
        if cancellation.is_cancelled() {
            return Err(JobError::Cancelled);
        }
        attempt += 1;
        let context = run_context(&job.id, &child.agent_run_id, "worker", attempt);
        let models = engine.models.as_ref().ok_or(JobError::RuntimeUnavailable)?;
        engine
            .database
            .reserve_model_call(&context, &job.snapshot.worker)
            .await?;
        let deadline = job.deadline_at.min(
            super::super::engine::now()?
                .saturating_add(i64::from(job.snapshot.limits.step_timeout_seconds)),
        );
        let answer = models
            .turn_traced(
                &context,
                &job.snapshot.worker,
                LlmRequest {
                    messages: messages.clone(),
                    tools: tools.clone(),
                },
                BackgroundMode::ToolsAuto,
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
                    "status":if answer.is_ok(){"completed"}else{"failed"},
                    "error_code":answer.as_ref().err().map(|e|e.code()),
                    "duration_ms":context.started_at.elapsed().as_millis() as u64,
                }),
            )
            .await?;
        let calls = match answer?.turn {
            BackgroundTurn::Text(text) if !text.trim().is_empty() => {
                let summary: String = text.trim().chars().take(2000).collect();
                let refs = engine
                    .database
                    .child_artifact_refs(&job.id, &child.agent_run_id)
                    .await?;
                let result = ChildResult {
                    status: "completed".into(),
                    summary: summary.clone(),
                    findings: Vec::new(),
                    evidence_refs: refs,
                    unresolved: Vec::new(),
                };
                bounded_result(&serde_json::to_value(&result).map_err(|_| JobError::InvalidPlan)?)?;
                engine
                    .database
                    .append_messages(
                        &job.id,
                        &session_id,
                        &[ChatMessage::AssistantText { content: summary }],
                    )
                    .await?;
                return Ok(result);
            }
            BackgroundTurn::ToolCalls(calls) if calls.len() == 1 => calls,
            _ => {
                engine.database.record_agent_decision(&context,false,
                    json!({"validation_code":"tool_call_count","validation_path":"$.tool_calls","attempt_no":attempt})).await?;
                repairs += 1;
                if repairs > job.snapshot.limits.max_plan_repairs {
                    return Err(JobError::InvalidPlan);
                }
                let correction = ChatMessage::User {
                    content: "Use a permitted function or provide a final assistant text answer."
                        .into(),
                };
                engine
                    .database
                    .append_messages(&job.id, &session_id, std::slice::from_ref(&correction))
                    .await?;
                messages.push(correction);
                continue;
            }
        };
        let call = calls.into_iter().next().expect("checked count");
        let mut problem: Option<&str> = None;
        if call.id.is_empty() || call.id.len() > 128 || !used_call_ids.insert(call.id.clone()) {
            problem = Some("invalid_tool_call_id");
        }
        if call.arguments.to_string().len() > 16384
            || !call.arguments.is_object()
            || super::super::validation::forbidden(&call.arguments)
        {
            problem = Some("invalid_arguments_schema");
        }
        let mut validation_path = "$.tool_calls[0]";
        let mut allowed_evidence_refs: Vec<String> = Vec::new();
        if call.name == "submit_result" {
            if problem.is_none() {
                let result: Result<ChildResult, _> = serde_json::from_value(call.arguments.clone());
                if let Ok(result) = result {
                    if result.status == "completed"
                        && bounded_result(
                            &serde_json::to_value(&result).map_err(|_| JobError::InvalidPlan)?,
                        )
                        .is_ok()
                    {
                        allowed_evidence_refs = engine
                            .database
                            .child_artifact_refs(&job.id, &child.agent_run_id)
                            .await?;
                        if child_evidence_allowed(&result.evidence_refs, &allowed_evidence_refs) {
                            engine
                                .database
                                .record_agent_decision(
                                    &context,
                                    true,
                                    json!({"action":"finish","attempt_no":attempt}),
                                )
                                .await?;
                            let pair=[
                                ChatMessage::AssistantToolCalls{calls:vec![call.clone()]},
                                ChatMessage::ToolResult{tool_call_id:call.id.clone(),
                                    content:json!({"status":"completed","evidence_refs":result.evidence_refs}).to_string()}
                            ];
                            engine
                                .database
                                .append_messages(&job.id, &session_id, &pair)
                                .await?;
                            return Ok(result);
                        }
                        problem = Some("invalid_child_evidence_refs");
                        validation_path = "$.tool_calls[0].arguments.evidence_refs";
                    } else {
                        problem = Some("invalid_child_result");
                    }
                } else {
                    problem = Some("invalid_child_result");
                }
            }
        } else if !bindings.contains_key(&call.name) {
            problem = Some("unknown_operation");
        }
        let mut arguments = call.arguments.clone();
        if problem.is_none() {
            let (_choice, contract) = bindings
                .get(&call.name)
                .ok_or(JobError::CapabilityUnavailable)?;
            super::super::plugins::normalize_tool_arguments(
                &contract["input_schema"],
                &mut arguments,
            );
            if !arguments.is_object()
                || arguments.to_string().len() > 16384
                || super::super::validation::forbidden(&arguments)
                || !jsonschema::validator_for(&contract["input_schema"])
                    .map_err(|_| JobError::InvalidPlan)?
                    .is_valid(&arguments)
            {
                problem = Some("invalid_arguments_schema");
            }
        }
        if let Some(code) = problem {
            let previous_len = messages.len();
            engine.database.record_agent_decision(&context,false,json!({
                "validation_code":code,"validation_path":validation_path,"attempt_no":attempt,
            })).await?;
            repairs += 1;
            if repairs > job.snapshot.limits.max_plan_repairs {
                return Err(JobError::InvalidPlan);
            }
            // A rejected function with a valid ID is answered by a matching error result.
            if code != "invalid_tool_call_id" && !call.id.is_empty() && call.id.len() <= 128 {
                messages.push(ChatMessage::AssistantToolCalls {
                    calls: vec![call.clone()],
                });
                messages.push(ChatMessage::ToolResult {
                    tool_call_id: call.id,
                    content: json!({
                        "ok":false,
                        "error_code":code,
                        "allowed_evidence_refs":allowed_evidence_refs,
                        "guidance":if code=="invalid_child_evidence_refs" {
                            "Use only artifact IDs from earlier plugin Tool Results. Put source URLs in findings."
                        } else if code=="invalid_child_result" {
                            "Call submit_result with status=completed, summary, findings, evidence_refs and unresolved."
                        } else {
                            "Correct the invalid function call."
                        },
                    }).to_string(),
                });
            } else {
                messages.push(ChatMessage::User {
                    content: format!("Invalid function call: {code}"),
                });
            }
            engine
                .database
                .append_messages(&job.id, &session_id, &messages[previous_len..])
                .await?;
            continue;
        }
        repairs = 0;
        let (choice, contract) = bindings
            .get(&call.name)
            .ok_or(JobError::CapabilityUnavailable)?;
        engine
            .database
            .record_agent_decision(
                &context,
                true,
                json!({
                    "action":"call_plugin","attempt_no":attempt
                }),
            )
            .await?;
        // Durable accepted call intent precedes the external plugin attempt.
        engine
            .database
            .append_messages(
                &job.id,
                &session_id,
                &[ChatMessage::AssistantToolCalls {
                    calls: vec![call.clone()],
                }],
            )
            .await?;
        let turn_id = last_turn_id(engine, &job.id, &child.agent_run_id).await?;
        let key = engine
            .database
            .insert_dynamic_step(
                &job.id,
                &child.agent_run_id,
                &turn_id,
                choice,
                &arguments,
                &contract["output_schema"],
            )
            .await?;
        let after = engine
            .execute_plugin_step_cancel(&job.id, &key, choice, arguments, cancellation.clone())
            .await?;
        if after
            .steps
            .iter()
            .find(|step| step.step_key == key)
            .is_some_and(|step| step.attempt_state.as_deref() == Some("unknown_outcome"))
        {
            return Err(JobError::RuntimeUnavailable);
        }
        let (status, result, error) = engine.database.job_step_outcome(&job.id, &key).await?;
        let evidence_refs = engine
            .database
            .job_step_artifact_refs(&job.id, &key)
            .await?;
        let observation = plugin_tool_result(
            &key,
            &status,
            error.as_deref(),
            result.as_deref(),
            &evidence_refs,
        );
        let pair = [
            ChatMessage::AssistantToolCalls {
                calls: vec![call.clone()],
            },
            ChatMessage::ToolResult {
                tool_call_id: call.id,
                content: observation.to_string(),
            },
        ];
        engine
            .database
            .append_messages(&job.id, &session_id, &pair[1..])
            .await?;
        messages.extend(pair);
    }
    Err(JobError::CapacityUnavailable)
}
pub(super) async fn finish_child(
    engine: &JobEngine,
    job: &JobView,
    child: &AgentRunView,
    cancel: CancellationToken,
) -> Result<(), JobError> {
    let outcome = run_child(engine, job, child, cancel).await;
    let (state, result, error) = match outcome {
        Ok(value) => (
            "succeeded",
            Some(serde_json::to_value(value).map_err(|_| JobError::InvalidPlan)?),
            None,
        ),
        Err(code) => (
            if code == JobError::Cancelled {
                "cancelled"
            } else {
                "failed"
            },
            None,
            Some(code.code()),
        ),
    };
    engine
        .database
        .finish_child_run(&job.id, &child.agent_run_id, state, result.as_ref(), error)
        .await
}
