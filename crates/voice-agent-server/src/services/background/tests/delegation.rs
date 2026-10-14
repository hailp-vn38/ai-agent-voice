use super::*;
use crate::providers::llm::{ChatMessage, LlmRequest};
use crate::services::background::models::JobModels;
use tokio_util::sync::CancellationToken;

pub(super) struct SessionModels;

#[async_trait::async_trait]
impl JobModels for SessionModels {
    async fn turn_traced(
        &self,
        _: &crate::services::background::model::LlmRunContext,
        _: &ProviderPin,
        request: LlmRequest,
        mode: crate::providers::llm::BackgroundMode,
        _: i64,
        _: CancellationToken,
    ) -> Result<crate::providers::llm::BackgroundTurnResult, JobError> {
        use crate::providers::llm::{
            BackgroundMode, BackgroundTurn, BackgroundTurnResult, ToolCall,
        };
        let root = request.tools.iter().any(|tool| tool.name == "subagent");
        let output = if root {
            assert!(matches!(mode, BackgroundMode::ToolsParallel));
            if request
                .messages
                .iter()
                .any(|m| matches!(m, ChatMessage::ToolResult { .. }))
            {
                BackgroundTurn::ToolCalls(vec![ToolCall {
                    id: "root_finish".into(),
                    name: "finish_job".into(),
                    arguments: json!({"summary":"Research done","findings":[],
                        "evidence_refs":[],"unresolved":[]}),
                }])
            } else {
                BackgroundTurn::ToolCalls(vec![ToolCall {
                    id: "root_delegate".into(),
                    name: "subagent".into(),
                    arguments: json!({"agent":"research",
                        "description":"Research task",
                        "prompt":"Investigate exact session tool result behavior and report your evidence.",
                        "model":null,"sessionID":null,"background":false}),
                }])
            }
        } else {
            assert!(matches!(mode, BackgroundMode::ToolsAuto));
            assert!(
                request
                    .messages
                    .iter()
                    .any(|m| matches!(m,ChatMessage::User{content}
                if content.contains("Investigate exact session tool result behavior")))
            );
            BackgroundTurn::Text("Child completed a verified text investigation".into())
        };
        Ok(BackgroundTurnResult {
            turn: output,
            usage: None,
        })
    }
}

#[tokio::test]
async fn root_subagent_prompt_child_text_and_matched_result_are_durable() {
    let config = Arc::new(config());
    let db = Arc::new(Database::connect(&config.database).await.unwrap());
    let runtime = BackgroundRuntime::execution_harness(db.clone(), config, Arc::new(SessionModels))
        .await
        .unwrap();
    enable(&runtime.engine).await;
    let receipt = db
        .submit_job(
            &runtime.engine.config,
            request("Research session tools"),
            &JobRequester::Admin,
            crate::database::jobs::SubmissionIdentity {
                key: Some("session-e2e"),
                ..Default::default()
            },
            crate::database::unix_seconds().unwrap(),
            || runtime.engine.runtime_ready(),
        )
        .await
        .unwrap();
    let job = wait_for(&runtime.engine, &receipt.job.id, |j| {
        j.status == "succeeded"
    })
    .await;
    assert_eq!(job.execution_mode, "hierarchical_v3");
    assert_eq!(job.agent_runs.len(), 2);
    let root = job
        .agent_runs
        .iter()
        .find(|r| r.parent_agent_run_id.is_none())
        .unwrap();
    let child = job
        .agent_runs
        .iter()
        .find(|r| r.parent_agent_run_id.is_some())
        .unwrap();
    let root_sid = db
        .session_for_run(&job.id, &root.agent_run_id)
        .await
        .unwrap();
    let child_sid = db
        .session_for_run(&job.id, &child.agent_run_id)
        .await
        .unwrap();
    assert_ne!(root_sid, child_sid);
    let root_messages = db.read_messages(&job.id, &root_sid).await.unwrap();
    let child_messages = db.read_messages(&job.id, &child_sid).await.unwrap();
    assert!(
        child_messages
            .iter()
            .any(|msg| matches!(msg,ChatMessage::User{content}
        if content.contains("Investigate exact session tool result behavior")))
    );
    assert!(
        child_messages
            .iter()
            .any(|msg| matches!(msg,ChatMessage::AssistantText{content}
        if content.contains("verified text investigation")))
    );
    assert!(
        root_messages
            .iter()
            .any(|msg| matches!(msg,ChatMessage::AssistantToolCalls{calls}
        if calls.iter().any(|c|c.id=="root_delegate")))
    );
    assert!(root_messages.iter().any(
        |msg| matches!(msg,ChatMessage::ToolResult{tool_call_id,content}
        if tool_call_id=="root_delegate" && content.contains(&child_sid))
    ));
    let events = db.job_events(&job.id, 0, 100).await.unwrap();
    assert_eq!(
        events
            .iter()
            .filter(|e| e.event_type == "join_completed")
            .count(),
        1
    );
}

