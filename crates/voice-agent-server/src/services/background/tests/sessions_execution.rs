//! Deterministic native executor acceptance, using real Core operation execution.
use super::super::models::JobModels;
use super::*;
use crate::providers::llm::{
    BackgroundMode, BackgroundTurn, BackgroundTurnResult, ChatMessage, LlmRequest, ToolCall,
};
use std::sync::atomic::{AtomicUsize, Ordering};
use tokio_util::sync::CancellationToken;

fn call(id: &str, name: &str, arguments: Value) -> ToolCall {
    ToolCall {
        id: id.into(),
        name: name.into(),
        arguments,
    }
}
fn delegate(id: &str, description: &str) -> ToolCall {
    call(
        id,
        "subagent",
        json!({"agent":"research","description":description,
        "prompt":format!("PRIVATE_PROMPT_{description}: complete this task"),
        "model":null,"sessionID":null,"background":false}),
    )
}
fn finish() -> ToolCall {
    call(
        "finish",
        "finish_job",
        json!({"summary":"Done","findings":[],"unresolved":[]}),
    )
}
struct Scenario {
    db: Arc<Database>,
    chain: bool,
    invalid: bool,
    semantic: Option<&'static str>,
    legacy_finish_refs: bool,
    root_round: AtomicUsize,
    child_round: AtomicUsize,
}
#[async_trait::async_trait]
impl JobModels for Scenario {
    async fn turn_traced(
        &self,
        _: &model::LlmRunContext,
        _: &ProviderPin,
        request: LlmRequest,
        mode: BackgroundMode,
        _: i64,
        _: CancellationToken,
    ) -> Result<BackgroundTurnResult, JobError> {
        let turn = if request.tools.iter().any(|t| t.name == "subagent") {
            assert!(matches!(mode, BackgroundMode::ToolsParallel));
            let round = self.root_round.fetch_add(1, Ordering::SeqCst);
            if round == 0 {
                if self.invalid {
                    let mut bad = delegate("bad", "invalid");
                    // Violates the SOURCE schema; serde's defaults must not authorize this.
                    if let Some(mode) = self.semantic {
                        match mode {
                            "model" => bad.arguments["model"] = json!("config:llm:foreign"),
                            "session" => bad.arguments["sessionID"] = json!("foreign-session"),
                            "background" => bad.arguments["background"] = json!(true),
                            _ => unreachable!(),
                        }
                    } else {
                        bad.arguments.as_object_mut().unwrap().remove("background");
                    }
                    BackgroundTurn::ToolCalls(vec![delegate("valid", "valid"), bad])
                } else if self.chain {
                    BackgroundTurn::ToolCalls(vec![delegate("chain", "chain")])
                } else {
                    BackgroundTurn::ToolCalls(vec![
                        delegate("good", "good"),
                        delegate("failure", "failure"),
                    ])
                }
            } else {
                let results: Vec<Value> = request
                    .messages
                    .iter()
                    .filter_map(|m| match m {
                        ChatMessage::ToolResult { content, .. } => {
                            Some(serde_json::from_str(content).unwrap())
                        }
                        _ => None,
                    })
                    .collect();
                if self.invalid {
                    let code = match self.semantic {
                        Some("model") => "model_not_allowed",
                        Some("session") => "session_access_denied",
                        Some("background") => "background_subagent_not_enabled",
                        _ => "agent_tool_protocol_error",
                    };
                    assert!(results.len() >= 2);
                    assert!(results.iter().take(2).all(|r| r["error_code"] == code));
                } else if !self.chain {
                    assert_eq!(results.len(), 2);
                    assert_eq!(
                        results
                            .iter()
                            .filter(|r| r["status"] == "completed")
                            .count(),
                        1
                    );
                    assert_eq!(
                        results.iter().filter(|r| r["status"] == "failed").count(),
                        1
                    );
                }
                if self.legacy_finish_refs && round == 2 {
                    assert!(request.messages.iter().any(
                        |msg| matches!(msg, ChatMessage::ToolResult{tool_call_id, content}
                            if tool_call_id == "finish_1"
                            && content.contains("finish_job_server_owned_evidence")
                            && content.contains("Remove evidence_refs"))
                    ));
                }
                let mut final_call = finish();
                final_call.id = format!("finish_{round}");
                if self.semantic == Some("article_uncertainty") {
                    final_call.arguments["summary"] =
                        json!("AFC chưa đưa ra quyết định chính thức về việc hoãn Asian Cup 2027.");
                    final_call.arguments["unresolved"] = json!([
                        "Khả năng lùi thời gian hoặc đổi nước chủ nhà vẫn chưa có quyết định chính thức."
                    ]);
                }
                if self.legacy_finish_refs && round == 1 {
                    // Replays the real regression: Root cites the source URL as
                    // an artifact, although a URL extract created no artifact.
                    final_call.arguments["evidence_refs"] =
                        json!(["https://news.example/asian-cup.html"]);
                }
                BackgroundTurn::ToolCalls(vec![final_call])
            }
        } else {
            assert!(matches!(mode, BackgroundMode::ToolsAuto));
            let prompt = request
                .messages
                .iter()
                .find_map(|m| match m {
                    ChatMessage::User { content } => Some(content.as_str()),
                    _ => None,
                })
                .unwrap();
            assert!(request.messages.iter().any(|m|matches!(m,ChatMessage::System{content} if content.contains("Research Sub-Agent"))));
            if prompt.contains("PRIVATE_PROMPT_failure") {
                return Err(JobError::ModelFailed);
            }
            let round = self.child_round.fetch_add(1, Ordering::SeqCst);
            if self.semantic == Some("core_disabled") {
                if round == 0 {
                    // Submission already froze the Core tool. Revoking it
                    // before Step start must produce a paired, known denial.
                    self.db
                        .set_core_plugin_enabled("core.data", 1, false)
                        .await
                        .unwrap();
                    assert!(
                        request
                            .tools
                            .iter()
                            .any(|tool| tool.name == "p_core_data_json_query")
                    );
                    return Ok(BackgroundTurnResult {
                        turn: BackgroundTurn::ToolCalls(vec![call(
                            "revoked_tool",
                            "p_core_data_json_query",
                            json!({"json_text":"{\"name\":\"Ada\"}","pointer":"/name"}),
                        )]),
                        usage: None,
                    });
                }
                let paired: Vec<_> = request
                    .messages
                    .iter()
                    .filter_map(|message| match message {
                        ChatMessage::ToolResult {
                            tool_call_id,
                            content,
                        } if tool_call_id == "revoked_tool" => Some(content.as_str()),
                        _ => None,
                    })
                    .collect();
                assert_eq!(
                    paired.len(),
                    1,
                    "disabled Core call must have exactly one Tool Result"
                );
                assert!(paired[0].contains("core_plugin_disabled"), "{paired:?}");
                return Ok(BackgroundTurnResult {
                    turn: BackgroundTurn::Text("PRIVATE_CHILD_OUTPUT: denied Core call".into()),
                    usage: None,
                });
            }
            if self.chain && round < 2 {
                let (name, args) = if round == 0 {
                    (
                        "p_core_data_json_query",
                        json!({"json_text":"{\"name\":\"Ada\"}","pointer":"/name"}),
                    )
                } else {
                    (
                        "p_core_data_transform",
                        json!({"json_text":"{\"name\":\"Ada\",\"age\":20}","mode":"pick","fields":["name"],"field":null,"equals":null}),
                    )
                };
                assert!(request.tools.iter().any(|t| t.name == name));
                if round == 1 {
                    assert!(request.messages.iter().any(|m|matches!(m,ChatMessage::ToolResult{tool_call_id,..} if tool_call_id=="tool_a")));
                }
                BackgroundTurn::ToolCalls(vec![call(
                    if round == 0 { "tool_a" } else { "tool_b" },
                    name,
                    args,
                )])
            } else {
                BackgroundTurn::Text("PRIVATE_CHILD_OUTPUT: Ada".into())
            }
        };
        Ok(BackgroundTurnResult { turn, usage: None })
    }
}
async fn execute(chain: bool, invalid: bool) -> (Arc<Database>, JobView) {
    execute_scenario(chain, invalid, None).await
}
async fn execute_scenario(
    chain: bool,
    invalid: bool,
    semantic: Option<&'static str>,
) -> (Arc<Database>, JobView) {
    execute_scenario_with_legacy(chain, invalid, semantic, false).await
}

