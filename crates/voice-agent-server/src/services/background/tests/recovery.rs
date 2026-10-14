use super::*;

#[tokio::test]
async fn expired_terminal_job_releases_artifacts_but_keeps_recent_history() {
    let (db, _, runtime) = harness().await;
    let old = runtime
        .engine
        .submit(request("old result"), &JobRequester::Admin, None)
        .await
        .unwrap()
        .job;
    let recent = runtime
        .engine
        .submit(request("recent"), &JobRequester::Admin, None)
        .await
        .unwrap()
        .job;
    runtime.shutdown().await;
    let now = crate::database::unix_seconds().unwrap();
    let old_time = now - 31 * 24 * 60 * 60;
    sqlx::query("UPDATE jobs SET status='failed',updated_at=? WHERE id=?")
        .bind(old_time)
        .bind(&old.id)
        .execute(db.pool())
        .await
        .unwrap();
    let artifact_id = uuid::Uuid::new_v4().to_string();
    let dir = crate::services::background::artifacts::root(&db)
        .unwrap()
        .join(&old.id);
    tokio::fs::create_dir_all(&dir).await.unwrap();
    tokio::fs::write(dir.join(&artifact_id), b"result")
        .await
        .unwrap();
    sqlx::query("INSERT INTO job_artifacts(id,job_id,relative_path,media_type,size_bytes,sha256,created_at) VALUES(?,?,?,'text/plain',6,'ignored',?)")
        .bind(&artifact_id).bind(&old.id).bind(format!("{}/{artifact_id}",old.id)).bind(old_time)
        .execute(db.pool()).await.unwrap();
    sqlx::query("UPDATE jobs SET status='failed',updated_at=? WHERE id=?")
        .bind(now)
        .bind(&recent.id)
        .execute(db.pool())
        .await
        .unwrap();
    assert_eq!(
        crate::services::background::artifacts::expire_terminal(&db, now)
            .await
            .unwrap(),
        1
    );
    assert!(runtime.engine.get(&old.id).await.is_err());
    assert!(runtime.engine.get(&recent.id).await.is_ok());
    assert!(!dir.join(&artifact_id).exists());
}

#[tokio::test]
async fn terminal_retention_drains_multiple_bounded_batches() {
    let (db, _, runtime) = harness().await;
    let seed = runtime
        .engine
        .submit(request("retention seed"), &JobRequester::Admin, None)
        .await
        .unwrap()
        .job;
    runtime.shutdown().await;
    let now = crate::database::unix_seconds().unwrap();
    let expired = now - 31 * 24 * 60 * 60;
    sqlx::query("UPDATE jobs SET status='failed',updated_at=? WHERE id=?")
        .bind(expired)
        .bind(&seed.id)
        .execute(db.pool())
        .await
        .unwrap();
    for _ in 0..100 {
        sqlx::query("INSERT INTO jobs(id,requester_scope,request_json,request_fingerprint,snapshot_json,plugin_choices_json,settings_revision,budget_limit_json,status,deadline_at,created_at,updated_at) SELECT ?,requester_scope,request_json,request_fingerprint,snapshot_json,plugin_choices_json,settings_revision,budget_limit_json,status,deadline_at,created_at,updated_at FROM jobs WHERE id=?")
            .bind(uuid::Uuid::new_v4().to_string()).bind(&seed.id).execute(db.pool()).await.unwrap();
    }
    assert_eq!(
        crate::services::background::artifacts::expire_terminal(&db, now)
            .await
            .unwrap(),
        100
    );
    assert_eq!(
        crate::services::background::artifacts::expire_terminal(&db, now)
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        crate::services::background::artifacts::expire_terminal(&db, now)
            .await
            .unwrap(),
        0
    );
}

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
    let worker: Option<String> = sqlx::query_scalar(
        "SELECT worker_instance_id FROM job_steps WHERE job_id=? AND step_key='fetch'",
    )
    .bind(&interrupted.id)
    .fetch_one(reopened.pool())
    .await
    .unwrap();
    assert!(worker.is_none());
    let admitted = restarted
        .engine
        .submit(
            request("Runner owns previous attempts"),
            &JobRequester::Admin,
            None,
        )
        .await
        .unwrap();
    assert_eq!(admitted.job.status, "queued");
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

#[tokio::test]
async fn retention_preserves_uncommitted_artifacts_of_other_jobs() {
    let (db, _, runtime) = harness().await;
    let old = runtime.engine.submit(request("Old retained Job"), &JobRequester::Admin, None)
        .await.unwrap().job;
    let active = runtime.engine.submit(request("Publishing artifact"), &JobRequester::Admin, None)
        .await.unwrap().job;
    runtime.shutdown().await;
    let now = crate::database::unix_seconds().unwrap();
    sqlx::query("UPDATE jobs SET status='failed',updated_at=? WHERE id=?")
        .bind(now - 31 * 24 * 60 * 60).bind(&old.id).execute(db.pool()).await.unwrap();
    sqlx::query("UPDATE jobs SET status='running',updated_at=? WHERE id=?")
        .bind(now).bind(&active.id).execute(db.pool()).await.unwrap();
    let artifact_dir = crate::services::background::artifacts::root(&db).unwrap().join(&active.id);
    tokio::fs::create_dir_all(&artifact_dir).await.unwrap();
    let pending = artifact_dir.join(uuid::Uuid::new_v4().to_string());
    tokio::fs::write(&pending, b"before metadata commit").await.unwrap();
    assert_eq!(crate::services::background::artifacts::expire_terminal(&db, now).await.unwrap(), 1);
    assert!(pending.exists(), "retention must not erase a publishing Job's uncommitted artifact");
    assert!(runtime.engine.get(&old.id).await.is_err());
}
