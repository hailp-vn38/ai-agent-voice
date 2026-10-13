use super::*;

#[tokio::test]
async fn periodic_scan_observes_lost_wake_and_terminalizes_expired_or_revoked_work() {
    let (db, _, runtime) = harness().await;
    // Suppress submit's Notify by going through the same database admission seam.
    let job = db
        .submit_job(
            &runtime.engine.config,
            request("Lost wake"),
            &JobRequester::Admin,
            Default::default(),
            crate::database::unix_seconds().unwrap(),
            || true,
        )
        .await
        .unwrap()
        .job;
    let seen = wait_for(&runtime.engine, &job.id, |job| {
        job.scheduler_seen_at.is_some()
    })
    .await;
    assert_eq!(seen.status, "queued");
    let caller = voice(&db).await;
    let revoked = runtime
        .engine
        .submit(request("Revoked"), &caller, None)
        .await
        .unwrap()
        .job;
    let _publication = db.tool_security.publication.write().await;
    sqlx::query("UPDATE devices SET enabled=0 WHERE device_id='voice-device'")
        .execute(db.pool())
        .await
        .unwrap();
    drop(_publication);
    let failed = wait_for(&runtime.engine, &revoked.id, |job| job.status == "failed").await;
    assert_eq!(failed.error_code.as_deref(), Some("authority_revoked"));
    runtime.shutdown().await;
    // The absolute deadline was snapshotted at admission, including queue waiting.
    let (_, config, runtime) = harness().await;
    let expired = runtime
        .engine
        .database
        .submit_job(
            &config,
            request("Expired"),
            &JobRequester::Admin,
            Default::default(),
            crate::database::unix_seconds().unwrap() - 1801,
            || true,
        )
        .await
        .unwrap()
        .job;
    let failed = wait_for(&runtime.engine, &expired.id, |job| job.status == "failed").await;
    assert_eq!(failed.error_code.as_deref(), Some("deadline_exceeded"));
    runtime.shutdown().await;
}

#[tokio::test]
async fn restart_preserves_queued_pins_and_recovers_cancelling_without_erasing_uncertainty() {
    let (db, config, runtime) = harness().await;
    let queued = runtime
        .engine
        .submit(request("Resume exact pins"), &JobRequester::Admin, None)
        .await
        .unwrap()
        .job;
    let interrupted = runtime
        .engine
        .submit(request("Possibly started"), &JobRequester::Admin, None)
        .await
        .unwrap()
        .job;
    runtime.shutdown().await;
    sqlx::query(
        "UPDATE jobs SET status='cancelling',budget_used_json='{\"worker_launches\":1}' WHERE id=?",
    )
    .bind(&interrupted.id)
    .execute(db.pool())
    .await
    .unwrap();
    sqlx::query("INSERT INTO job_steps(job_id,step_key,plan_revision,status,attempt,attempt_id,attempt_state,worker_instance_id,created_at,updated_at) VALUES(?,'fetch',1,'running',1,'attempt-1','start_requested','owned-worker',0,0)").bind(&interrupted.id).execute(db.pool()).await.unwrap();
    let reopened = Arc::new(Database::connect(&config.database).await.unwrap());
    let restarted = BackgroundRuntime::foundation_harness(reopened.clone(), config.clone())
        .await
        .unwrap();
    assert_eq!(
        restarted
            .engine
            .get(&queued.id)
            .await
            .unwrap()
            .snapshot
            .orchestrator,
        queued.snapshot.orchestrator
    );
    let recovered = restarted.engine.get(&interrupted.id).await.unwrap();
    assert_eq!(recovered.status, "cancelled");
    assert_eq!(
        recovered.steps[0].attempt_state.as_deref(),
        Some("unknown_outcome")
    );
    assert_eq!(
        recovered.steps[0].error_code.as_deref(),
        Some("unknown_outcome")
    );
    reopened
        .recover_jobs(crate::database::unix_seconds().unwrap())
        .await
        .unwrap();
    assert_eq!(
        restarted.engine.get(&interrupted.id).await.unwrap().steps,
        recovered.steps
    );
    let used: String = sqlx::query_scalar("SELECT budget_used_json FROM jobs WHERE id=?")
        .bind(&interrupted.id)
        .fetch_one(reopened.pool())
        .await
        .unwrap();
    assert_eq!(used, "{\"worker_launches\":1}");
    assert_eq!(
        restarted
            .engine
            .submit(
                request("Blocked by unreaped worker"),
                &JobRequester::Admin,
                None
            )
            .await
            .unwrap_err(),
        JobError::RuntimeUnavailable
    );
    restarted.shutdown().await;
}

#[tokio::test]
async fn restart_fails_running_work_and_configuration_changes_without_replaying() {
    let (db, config, runtime) = harness().await;
    let running = runtime
        .engine
        .submit(request("Interrupted planner"), &JobRequester::Admin, None)
        .await
        .unwrap()
        .job;
    let queued = runtime
        .engine
        .submit(request("Pinned provider"), &JobRequester::Admin, None)
        .await
        .unwrap()
        .job;
    runtime.shutdown().await;
    sqlx::query("UPDATE jobs SET status='planning' WHERE id=?")
        .bind(&running.id)
        .execute(db.pool())
        .await
        .unwrap();
    let mut changed = (*config).clone();
    if let crate::config::LlmInstanceConfig::Openai(llm) =
        changed.providers.llm.instances.get_mut("planner").unwrap()
    {
        llm.model = "new-model".into();
    }
    let restarted = BackgroundRuntime::foundation_harness(db.clone(), Arc::new(changed))
        .await
        .unwrap();
    assert_eq!(
        restarted
            .engine
            .get(&running.id)
            .await
            .unwrap()
            .error_code
            .as_deref(),
        Some("interrupted")
    );
    assert_eq!(
        restarted
            .engine
            .get(&queued.id)
            .await
            .unwrap()
            .error_code
            .as_deref(),
        Some("configuration_changed")
    );
    assert!(
        sqlx::query("UPDATE jobs SET status='queued' WHERE id=?")
            .bind(&running.id)
            .execute(db.pool())
            .await
            .is_err()
    );
    restarted.shutdown().await;
}