async fn execute_scenario_with_legacy(
    chain: bool,
    invalid: bool,
    semantic: Option<&'static str>,
    legacy_finish_refs: bool,
) -> (Arc<Database>, JobView) {
    let config = Arc::new(config());
    let db = Arc::new(Database::connect(&config.database).await.unwrap());
    let runtime = BackgroundRuntime::execution_harness(
        db.clone(),
        config,
        Arc::new(Scenario {
            db: db.clone(),
            chain,
            invalid,
            semantic,
            legacy_finish_refs,
            root_round: AtomicUsize::new(0),
            child_round: AtomicUsize::new(0),
        }),
    )
    .await
    .unwrap();
    enable(&runtime.engine).await;
    let job = db
        .submit_job(
            &runtime.engine.config,
            request("native acceptance"),
            &JobRequester::Admin,
            crate::database::jobs::SubmissionIdentity {
                ..Default::default()
            },
            crate::database::unix_seconds().unwrap(),
            || true,
        )
        .await
        .unwrap()
        .job;
    runtime.engine.wake.notify_one();
    let result = wait_for(&runtime.engine, &job.id, |j| {
        matches!(j.status.as_str(), "succeeded" | "failed")
    })
    .await;
    runtime.shutdown().await;
    (db, result)
}
#[tokio::test]
async fn child_tool_chain_persists_pairs_then_final_text_and_metadata_only_events() {
    let (db, job) = execute(true, false).await;
    assert_eq!(job.status, "succeeded", "{:?}", job.error_code);
    assert_eq!(job.steps.len(), 2);
    assert!(job.steps.iter().all(|s| s.status == "succeeded"));
    let child = job
        .agent_runs
        .iter()
        .find(|r| r.parent_agent_run_id.is_some())
        .unwrap();
    assert_eq!(
        child.goal_excerpt, "chain",
        "display title must not expose full task prompt"
    );
    let sid = db
        .session_for_run(&job.id, &child.agent_run_id)
        .await
        .unwrap();
    let history = db.read_messages(&job.id, &sid).await.unwrap();
    for id in ["tool_a", "tool_b"] {
        assert_eq!(
            history
                .iter()
                .filter(|m| matches!(m,ChatMessage::AssistantToolCalls{calls} if calls[0].id==id))
                .count(),
            1
        );
        assert_eq!(
            history
                .iter()
                .filter(
                    |m| matches!(m,ChatMessage::ToolResult{tool_call_id,..} if tool_call_id==id)
                )
                .count(),
            1
        );
    }
    assert!(matches!(
        history.last().unwrap(),
        ChatMessage::AssistantText { .. }
    ));
    let events = db.job_events(&job.id, 0, 100).await.unwrap();
    let wire = serde_json::to_string(&events).unwrap();
    assert!(!wire.contains("PRIVATE_PROMPT"));
    assert!(!wire.contains("PRIVATE_CHILD_OUTPUT"));
    // Terminal deletion must remove both sessions and transcript, with FK checks on.
    db.delete_terminal_job(&job.id).await.unwrap();
    let remaining: i64 =
        sqlx::query_scalar("SELECT count(*) FROM job_agent_sessions WHERE job_id=?")
            .bind(&job.id)
            .fetch_one(db.pool())
            .await
            .unwrap();
    assert_eq!(remaining, 0);
}
#[tokio::test]
async fn one_failed_child_still_joins_and_delivers_both_outcomes() {
    let (db, job) = execute(false, false).await;
    assert_eq!(job.status, "failed");
    let events = db.job_events(&job.id, 0, 100).await.unwrap();
    assert_eq!(
        events
            .iter()
            .filter(|e| e.event_type == "join_completed")
            .count(),
        1
    );
    assert_eq!(
        job.agent_runs
            .iter()
            .filter(|r| r.parent_agent_run_id.is_some() && r.status == "succeeded")
            .count(),
        1
    );
}
#[tokio::test]
async fn schema_invalid_second_call_rejects_whole_batch_without_children_or_steps() {
    let (_, job) = execute(false, true).await;
    assert_eq!(job.agent_runs.len(), 1);
    assert!(job.steps.is_empty());
}

