use super::*;
use crate::providers::llm::{ChatMessage, LlmRequest};
use crate::services::background::models::JobModels;
use tokio_util::sync::CancellationToken;

struct HierarchicalScriptedModels;
#[async_trait::async_trait]
impl JobModels for HierarchicalScriptedModels {
    async fn turn_traced(
        &self,
        run: &crate::services::background::model::LlmRunContext,
        pin: &ProviderPin,
        request: LlmRequest,
        mode: crate::providers::llm::BackgroundMode,
        deadline: i64,
        cancel: CancellationToken,
    ) -> Result<crate::providers::llm::BackgroundTurnResult, JobError> {
        use crate::providers::llm::{
            BackgroundMode, BackgroundTurn, BackgroundTurnResult, ToolCall,
        };
        match mode {
            BackgroundMode::Tools => {
                assert!(
                    request
                        .tools
                        .iter()
                        .any(|tool| tool.name == "submit_result")
                );
                Ok(BackgroundTurnResult {
                    turn: BackgroundTurn::ToolCalls(vec![ToolCall {
                        id: "complete_1".into(),
                        name: "submit_result".into(),
                        arguments: json!({
                            "status":"completed","summary":"Sub-agent task done",
                            "findings":[],"evidence_refs":[],"unresolved":[]
                        }),
                    }]),
                    usage: None,
                })
            }
            BackgroundMode::Structured { schema, .. } => {
                let raw = self
                    .complete_structured_traced(run, pin, request, schema, deadline, cancel)
                    .await?;
                Ok(BackgroundTurnResult {
                    turn: BackgroundTurn::Text(raw),
                    usage: None,
                })
            }
        }
    }
    async fn complete(
        &self,
        _: &str,
        _: &ProviderPin,
        request: LlmRequest,
        _: i64,
        _: CancellationToken,
    ) -> Result<String, JobError> {
        let is_root = request.messages.iter().any(
            |m| matches!(m,ChatMessage::System{content} if content.contains("Root Orchestrator")),
        );
        let first=request.messages.iter().any(|m|matches!(m,ChatMessage::User{content} if content.contains("\"joined_children\":[]")));
        if is_root && first {
            Ok(json!({"action":"delegate","tasks":[
                {"task_key":"research","role":"research","goal":"Find references",
                    "allowed_operations":[]},
                {"task_key":"document","role":"document","goal":"Review documents",
                    "allowed_operations":[]}
            ]})
            .to_string())
        } else if is_root {
            Ok(json!({"action":"finish","result":{
                "summary":"Both independent tasks completed",
                "findings":["Research completed","Document completed"],
                "evidence_refs":[],"unresolved":[]
            }})
            .to_string())
        } else {
            Ok(json!({"action":"finish","result":{
                "status":"completed","summary":"Sub-agent task done",
                "findings":[],"evidence_refs":[],"unresolved":[]
            }})
            .to_string())
        }
    }
}