struct DelegationModels {
    resume: bool,
}

#[async_trait::async_trait]
impl JobModels for DelegationModels {
    async fn turn_traced(
        &self,
        _: &crate::services::background::model::LlmRunContext,
        _: &ProviderPin,
        request: LlmRequest,
        mode: crate::providers::llm::BackgroundMode,
        _: i64,
        _: CancellationToken,
    ) -> Result<crate::providers::llm::BackgroundTurnResult, JobError> {
        use crate::providers::llm::{
            BackgroundMode, BackgroundTurn, BackgroundTurnResult, ToolCall,
        };
        let make_sub = |id: &str, agent: &str, prompt: &str, session: Option<String>| ToolCall {
            id: id.into(),
            name: "subagent".into(),
            arguments: json!({"agent":agent,"description":"Investigate",
                "prompt":prompt,"model":null,"sessionID":session,"background":false}),
        };
        let root = request.tools.iter().any(|t| t.name == "subagent");
        let turn = if root {
            assert!(matches!(mode, BackgroundMode::ToolsParallel));
            let outcomes: Vec<_> = request
                .messages
                .iter()
                .filter_map(|msg| {
                    if let ChatMessage::ToolResult { content, .. } = msg {
                        serde_json::from_str::<serde_json::Value>(content).ok()
                    } else {
                        None
                    }
                })
                .filter(|v| v.get("sessionID").is_some())
                .collect();
            if outcomes.is_empty() {
                if self.resume {
                    BackgroundTurn::ToolCalls(vec![make_sub(
                        "start_1",
                        "research",
                        "Initial analysis",
                        None,
                    )])
                } else {
                    BackgroundTurn::ToolCalls(vec![
                        make_sub("parallel_a", "research", "Research architecture", None),
                        make_sub("parallel_b", "document", "Summarize architecture", None),
                    ])
                }
            } else if self.resume && outcomes.len() == 1 {
                let session = outcomes[0]["sessionID"].as_str().unwrap();
                BackgroundTurn::ToolCalls(vec![make_sub(
                    "resume_2",
                    "research",
                    "Verify previous findings",
                    Some(session.into()),
                )])
            } else {
                BackgroundTurn::ToolCalls(vec![ToolCall {
                    id: "end_3".into(),
                    name: "finish_job".into(),
                    arguments: json!({"summary":"Tasks completed","findings":[],
                        "evidence_refs":[],"unresolved":[]}),
                }])
            }
        } else {
            assert!(matches!(mode, BackgroundMode::ToolsAuto));
            let resumed=request.messages.iter().any(|msg|
                matches!(msg,ChatMessage::User{content} if content=="Verify previous findings")
            );
            if resumed {
                assert!(
                    request
                        .messages
                        .iter()
                        .any(|msg| matches!(msg,ChatMessage::AssistantText{content}
                    if content=="Initial analysis completed"))
                );
                BackgroundTurn::Text("Verification completed".into())
            } else if request.messages.iter().any(|msg| {
                matches!(msg,ChatMessage::User{content}
                if content=="Initial analysis")
            }) {
                BackgroundTurn::Text("Initial analysis completed".into())
            } else {
                BackgroundTurn::Text("Independent task completed".into())
            }
        };
        Ok(BackgroundTurnResult { turn, usage: None })
    }
}