#[tokio::test]
async fn unauthorized_model_session_or_background_rejects_whole_batch_with_paired_errors() {
    for violation in ["model", "session", "background"] {
        let (_, job) = execute_scenario(false, true, Some(violation)).await;
        assert_eq!(
            job.agent_runs.len(),
            1,
            "{violation} must reject the valid sibling too"
        );
        assert!(job.steps.is_empty());
        assert_eq!(
            job.status, "failed",
            "Root text alone must not bypass completion validation"
        );
    }
}

#[tokio::test]
async fn root_bad_url_evidence_gets_matched_guidance_then_finishes_without_artifacts() {
    // Correct integration seam: Root -> Child native tools -> Child completes
    // with no artifact -> Root sends legacy URL in evidence_refs -> matched
    // protocol error -> Root retries with new schema -> persisted Job succeeds.
    let (db, job) = execute_scenario_with_legacy(true, false, None, true).await;
    assert_eq!(job.status, "succeeded", "{:?}", job.error_code);
    assert!(job.artifacts.is_empty());
    assert_eq!(job.result.as_ref().unwrap()["evidence_refs"], json!([]));
    let root = job
        .agent_runs
        .iter()
        .find(|run| run.parent_agent_run_id.is_none())
        .unwrap();
    let root_session = db
        .session_for_run(&job.id, &root.agent_run_id)
        .await
        .unwrap();
    let messages = db.read_messages(&job.id, &root_session).await.unwrap();
    let repairs: Vec<_> = messages
        .iter()
        .filter_map(|message| match message {
            ChatMessage::ToolResult {
                tool_call_id,
                content,
            } if tool_call_id == "finish_1" => {
                Some(serde_json::from_str::<Value>(content).unwrap())
            }
            _ => None,
        })
        .collect();
    assert_eq!(repairs.len(), 1);
    assert_eq!(repairs[0]["error_code"], "finish_job_server_owned_evidence");
    assert!(
        repairs[0]["guidance"]
            .as_str()
            .unwrap()
            .contains("Remove evidence_refs")
    );
    let events = db.job_events(&job.id, 0, 100).await.unwrap();
    assert!(
        events
            .iter()
            .any(|event| event.event_type == "agent_decision_rejected"
                && event.payload["validation_code"] == "finish_job_server_owned_evidence")
    );
}