#[tokio::test]
async fn hierarchical_job_forks_two_children_then_joins_and_finishes() {
    let config = Arc::new(config());
    let db = Arc::new(Database::connect(&config.database).await.unwrap());
    let runtime = BackgroundRuntime::execution_harness(
        db.clone(),
        config,
        Arc::new(HierarchicalScriptedModels),
    )
    .await
    .unwrap();
    enable(&runtime.engine).await;
    let receipt = db
        .submit_job(
            &runtime.engine.config,
            request("Research two independent topics"),
            &JobRequester::Admin,
            crate::database::jobs::SubmissionIdentity {
                key: Some("hierarchy-e2e"),
                test_execution_mode: Some("hierarchical_v2"),
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
    assert_eq!(job.execution_mode, "hierarchical_v2");
    assert_eq!(job.agent_runs.len(), 3);
    assert_eq!(
        job.agent_runs
            .iter()
            .filter(|r| r.parent_agent_run_id.is_some())
            .count(),
        2
    );
    assert!(job.agent_runs.iter().all(|r| r.status == "succeeded"));
    assert_eq!(
        job.result.as_ref().unwrap()["summary"],
        "Both independent tasks completed"
    );
    assert_eq!(job.budget_used["llm_calls"], 4);
    let log = db.job_events(&job.id, 0, 100).await.unwrap();
    assert_eq!(
        log.iter()
            .filter(|e| e.event_type == "agent_run_created")
            .count(),
        3
    );
    assert_eq!(
        log.iter()
            .filter(|e| e.event_type == "agent_run_finished")
            .count(),
        3
    );
    assert_eq!(
        log.iter()
            .filter(|e| e.event_type == "join_completed")
            .count(),
        1
    );
    assert_eq!(
        log.iter()
            .filter(|e| e.event_type == "llm_run_started")
            .count(),
        4
    );
    assert!(
        log.iter()
            .any(|e| e.event_type == "llm_run_started" && e.payload["agent_run_id"].is_string())
    );
    runtime.shutdown().await;
}

#[tokio::test]
async fn hierarchical_job_rejects_child_authority_expansion() {
    let (db, config, runtime) = harness().await;
    let receipt = db
        .submit_job(
            &config,
            request("Bounded authority"),
            &JobRequester::Admin,
            crate::database::jobs::SubmissionIdentity {
                key: Some("hierarchy-authority"),
                test_execution_mode: Some("hierarchical_v2"),
                ..Default::default()
            },
            crate::database::unix_seconds().unwrap(),
            || runtime.engine.runtime_ready(),
        )
        .await
        .unwrap();
    let id = &receipt.job.id;
    // Foundation harness does not run a model; transition to planning/claimed state.
    let claimed = db.claim_job(id, 2).await.unwrap();
    assert!(claimed);
    let root = db.create_root_run(id).await.unwrap();
    let attempted = db
        .create_children_for_decision(
            id,
            &root.agent_run_id,
            "fake-root-turn",
            &[crate::services::background::model::DelegatedTask {
                task_key: "unapproved".into(),
                role: "research".into(),
                goal: "Attempt a forbidden operation".into(),
                allowed_operations: vec!["uninstalled.execute".into()],
            }],
        )
        .await;
    assert!(matches!(
        attempted,
        Err(JobError::CapabilityUnavailable) | Err(JobError::InvalidPlan)
    ));
    runtime.shutdown().await;
}

struct RepairScriptedModels {
    attempts: std::sync::atomic::AtomicUsize,
}
#[async_trait::async_trait]
impl JobModels for RepairScriptedModels {
    async fn complete(
        &self,
        _id: &str,
        _pin: &ProviderPin,
        request: LlmRequest,
        _deadline: i64,
        _cancel: CancellationToken,
    ) -> Result<String, JobError> {
        use std::sync::atomic::Ordering;
        let n = self.attempts.fetch_add(1, Ordering::SeqCst);
        if n == 0 {
            return Ok(r#"{"action":"delegate","tasks":[{"role":"research","goal":"Read one document","allowed_operations":[]}]}"#.into());
        }
        let user = request
            .messages
            .iter()
            .find_map(|m| match m {
                ChatMessage::User { content } => Some(content.as_str()),
                _ => None,
            })
            .unwrap();
        assert!(
            user.contains("missing_field"),
            "repair should receive a precise code"
        );
        assert!(
            user.contains("$.tasks[0].task_key"),
            "repair should name the rejected path"
        );
        Ok("```json\n{\"action\":\"finish\",\"result\":{\"summary\":\"completed after validation repair\",\"findings\":[],\"evidence_refs\":[],\"unresolved\":[]}}\n```".into())
    }
}
#[tokio::test]
async fn invalid_root_decision_emits_precise_diagnostic_then_repairs_from_fenced_json() {
    let config = Arc::new(config());
    let db = Arc::new(Database::connect(&config.database).await.unwrap());
    let models = Arc::new(RepairScriptedModels {
        attempts: std::sync::atomic::AtomicUsize::new(0),
    });
    let runtime = BackgroundRuntime::execution_harness(db.clone(), config, models.clone())
        .await
        .unwrap();
    enable(&runtime.engine).await;
    let receipt = db
        .submit_job(
            &runtime.engine.config,
            request("Summarize one document"),
            &JobRequester::Admin,
            crate::database::jobs::SubmissionIdentity {
                key: Some("hierarchy-repair-diagnostics"),
                test_execution_mode: Some("hierarchical_v2"),
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
    assert_eq!(
        job.result.as_ref().unwrap()["summary"],
        "completed after validation repair"
    );
    assert_eq!(models.attempts.load(std::sync::atomic::Ordering::SeqCst), 2);
    let events = db.job_events(&job.id, 0, 100).await.unwrap();
    let rejected = events
        .iter()
        .find(|e| e.event_type == "agent_decision_rejected")
        .unwrap();
    assert_eq!(rejected.payload["validation_code"], "missing_field");
    assert_eq!(rejected.payload["validation_path"], "$.tasks[0].task_key");
    assert_eq!(rejected.payload["attempt_no"], 1);
    assert!(rejected.payload.get("raw_response").is_none());
    let accepted = events
        .iter()
        .find(|e| e.event_type == "agent_decision_accepted")
        .unwrap();
    assert_eq!(accepted.payload["action"], "finish");
    assert_eq!(
        events
            .iter()
            .filter(|e| e.event_type == "llm_run_started")
            .count(),
        2
    );
    assert_eq!(
        events
            .iter()
            .filter(|e| e.event_type == "llm_run_finished")
            .count(),
        2
    );
    runtime.shutdown().await;
}

struct OperationRepairModels {
    attempts: std::sync::atomic::AtomicUsize,
    expected_operations: Value,
}
#[async_trait::async_trait]
impl JobModels for OperationRepairModels {
    async fn complete_structured(
        &self,
        id: &str,
        pin: &ProviderPin,
        request: LlmRequest,
        schema: Value,
        deadline: i64,
        cancel: CancellationToken,
    ) -> Result<String, JobError> {
        let validator = jsonschema::validator_for(&schema).unwrap();
        let mut decision = json!({"decision":{"action":"delegate","tasks":[{
            "task_key":"research","role":"research","goal":"Read references",
            "allowed_operations":self.expected_operations
        }]}});
        assert!(validator.is_valid(&decision));
        decision["decision"]["tasks"][0]["allowed_operations"] =
            json!(["fixture-research.invented"]);
        assert!(!validator.is_valid(&decision));
        self.complete(id, pin, request, deadline, cancel).await
    }

    async fn complete(
        &self,
        _: &str,
        _: &ProviderPin,
        request: LlmRequest,
        _: i64,
        _: CancellationToken,
    ) -> Result<String, JobError> {
        let input: Value = request
            .messages
            .iter()
            .find_map(|message| match message {
                ChatMessage::User { content } => Some(serde_json::from_str(content).unwrap()),
                _ => None,
            })
            .unwrap();
        assert_eq!(input["operation_allowlist"], self.expected_operations);
        let attempt = self
            .attempts
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        if attempt == 0 {
            assert!(input["correction"].is_null());
            return Ok(json!({"action":"delegate","tasks":[{
                "task_key":"research","role":"research","goal":"Read references",
                "allowed_operations":["fixture-research.invented"]
            }]})
            .to_string());
        }
        assert_eq!(
            input["correction"]["diagnostic"]["validation_code"],
            "unknown_operation"
        );
        assert_eq!(
            input["correction"]["diagnostic"]["validation_path"],
            "$.tasks[0].allowed_operations[0]"
        );
        assert_eq!(
            input["correction"]["operation_allowlist"],
            self.expected_operations
        );
        Ok(json!({"action":"finish","result":{
            "summary":"Corrected operation selection","findings":[],"evidence_refs":[],"unresolved":[]
        }}).to_string())
    }
}

#[tokio::test]
async fn root_operation_allowlist_repairs_unknown_operation_with_and_without_plugins() {
    for installed in [false, true] {
        let config = Arc::new(config());
        let db = Arc::new(Database::connect(&config.database).await.unwrap());
        if installed {
            let manifest: toml::Value = toml::from_str(include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../plugins/fixture-research-1.0.0/plugin.toml"
            )))
            .unwrap();
            sqlx::query("INSERT INTO plugin_installations(plugin_id,version,package_digest,installation_digest,status,revision,operations_json,created_at,updated_at) VALUES('fixture-research','1.0.0',?,?,'ready',1,?,0,0)")
                .bind("a".repeat(64)).bind("b".repeat(64))
                .bind(serde_json::to_string(&manifest).unwrap())
                .execute(db.pool()).await.unwrap();
        }
        let models = Arc::new(OperationRepairModels {
            attempts: std::sync::atomic::AtomicUsize::new(0),
            expected_operations: if installed {
                json!([
                    "fixture-research.collect_topic",
                    "fixture-research.simulate_failure"
                ])
            } else {
                json!([])
            },
        });
        let runtime = BackgroundRuntime::execution_harness(db.clone(), config, models.clone())
            .await
            .unwrap();
        enable(&runtime.engine).await;
        let receipt = db
            .submit_job(
                &runtime.engine.config,
                request("Check operation selection"),
                &JobRequester::Admin,
                crate::database::jobs::SubmissionIdentity {
                    test_execution_mode: Some("hierarchical_v2"),
                    ..Default::default()
                },
                crate::database::unix_seconds().unwrap(),
                || runtime.engine.runtime_ready(),
            )
            .await
            .unwrap();
        let job = wait_for(&runtime.engine, &receipt.job.id, |job| {
            job.status == "succeeded"
        })
        .await;
        assert_eq!(models.attempts.load(std::sync::atomic::Ordering::SeqCst), 2);
        assert_eq!(job.agent_runs.len(), 1);
        assert!(job.steps.is_empty());
        let events = db.job_events(&job.id, 0, 100).await.unwrap();
        let rejection = events
            .iter()
            .find(|event| event.event_type == "agent_decision_rejected")
            .unwrap();
        assert_eq!(rejection.payload["validation_code"], "unknown_operation");
        assert_eq!(
            rejection.payload["validation_path"],
            "$.tasks[0].allowed_operations[0]"
        );
        assert!(rejection.payload.get("operation_allowlist").is_none());
        runtime.shutdown().await;
    }
}

struct AlwaysInvalidModels;
#[async_trait::async_trait]
impl JobModels for AlwaysInvalidModels {
    async fn complete(
        &self,
        _id: &str,
        _pin: &ProviderPin,
        _request: LlmRequest,
        _deadline: i64,
        _cancel: CancellationToken,
    ) -> Result<String, JobError> {
        Ok(r#"{"action":"delegate","tasks":[{"task_key":"sample","role":"search","goal":"x","allowed_operations":[]}]}"#.into())
    }
}
#[tokio::test]
async fn three_invalid_root_responses_report_exact_rejection_before_invalid_plan() {
    let config = Arc::new(config());
    let db = Arc::new(Database::connect(&config.database).await.unwrap());
    let runtime =
        BackgroundRuntime::execution_harness(db.clone(), config, Arc::new(AlwaysInvalidModels))
            .await
            .unwrap();
    enable(&runtime.engine).await;
    let receipt = db
        .submit_job(
            &runtime.engine.config,
            request("Read one document"),
            &JobRequester::Admin,
            crate::database::jobs::SubmissionIdentity {
                key: Some("hierarchy-invalid-three"),
                test_execution_mode: Some("hierarchical_v2"),
                ..Default::default()
            },
            crate::database::unix_seconds().unwrap(),
            || runtime.engine.runtime_ready(),
        )
        .await
        .unwrap();
    let job = wait_for(&runtime.engine, &receipt.job.id, |j| j.status == "failed").await;
    assert_eq!(job.error_code.as_deref(), Some("invalid_plan"));
    let events = db.job_events(&job.id, 0, 100).await.unwrap();
    let rejects: Vec<_> = events
        .iter()
        .filter(|e| e.event_type == "agent_decision_rejected")
        .collect();
    assert_eq!(rejects.len(), 3);
    assert!(
        rejects
            .iter()
            .all(|e| e.payload["validation_code"] == "invalid_role")
    );
    assert!(
        rejects
            .iter()
            .all(|e| e.payload["validation_path"] == "$.tasks[0].role")
    );
    assert_eq!(
        events
            .iter()
            .filter(|e| e.event_type == "agent_decision_accepted")
            .count(),
        0
    );
    runtime.shutdown().await;
}

#[tokio::test]
async fn hierarchical_submission_rejects_provider_without_required_native_mode() {
    let (db, config, runtime) = harness().await;
    sqlx::query("INSERT INTO providers(key,name,type,adapter,config_json,created_at,updated_at) VALUES('unqualified','Unqualified','llm','openai','{}',0,0)")
        .execute(db.pool()).await.unwrap();
    for (root, worker, expected_code) in [
        ("unqualified", "config:llm:planner", "orchestrator_structured_outputs_required"),
        ("config:llm:planner", "unqualified", "worker_function_calling_required"),
    ] {
        let revision = runtime.engine.settings().await.unwrap().revision;
        runtime
            .engine
            .update_settings(
                BackgroundSettingsUpdate {
                    enabled: true,
                    orchestrator_llm_provider: Some(root.into()),
                    worker_llm_provider: Some(worker.into()),
                },
                revision,
            )
            .await
            .unwrap();
        let receipt = db
            .submit_job(
                &config,
                request("Requires strict provider"),
                &JobRequester::Admin,
                crate::database::jobs::SubmissionIdentity {
                    test_execution_mode: Some("hierarchical_v2"),
                    ..Default::default()
                },
                crate::database::unix_seconds().unwrap(),
                || runtime.engine.runtime_ready(),
            )
            .await;
        assert!(matches!(receipt, Err(error) if error.code() == expected_code),
            "Missing capability must identify the selected role and required native mode");
    }
    runtime.shutdown().await;
}

#[derive(Clone, Copy, Debug)]
enum RejectedChildKind {
    MultipleCalls,
    UnknownFunction,
    FakeEvidence,
}
struct AdversarialChildModels {
    kind: RejectedChildKind,
    attempts: std::sync::atomic::AtomicUsize,
}
#[async_trait::async_trait]
impl JobModels for AdversarialChildModels {
    async fn complete(
        &self,
        _: &str,
        _: &ProviderPin,
        _: LlmRequest,
        _: i64,
        _: CancellationToken,
    ) -> Result<String, JobError> {
        Err(JobError::InvalidPlan)
    }
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
        let turn =
            match mode {
                BackgroundMode::Structured { .. } => {
                    let input = request
                        .messages
                        .iter()
                        .find_map(|m| {
                            if let ChatMessage::User { content } = m {
                                serde_json::from_str::<Value>(content).ok()
                            } else {
                                None
                            }
                        })
                        .ok_or(JobError::InvalidPlan)?;
                    if input["joined_children"]
                        .as_array()
                        .is_some_and(Vec::is_empty)
                    {
                        BackgroundTurn::Text(json!({"decision":{"action":"delegate","tasks":[{
                        "task_key":"research","role":"research","goal":"Inspect a reference",
                        "allowed_operations":[]
                    }]}}).to_string())
                    } else {
                        BackgroundTurn::Text(
                            json!({"decision":{"action":"finish","result":{
                                "summary":"Child did not provide usable evidence",
                                "findings":[],"evidence_refs":[],"unresolved":["Child failed"]
                            }}})
                            .to_string(),
                        )
                    }
                }
                BackgroundMode::Tools => {
                    assert_eq!(
                        request.tools.len(),
                        1,
                        "only internal completion tool should be exposed"
                    );
                    assert_eq!(request.tools[0].name, "submit_result");
                    let n = self
                        .attempts
                        .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    let call = ToolCall {
                        id: format!("call_{n}"),
                        name: "submit_result".into(),
                        arguments: json!({"status":"completed","summary":"claimed",
                        "findings":[],"evidence_refs":[],"unresolved":[]}),
                    };
                    let calls = match self.kind {
                        RejectedChildKind::MultipleCalls => vec![
                            call.clone(),
                            ToolCall {
                                id: format!("extra_{n}"),
                                ..call
                            },
                        ],
                        RejectedChildKind::UnknownFunction => vec![ToolCall {
                            name: "unfrozen_plugin".into(),
                            ..call
                        }],
                        RejectedChildKind::FakeEvidence => vec![ToolCall {
                            arguments: json!({"status":"completed","summary":"claimed",
                            "findings":[],"evidence_refs":["forged_artifact"],"unresolved":[]}),
                            ..call
                        }],
                    };
                    BackgroundTurn::ToolCalls(calls)
                }
            };
        Ok(BackgroundTurnResult { turn, usage: None })
    }
}

#[tokio::test]
async fn invalid_native_child_calls_never_create_plugin_steps() {
    for (kind, code) in [
        (RejectedChildKind::MultipleCalls, "tool_call_count"),
        (RejectedChildKind::UnknownFunction, "unknown_operation"),
        (RejectedChildKind::FakeEvidence, "invalid_child_evidence_refs"),
    ] {
        let config = Arc::new(config());
        let db = Arc::new(Database::connect(&config.database).await.unwrap());
        let runtime = BackgroundRuntime::execution_harness(
            db.clone(),
            config,
            Arc::new(AdversarialChildModels {
                kind,
                attempts: std::sync::atomic::AtomicUsize::new(0),
            }),
        )
        .await
        .unwrap();
        enable(&runtime.engine).await;
        let receipt = db
            .submit_job(
                &runtime.engine.config,
                request("Reject unsafe Child function call"),
                &JobRequester::Admin,
                crate::database::jobs::SubmissionIdentity {
                    test_execution_mode: Some("hierarchical_v2"),
                    ..Default::default()
                },
                crate::database::unix_seconds().unwrap(),
                || runtime.engine.runtime_ready(),
            )
            .await
            .unwrap();
        let job = wait_for(&runtime.engine, &receipt.job.id, |job| {
            job.status == "failed"
        })
        .await;
        assert!(
            job.steps.is_empty(),
            "Rejected native calls must not persist steps: {kind:?}"
        );
        let events = db.job_events(&job.id, 0, 100).await.unwrap();
        assert!(
            events
                .iter()
                .any(|e| e.event_type == "agent_decision_rejected"
                    && e.payload["validation_code"] == code),
            "Expected bounded rejection for {kind:?}: {code}; job_error={:?}; runs={:?}; events={:?}",
            job.error_code,
            job.agent_runs
                .iter()
                .map(|r| (&r.role, &r.status, &r.error_code))
                .collect::<Vec<_>>(),
            events
                .iter()
                .map(|e| (&e.event_type, &e.payload))
                .collect::<Vec<_>>()
        );
        assert!(!events.iter().any(|e| e.event_type == "plugin_step_created"));
        runtime.shutdown().await;
    }
}