#[tokio::test]
async fn two_native_subagents_return_two_paired_results_and_one_join() {
    let config = Arc::new(config());
    let db = Arc::new(Database::connect(&config.database).await.unwrap());
    let runtime = BackgroundRuntime::execution_harness(
        db.clone(),
        config,
        Arc::new(DelegationModels { resume: false }),
    )
    .await
    .unwrap();
    enable(&runtime.engine).await;
    let receipt = db
        .submit_job(
            &runtime.engine.config,
            request("Two parallel investigations"),
            &JobRequester::Admin,
            crate::database::jobs::SubmissionIdentity {
                ..Default::default()
            },
            crate::database::unix_seconds().unwrap(),
            || runtime.engine.runtime_ready(),
        )
        .await
        .unwrap();
    let job = wait_for(&runtime.engine, &receipt.job.id, |j| {
        j.status == "succeeded"
    })
    .await;
    assert_eq!(job.agent_runs.len(), 3);
    assert!(job.agent_runs.iter().all(|r| r.status == "succeeded"));
    let root = job
        .agent_runs
        .iter()
        .find(|r| r.parent_agent_run_id.is_none())
        .unwrap();
    let session = db
        .session_for_run(&job.id, &root.agent_run_id)
        .await
        .unwrap();
    let messages = db.read_messages(&job.id, &session).await.unwrap();
    let results: Vec<String> = messages
        .iter()
        .filter_map(|m| {
            if let ChatMessage::ToolResult { tool_call_id, .. } = m {
                Some(tool_call_id.clone())
            } else {
                None
            }
        })
        .collect();
    assert!(results.contains(&"parallel_a".to_owned()));
    assert!(results.contains(&"parallel_b".to_owned()));
    let log = db.job_events(&job.id, 0, 100).await.unwrap();
    assert_eq!(
        log.iter()
            .filter(|e| e.event_type == "join_completed")
            .count(),
        1
    );
    runtime.shutdown().await;
}

#[tokio::test]
async fn resume_child_creates_new_run_and_keeps_prior_session_messages() {
    let config = Arc::new(config());
    let db = Arc::new(Database::connect(&config.database).await.unwrap());
    let runtime = BackgroundRuntime::execution_harness(
        db.clone(),
        config,
        Arc::new(DelegationModels { resume: true }),
    )
    .await
    .unwrap();
    enable(&runtime.engine).await;
    let receipt = db
        .submit_job(
            &runtime.engine.config,
            request("Investigate and follow up"),
            &JobRequester::Admin,
            crate::database::jobs::SubmissionIdentity {
                ..Default::default()
            },
            crate::database::unix_seconds().unwrap(),
            || runtime.engine.runtime_ready(),
        )
        .await
        .unwrap();
    let job = wait_for(&runtime.engine, &receipt.job.id, |j| {
        j.status == "succeeded"
    })
    .await;
    let children: Vec<_> = job
        .agent_runs
        .iter()
        .filter(|r| r.parent_agent_run_id.is_some())
        .collect();
    assert_eq!(children.len(), 2);
    assert_ne!(children[0].agent_run_id, children[1].agent_run_id);
    let first = db
        .session_for_run(&job.id, &children[0].agent_run_id)
        .await
        .unwrap();
    let second = db
        .session_for_run(&job.id, &children[1].agent_run_id)
        .await
        .unwrap();
    assert_eq!(first, second);
    let messages = db.read_messages(&job.id, &first).await.unwrap();
    assert!(
        messages
            .iter()
            .any(|m| matches!(m,ChatMessage::User{content}
        if content=="Initial analysis"))
    );
    assert!(
        messages
            .iter()
            .any(|m| matches!(m,ChatMessage::User{content}
        if content=="Verify previous findings"))
    );
    assert!(
        messages
            .iter()
            .any(|m| matches!(m,ChatMessage::AssistantText{content}
        if content=="Initial analysis completed"))
    );
    runtime.shutdown().await;
}

#[tokio::test]
async fn persisted_execution_mode_does_not_select_a_different_flow() {
    for mode in ["dag_v1", "hierarchical_v2", "hierarchical_v3"] {
        let config = Arc::new(config());
        let db = Arc::new(Database::connect(&config.database).await.unwrap());
        let runtime =
            BackgroundRuntime::execution_harness(db.clone(), config, Arc::new(SessionModels))
                .await
                .unwrap();
        enable(&runtime.engine).await;
        let receipt = db
            .submit_job(
                &runtime.engine.config,
                request("One native session flow for every stored mode"),
                &JobRequester::Admin,
                crate::database::jobs::SubmissionIdentity {
                    test_execution_mode: Some(mode),
                    ..Default::default()
                },
                crate::database::unix_seconds().unwrap(),
                || runtime.engine.runtime_ready(),
            )
            .await
            .unwrap();
        runtime.engine.wake.notify_one();
        let job = wait_for(&runtime.engine, &receipt.job.id, |job| {
            matches!(job.status.as_str(), "succeeded" | "failed")
        })
        .await;
        assert_eq!(job.status, "succeeded", "{mode}: {:?}", job.error_code);
        assert_eq!(job.execution_mode, mode, "stored history remains immutable");
        assert_eq!(job.budget_used["llm_calls"], 3);
        assert_eq!(job.agent_runs.len(), 2);
        for run in &job.agent_runs {
            assert_eq!(run.status, "succeeded");
            db.session_for_run(&job.id, &run.agent_run_id)
                .await
                .unwrap();
        }
        runtime.shutdown().await;
    }
}