#[tokio::test]
async fn root_unresolved_article_facts_do_not_fail_completed_job() {
    // Regression from a real single-article Job: Child and required source
    // succeeded, but Root used unresolved to report that AFC has not yet
    // decided whether to postpone the tournament. This is not unfinished work.
    let (db, job) = execute_scenario(true, false, Some("article_uncertainty")).await;
    assert_eq!(job.status, "succeeded", "{:?}", job.error_code);
    assert!(job.error_code.is_none());
    assert!(job.artifacts.is_empty(), "text-only jobs need no artifact");
    assert!(job.agent_runs.iter().all(|run| run.status == "succeeded"));
    assert_eq!(job.steps.len(), 2);
    assert!(job.steps.iter().all(|step| step.status == "succeeded"));
    let result = job.result.as_ref().expect("terminal Job result");
    assert_eq!(result["evidence_refs"], json!([]));
    assert_eq!(
        result["unresolved"],
        json!(["Khả năng lùi thời gian hoặc đổi nước chủ nhà vẫn chưa có quyết định chính thức."])
    );
    assert!(
        result["summary"]
            .as_str()
            .unwrap()
            .contains("chưa đưa ra quyết định")
    );
    let events = db.job_events(&job.id, 0, 100).await.unwrap();
    assert!(
        events
            .iter()
            .any(|event| event.event_type == "job_finished"
                && event.payload["status"] == "succeeded")
    );
    assert!(
        !events
            .iter()
            .any(|event| event.event_type == "agent_decision_rejected")
    );
}

#[tokio::test]
async fn disabled_core_tool_call_is_paired_in_child_session_v3() {
    let (db, job) = execute_scenario(true, false, Some("core_disabled")).await;
    let disabled = job
        .steps
        .iter()
        .find(|step| step.operation.as_deref() == Some("data.json_query"))
        .expect("child attempted frozen Core operation");
    assert_eq!(disabled.status, "failed");
    assert_eq!(disabled.error_code.as_deref(), Some("core_plugin_disabled"));
    assert_ne!(disabled.attempt_state.as_deref(), Some("unknown_outcome"));
    let child = job
        .agent_runs
        .iter()
        .find(|run| run.parent_agent_run_id.is_some())
        .unwrap();
    let sid = db
        .session_for_run(&job.id, &child.agent_run_id)
        .await
        .unwrap();
    let transcript = db.read_messages(&job.id, &sid).await.unwrap();
    assert_eq!(
        transcript
            .iter()
            .filter(|message| matches!(message,
                ChatMessage::AssistantToolCalls { calls }
                if calls.iter().any(|call| call.id == "revoked_tool")
            ))
            .count(),
        1
    );
    assert_eq!(
        transcript
            .iter()
            .filter(|message| matches!(message,
                ChatMessage::ToolResult { tool_call_id, content }
                if tool_call_id == "revoked_tool" && content.contains("core_plugin_disabled")
            ))
            .count(),
        1
    );
}
