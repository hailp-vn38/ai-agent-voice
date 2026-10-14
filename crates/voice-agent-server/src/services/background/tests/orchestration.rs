use super::super::models::JobModels;
use super::*;
use crate::providers::llm::{BackgroundMode, BackgroundTurnResult, LlmRequest};
use tokio_util::sync::CancellationToken;

struct WaitingModels {
    started: tokio::sync::Notify,
}
struct DeadlineModels;
#[async_trait::async_trait]
impl JobModels for DeadlineModels {
    async fn turn_traced(
        &self,
        _: &crate::services::background::model::LlmRunContext,
        _: &ProviderPin,
        _: LlmRequest,
        _: BackgroundMode,
        deadline: i64,
        _: CancellationToken,
    ) -> Result<BackgroundTurnResult, JobError> {
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
    async fn turn_traced(
        &self,
        _: &crate::services::background::model::LlmRunContext,
        _: &ProviderPin,
        _: LlmRequest,
        _: BackgroundMode,
        _: i64,
        cancellation: CancellationToken,
    ) -> Result<BackgroundTurnResult, JobError> {
        self.started.notify_one();
        cancellation.cancelled().await;
        Err(JobError::Cancelled)
    }
}
#[tokio::test]
async fn cancellation_terminalizes_root_turn_and_retry_creates_a_new_linked_job() {
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
        .submit(request("Cancel Root turn"), &JobRequester::Admin, None)
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
async fn revocation_requests_stop_of_active_root_turn_and_keeps_typed_reason() {
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
        .submit(request("Revocation during Root turn"), &requester, None)
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

struct FailingModels {
    calls: std::sync::atomic::AtomicUsize,
    started: tokio::sync::Notify,
}
#[async_trait::async_trait]
impl JobModels for FailingModels {
    async fn turn_traced(
        &self,
        _: &crate::services::background::model::LlmRunContext,
        _: &ProviderPin,
        _: LlmRequest,
        _: BackgroundMode,
        _: i64,
        _: CancellationToken,
    ) -> Result<BackgroundTurnResult, JobError> {
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
        let models = Arc::new(FailingModels {
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
        assert_eq!(terminal.budget_used["llm_calls"], 1);
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
    assert_eq!(runtime.engine.get(&job.id).await.unwrap().status, "running");
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

#[tokio::test]
async fn llm_run_failure_retains_provider_pin_and_safe_terminal_code() {
    let config = Arc::new(config());
    let db = Arc::new(Database::connect(&config.database).await.unwrap());
    let runtime = BackgroundRuntime::execution_harness(
        db.clone(),
        config,
        Arc::new(FailingModels {
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
