use super::*;
use crate::providers::llm::LlmRequest;
use crate::services::background::models::JobModels;
use tokio_util::sync::CancellationToken;

pub(super) struct ScriptedModels;
#[async_trait::async_trait]
impl JobModels for ScriptedModels {
    async fn complete(
        &self,
        _: &str,
        _: &ProviderPin,
        request: LlmRequest,
        _: i64,
        _: CancellationToken,
    ) -> Result<String, JobError> {
        if request.messages.iter().any(|message| matches!(message, crate::providers::llm::ChatMessage::System { content } if content.contains("Orchestrator"))) {
            Ok(json!({"steps":[{"kind":"llm","step_key":"summary","instruction":"Summarize the goal"}]}).to_string())
        } else {
            Ok(json!({"summary":"Checked worker result"}).to_string())
        }
    }
}

#[tokio::test]
async fn job_scheduler_plans_and_runs_worker_after_lost_wake() {
    let config = Arc::new(config());
    let db = Arc::new(Database::connect(&config.database).await.unwrap());
    let runtime =
        BackgroundRuntime::execution_harness(db.clone(), config, Arc::new(ScriptedModels))
            .await
            .unwrap();
    enable(&runtime.engine).await;
    let receipt = db
        .submit_job(
            &runtime.engine.config,
            request("Summarize public research"),
            &JobRequester::Admin,
            crate::database::jobs::SubmissionIdentity {
                key: Some("dag-once"),
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
    assert_eq!(job.steps.len(), 1);
    assert_eq!(job.steps[0].status, "succeeded");
    assert_eq!(
        job.result.as_ref().unwrap()["coverage_basis"],
        "goal_only_unverified"
    );
    assert_eq!(
        job.result.as_ref().unwrap()["steps"]["summary"]["summary"],
        "Checked worker result"
    );
    assert!(job.scheduler_seen_at.is_some());
    assert_eq!(job.budget_used["planner_calls"], 1);
    assert_eq!(job.budget_used["worker_attempts"], 1);
    assert_eq!(job.budget_used["steps"]["summary"]["llm_calls"], 1);
    assert_eq!(job.steps[0].instruction, "Summarize the goal");
    assert!(db.pool().acquire().await.is_ok());
    runtime.shutdown().await;
}

struct ProposedModels {
    plan: Value,
    worker: Value,
}
#[async_trait::async_trait]
impl JobModels for ProposedModels {
    async fn complete(
        &self,
        _: &str,
        _: &ProviderPin,
        request: LlmRequest,
        _: i64,
        _: CancellationToken,
    ) -> Result<String, JobError> {
        let planner=request.messages.iter().any(|message| matches!(message,crate::providers::llm::ChatMessage::System {content} if content.contains("Orchestrator")));
        Ok(if planner { &self.plan } else { &self.worker }.to_string())
    }
}
fn proposal_step(key: &str) -> Value {
    json!({"kind":"llm","step_key":key,"instruction":"Summarize"})
}
fn step(key: &str) -> Value {
    json!({"step_key":key,"instruction":"Summarize","output_schema":{"type":"object","properties":{"summary":{"type":"string"}},"required":["summary"],"additionalProperties":false}})
}

#[tokio::test]
async fn scheduler_rejects_invalid_dags_without_dispatching_worker() {
    let mut cycle_a = proposal_step("a");
    cycle_a["dependencies"] = json!(["b"]);
    let mut cycle_b = proposal_step("b");
    cycle_b["dependencies"] = json!(["a"]);
    let mut missing = proposal_step("a");
    missing["input_refs"] = json!(["missing"]);
    let mut unknown = proposal_step("a");
    unknown["plugin_id"] = json!("later-plugin");
    unknown["operation"] = json!("read");
    for proposal in [
        json!({"steps":[cycle_a,cycle_b]}),
        json!({"steps":[missing]}),
        json!({"steps":[unknown,proposal_step("final")]}),
        json!({"steps":[proposal_step("a")],"document_refs":[]}),
    ] {
        let expected_calls = if proposal.get("document_refs").is_some() {
            1
        } else {
            3
        };
        let config = Arc::new(config());
        let db = Arc::new(Database::connect(&config.database).await.unwrap());
        let runtime = BackgroundRuntime::execution_harness(
            db,
            config,
            Arc::new(ProposedModels {
                plan: proposal,
                worker: json!({"summary":"must not run"}),
            }),
        )
        .await
        .unwrap();
        enable(&runtime.engine).await;
        let job = runtime
            .engine
            .submit(request("Invalid DAG"), &JobRequester::Admin, None)
            .await
            .unwrap()
            .job;
        let terminal = wait_for(&runtime.engine, &job.id, |job| job.status == "failed").await;
        assert_eq!(terminal.error_code.as_deref(), Some("invalid_plan"));
        assert!(terminal.steps.is_empty());
        assert_eq!(terminal.budget_used["planner_calls"], expected_calls);
        assert!(terminal.budget_used.get("worker_attempts").is_none());
        runtime.shutdown().await;
    }
}

#[tokio::test]
async fn worker_schema_failure_skips_descendants_and_contract_stays_immutable() {
    let first = proposal_step("first");
    let mut last = proposal_step("last");
    last["dependencies"] = json!(["first"]);
    let config = Arc::new(config());
    let db = Arc::new(Database::connect(&config.database).await.unwrap());
    let runtime = BackgroundRuntime::execution_harness(
        db.clone(),
        config,
        Arc::new(ProposedModels {
            plan: json!({"steps":[first,last]}),
            worker: json!({"unexpected":true}),
        }),
    )
    .await
    .unwrap();
    enable(&runtime.engine).await;
    let job = runtime
        .engine
        .submit(request("Schema bounded work"), &JobRequester::Admin, None)
        .await
        .unwrap()
        .job;
    let terminal = wait_for(&runtime.engine, &job.id, |job| job.status == "failed").await;
    assert_eq!(terminal.steps[0].status, "failed");
    assert_eq!(terminal.steps[1].status, "skipped");
    assert!(
        sqlx::query(
            "UPDATE job_steps SET arguments_json='{}',instruction='changed' WHERE job_id=?"
        )
        .bind(&job.id)
        .execute(db.pool())
        .await
        .is_err()
    );
    runtime.shutdown().await;
}

struct WaitingModels {
    started: tokio::sync::Notify,
}
struct DeadlineModels;
#[async_trait::async_trait]
impl JobModels for DeadlineModels {
    async fn complete(
        &self,
        _: &str,
        _: &ProviderPin,
        request: LlmRequest,
        deadline: i64,
        _: CancellationToken,
    ) -> Result<String, JobError> {
        if request.messages.iter().any(|message|matches!(message,crate::providers::llm::ChatMessage::System {content} if content.contains("Orchestrator"))) {return Ok(json!({"steps":[proposal_step("worker")]}).to_string());}
        tokio::time::sleep(
            crate::services::background::models::remaining(deadline).unwrap_or_default()
                + std::time::Duration::from_millis(20),
        )
        .await;
        Err(JobError::DeadlineExceeded)
    }
}
#[tokio::test]
async fn worker_absolute_deadline_is_failed_deadline_exceeded() {
    let mut config = config();
    config.background.limits.job_deadline_seconds = 1;
    config.background.limits.step_timeout_seconds = 1;
    let config = Arc::new(config);
    let db = Arc::new(Database::connect(&config.database).await.unwrap());
    let runtime = BackgroundRuntime::execution_harness(db, config, Arc::new(DeadlineModels))
        .await
        .unwrap();
    enable(&runtime.engine).await;
    let job = runtime
        .engine
        .submit(request("Deadline worker"), &JobRequester::Admin, None)
        .await
        .unwrap()
        .job;
    let terminal = wait_for(&runtime.engine, &job.id, |job| job.status == "failed").await;
    assert_eq!(terminal.error_code.as_deref(), Some("deadline_exceeded"));
    runtime.shutdown().await;
}
#[async_trait::async_trait]
impl JobModels for WaitingModels {
    async fn complete(
        &self,
        _: &str,
        _: &ProviderPin,
        _: LlmRequest,
        _: i64,
        cancellation: CancellationToken,
    ) -> Result<String, JobError> {
        self.started.notify_one();
        cancellation.cancelled().await;
        Err(JobError::Cancelled)
    }
}
#[tokio::test]
async fn cancellation_terminalizes_planning_and_retry_creates_a_new_linked_job() {
    let config = Arc::new(config());
    let db = Arc::new(Database::connect(&config.database).await.unwrap());
    let models = Arc::new(WaitingModels {
        started: Default::default(),
    });
    let runtime = BackgroundRuntime::execution_harness(db, config, models.clone())
        .await
        .unwrap();
    enable(&runtime.engine).await;
    let old = runtime
        .engine
        .submit(request("Cancel planning"), &JobRequester::Admin, None)
        .await
        .unwrap()
        .job;
    models.started.notified().await;
    assert_eq!(
        runtime.engine.cancel(&old.id).await.unwrap().status,
        "cancelling"
    );
    wait_for(&runtime.engine, &old.id, |job| job.status == "cancelled").await;
    assert_eq!(
        runtime
            .engine
            .retry(&old.id, false, None)
            .await
            .unwrap_err(),
        JobError::InvalidArguments
    );
    let retry = runtime
        .engine
        .retry(&old.id, true, Some("retry-once"))
        .await
        .unwrap();
    assert_ne!(retry.job.id, old.id);
    assert_eq!(retry.job.retry_of_job_id.as_deref(), Some(old.id.as_str()));
    let replay = runtime
        .engine
        .retry(&old.id, true, Some("retry-once"))
        .await
        .unwrap();
    assert_eq!(replay.job.id, retry.job.id);
    assert!(replay.replayed);
    let settings = runtime.engine.settings().await.unwrap();
    runtime
        .engine
        .update_settings(
            BackgroundSettingsUpdate {
                enabled: false,
                ..settings.settings
            },
            settings.revision,
        )
        .await
        .unwrap();
    let disabled_replay = runtime
        .engine
        .retry(&old.id, true, Some("retry-once"))
        .await
        .unwrap();
    assert_eq!(disabled_replay.job.id, retry.job.id);
    assert_eq!(
        runtime.engine.get(&old.id).await.unwrap().status,
        "cancelled"
    );
    runtime.shutdown().await;
}

#[tokio::test]
async fn revocation_requests_stop_of_active_planning_and_keeps_typed_reason() {
    let config = Arc::new(config());
    let db = Arc::new(Database::connect(&config.database).await.unwrap());
    let models = Arc::new(WaitingModels {
        started: Default::default(),
    });
    let runtime = BackgroundRuntime::execution_harness(db.clone(), config, models.clone())
        .await
        .unwrap();
    enable(&runtime.engine).await;
    let requester = voice(&db).await;
    let job = runtime
        .engine
        .submit(request("Revocation during planner"), &requester, None)
        .await
        .unwrap()
        .job;
    models.started.notified().await;
    let publication = db.tool_security.publication.write().await;
    sqlx::query("UPDATE devices SET enabled=0 WHERE device_id='voice-device'")
        .execute(db.pool())
        .await
        .unwrap();
    drop(publication);
    let terminal = wait_for(&runtime.engine, &job.id, |job| job.status == "failed").await;
    assert_eq!(terminal.error_code.as_deref(), Some("authority_revoked"));
    runtime.shutdown().await;
}

#[tokio::test]
async fn quiescent_replan_preserves_step_contracts_and_rejects_changed_arguments() {
    use crate::services::background::plan::JobPlan;
    let (db, _, runtime) = harness().await;
    let job = runtime
        .engine
        .submit(
            request("Replan at acknowledged boundary"),
            &JobRequester::Admin,
            None,
        )
        .await
        .unwrap()
        .job;
    assert!(db.claim_job(&job.id, 2).await.unwrap());
    let initial = JobPlan::parse(&json!({"steps":[step("one")]}).to_string(), &job).unwrap();
    db.publish_job_plan(&job.id, &initial, None).await.unwrap();
    let mut next = initial.clone();
    next.steps
        .push(serde_json::from_value(step("two")).unwrap());
    assert_eq!(
        db.publish_job_plan(&job.id, &next, Some(&initial))
            .await
            .unwrap_err(),
        JobError::InvalidPlan
    );
    db.reserve_plugin_attempt(&job.id, "one", "attempt-one")
        .await
        .unwrap();
    db.checkpoint_llm_step(
        &job.id,
        "one",
        "attempt-one",
        Ok(&json!({"summary":"done"})),
    )
    .await
    .unwrap();
    let mut changed = next.clone();
    changed.steps[0].arguments = json!({"forged":true});
    assert_eq!(
        db.publish_job_plan(&job.id, &changed, Some(&initial))
            .await
            .unwrap_err(),
        JobError::InvalidPlan
    );
    db.publish_job_plan(&job.id, &next, Some(&initial))
        .await
        .unwrap();
    let published = runtime.engine.get(&job.id).await.unwrap();
    assert_eq!(published.plan_revision, 2);
    assert_eq!(published.steps[0].status, "succeeded");
    assert_eq!(published.steps[1].status, "pending");
    runtime.engine.cancel(&job.id).await.unwrap();
    runtime.shutdown().await;
}

struct FailingPlanner {
    calls: std::sync::atomic::AtomicUsize,
    started: tokio::sync::Notify,
}
#[async_trait::async_trait]
impl JobModels for FailingPlanner {
    async fn complete(
        &self,
        _: &str,
        _: &ProviderPin,
        _: LlmRequest,
        _: i64,
        _: CancellationToken,
    ) -> Result<String, JobError> {
        self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        self.started.notify_one();
        Err(JobError::ModelFailed)
    }
}

#[tokio::test]
async fn terminal_checkpoint_retries_without_replaying_work_and_preserves_cancel() {
    for cancel in [false, true] {
        let config = Arc::new(config());
        let db = Arc::new(Database::connect(&config.database).await.unwrap());
        let models = Arc::new(FailingPlanner {
            calls: Default::default(),
            started: Default::default(),
        });
        let runtime = BackgroundRuntime::execution_harness(db.clone(), config, models.clone())
            .await
            .unwrap();
        enable(&runtime.engine).await;
        sqlx::query("CREATE TRIGGER reject_terminal BEFORE UPDATE OF status ON jobs WHEN NEW.status IN ('failed','cancelled') BEGIN SELECT RAISE(ABORT,'temporary terminal write failure'); END")
            .execute(db.pool()).await.unwrap();
        let job = runtime
            .engine
            .submit(
                request("Retry terminal checkpoint"),
                &JobRequester::Admin,
                None,
            )
            .await
            .unwrap()
            .job;
        tokio::time::timeout(std::time::Duration::from_secs(3), models.started.notified())
            .await
            .unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(150)).await;
        assert!(
            runtime
                .engine
                .cancellations
                .lock()
                .unwrap()
                .contains_key(&job.id),
            "failed terminal write must retain its execution owner"
        );
        if cancel {
            runtime.engine.cancel(&job.id).await.unwrap();
        }
        sqlx::query("DROP TRIGGER reject_terminal")
            .execute(db.pool())
            .await
            .unwrap();
        let terminal = wait_for(&runtime.engine, &job.id, |job| {
            job.status == if cancel { "cancelled" } else { "failed" }
        })
        .await;
        assert_eq!(terminal.budget_used["planner_calls"], 1);
        assert_eq!(
            models.calls.load(std::sync::atomic::Ordering::SeqCst),
            1,
            "checkpoint retry must not dispatch the model again"
        );
        runtime.shutdown().await;
    }
}

#[tokio::test]
async fn terminal_checkpoint_failure_does_not_block_shutdown_recovery() {
    let (db, _, runtime) = harness().await;
    let job = runtime
        .engine
        .submit(
            request("Terminal write at shutdown"),
            &JobRequester::Admin,
            None,
        )
        .await
        .unwrap()
        .job;
    assert!(db.claim_job(&job.id, 2).await.unwrap());
    sqlx::query("CREATE TRIGGER reject_terminal BEFORE UPDATE OF status ON jobs WHEN NEW.status='failed' BEGIN SELECT RAISE(ABORT,'terminal write unavailable'); END")
        .execute(db.pool()).await.unwrap();
    runtime
        .engine
        .running
        .store(false, std::sync::atomic::Ordering::Release);
    tokio::time::timeout(
        std::time::Duration::from_secs(1),
        super::super::orchestration::execute(
            runtime.engine.clone(),
            job.id.clone(),
            CancellationToken::new(),
        ),
    )
    .await
    .unwrap();
    assert_eq!(
        runtime.engine.get(&job.id).await.unwrap().status,
        "planning"
    );
    sqlx::query("DROP TRIGGER reject_terminal")
        .execute(db.pool())
        .await
        .unwrap();
    db.recover_jobs(crate::database::unix_seconds().unwrap())
        .await
        .unwrap();
    assert_eq!(runtime.engine.get(&job.id).await.unwrap().status, "failed");
    runtime.shutdown().await;
}

#[tokio::test]
async fn terminal_checkpoint_shutdown_is_bounded_when_publication_is_blocked() {
    let (db, _, runtime) = harness().await;
    let job = runtime
        .engine
        .submit(
            request("Shutdown with blocked publication"),
            &JobRequester::Admin,
            None,
        )
        .await
        .unwrap()
        .job;
    assert!(db.claim_job(&job.id, 2).await.unwrap());
    let publication = db.tool_security.publication.write().await;
    runtime
        .engine
        .running
        .store(false, std::sync::atomic::Ordering::Release);
    tokio::time::timeout(
        std::time::Duration::from_secs(6),
        super::super::orchestration::execute(
            runtime.engine.clone(),
            job.id.clone(),
            CancellationToken::new(),
        ),
    )
    .await
    .unwrap();
    drop(publication);
    db.recover_jobs(crate::database::unix_seconds().unwrap())
        .await
        .unwrap();
    assert_eq!(runtime.engine.get(&job.id).await.unwrap().status, "failed");
    runtime.shutdown().await;
}

struct RepairModels {
    planner_calls: std::sync::atomic::AtomicUsize,
}
#[async_trait::async_trait]
impl JobModels for RepairModels {
    async fn complete(
        &self,
        _: &str,
        _: &ProviderPin,
        request: LlmRequest,
        _: i64,
        _: CancellationToken,
    ) -> Result<String, JobError> {
        let planner = request.messages.iter().any(|message|
            matches!(message,crate::providers::llm::ChatMessage::System {content} if content.contains("Orchestrator")));
        if !planner {
            return Ok(json!({"summary":"repaired successfully"}).to_string());
        }
        let count = self
            .planner_calls
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        if count == 0 {
            Ok("not a JSON proposal".into())
        } else {
            assert!(request.messages.iter().any(|message|
                matches!(message,crate::providers::llm::ChatMessage::User {content} if content.contains("proposal_not_json"))));
            Ok(json!({"steps":[proposal_step("final")]}).to_string())
        }
    }
}
#[tokio::test]
async fn invalid_model_json_is_repaired_before_worker_and_visible_in_timeline() {
    let config = Arc::new(config());
    let db = Arc::new(Database::connect(&config.database).await.unwrap());
    let runtime = BackgroundRuntime::execution_harness(
        db.clone(),
        config,
        Arc::new(RepairModels {
            planner_calls: Default::default(),
        }),
    )
    .await
    .unwrap();
    enable(&runtime.engine).await;
    let job = runtime
        .engine
        .submit(request("Repair malformed JSON"), &JobRequester::Admin, None)
        .await
        .unwrap()
        .job;
    let done = wait_for(&runtime.engine, &job.id, |job| job.status == "succeeded").await;
    assert_eq!(done.budget_used["planner_calls"], 2);
    assert_eq!(done.steps.len(), 1);
    assert_eq!(done.steps[0].status, "succeeded");
    let events = db.job_events(&job.id, 0, 100).await.unwrap();
    let kinds: Vec<_> = events
        .iter()
        .map(|event| event.event_type.as_str())
        .collect();
    assert_eq!(
        kinds,
        vec![
            "llm_run_started",
            "plan_attempt",
            "plan_rejected",
            "llm_run_finished",
            "llm_run_started",
            "plan_attempt",
            "llm_run_finished",
            "plan_accepted",
            "step_started",
            "llm_run_started",
            "llm_run_finished",
            "step_finished",
            "job_finished"
        ]
    );
    let rejected = events
        .iter()
        .find(|event| event.event_type == "plan_rejected")
        .unwrap();
    assert_eq!(rejected.payload["code"], "proposal_not_json");
    runtime.shutdown().await;
}

#[tokio::test]
async fn llm_run_start_and_finish_are_durable_and_correlated_with_job_events() {
    let config = Arc::new(config());
    let db = Arc::new(Database::connect(&config.database).await.unwrap());
    let runtime =
        BackgroundRuntime::execution_harness(db.clone(), config, Arc::new(ScriptedModels))
            .await
            .unwrap();
    enable(&runtime.engine).await;
    let receipt = runtime
        .engine
        .submit(
            request("LLM activity correlation"),
            &JobRequester::Admin,
            None,
        )
        .await
        .unwrap();
    let terminal = wait_for(&runtime.engine, &receipt.job.id, |job| {
        job.status == "succeeded"
    })
    .await;
    assert_eq!(terminal.budget_used["planner_calls"], 1);
    let events = db.job_events(&terminal.id, 0, 100).await.unwrap();
    let starts: Vec<_> = events
        .iter()
        .filter(|e| e.event_type == "llm_run_started")
        .collect();
    let finishes: Vec<_> = events
        .iter()
        .filter(|e| e.event_type == "llm_run_finished")
        .collect();
    assert_eq!(starts.len(), 2); // Planner and Worker, no invented plugin LLM.
    assert_eq!(finishes.len(), starts.len());
    assert!(starts.iter().all(|e| e.payload["provider_ref"].is_string()));
    for started in starts {
        let run_id = started.payload["run_id"].as_str().unwrap();
        let completed = finishes
            .iter()
            .find(|event| event.payload["run_id"] == run_id)
            .unwrap();
        assert!(started.sequence < completed.sequence);
        assert_eq!(completed.payload["status"], "completed");
    }
    let plan_attempt = events
        .iter()
        .find(|e| e.event_type == "plan_attempt")
        .unwrap();
    let planner = events
        .iter()
        .find(|e| e.event_type == "llm_run_started" && e.payload["role"] == "planner")
        .unwrap();
    assert_eq!(plan_attempt.payload["run_id"], planner.payload["run_id"]);
    runtime.shutdown().await;
}

#[tokio::test]
async fn llm_run_failure_retains_provider_pin_and_safe_terminal_code() {
    let config = Arc::new(config());
    let db = Arc::new(Database::connect(&config.database).await.unwrap());
    let runtime = BackgroundRuntime::execution_harness(
        db.clone(),
        config,
        Arc::new(FailingPlanner {
            calls: Default::default(),
            started: Default::default(),
        }),
    )
    .await
    .unwrap();
    enable(&runtime.engine).await;
    let receipt = runtime
        .engine
        .submit(
            request("Provider failure tracing"),
            &JobRequester::Admin,
            None,
        )
        .await
        .unwrap();
    let terminal = wait_for(&runtime.engine, &receipt.job.id, |job| {
        job.status == "failed"
    })
    .await;
    assert_eq!(terminal.error_code.as_deref(), Some("llm_execution_failed"));
    let events = db.job_events(&terminal.id, 0, 100).await.unwrap();
    let start = events
        .iter()
        .find(|e| e.event_type == "llm_run_started")
        .unwrap();
    let finish = events
        .iter()
        .find(|e| e.event_type == "llm_run_finished")
        .unwrap();
    assert_eq!(finish.payload["run_id"], start.payload["run_id"]);
    assert_eq!(finish.payload["error_code"], "llm_execution_failed");
    assert!(start.payload["provider_ref"].is_string());
    assert!(!start.payload.to_string().contains("Authorization"));
    runtime.shutdown().await;
}
