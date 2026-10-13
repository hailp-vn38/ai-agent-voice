use super::*;

#[tokio::test]
async fn committed_jobs_replay_before_capacity_and_global_enable_checks() {
    let (_, _, runtime) = harness().await;
    let engine = &runtime.engine;
    let first = engine
        .submit(
            request("  Research public AI  "),
            &JobRequester::Admin,
            Some("one"),
        )
        .await
        .unwrap();
    assert_eq!(first.job.status, "queued");
    assert!(!first.replayed);
    engine
        .submit(request("Second"), &JobRequester::Admin, Some("two"))
        .await
        .unwrap();
    assert_eq!(
        engine
            .submit(request("Third"), &JobRequester::Admin, None)
            .await
            .unwrap_err(),
        JobError::QueueFull
    );
    let replay = engine
        .submit(
            request("Research public AI"),
            &JobRequester::Admin,
            Some("one"),
        )
        .await
        .unwrap();
    assert_eq!(replay.job.id, first.job.id);
    assert!(replay.replayed);
    assert_eq!(
        engine
            .submit(request("Different"), &JobRequester::Admin, Some("one"))
            .await
            .unwrap_err(),
        JobError::IdempotencyConflict
    );
    let revision = engine.settings().await.unwrap().revision;
    engine
        .update_settings(
            BackgroundSettingsUpdate {
                enabled: false,
                orchestrator_llm_provider: None,
                worker_llm_provider: None,
            },
            revision,
        )
        .await
        .unwrap();
    assert!(
        engine
            .submit(
                request("Research public AI"),
                &JobRequester::Admin,
                Some("one")
            )
            .await
            .unwrap()
            .replayed
    );
    runtime.shutdown().await;
}

#[tokio::test]
async fn concurrent_queue_admission_never_exceeds_the_database_limit() {
    let (_, _, runtime) = harness().await;
    let mut tasks = Vec::new();
    for i in 0..10 {
        let engine = runtime.engine.clone();
        tasks.push(tokio::spawn(async move {
            engine
                .submit(request(&format!("Request {i}")), &JobRequester::Admin, None)
                .await
        }));
    }
    let mut admitted = 0;
    for task in tasks {
        match task.await.unwrap() {
            Ok(_) => admitted += 1,
            Err(e) => assert_eq!(e, JobError::QueueFull),
        }
    }
    assert_eq!(admitted, 2);
    assert_eq!(runtime.engine.list(1, 30, None, None).await.unwrap().1, 2);
    runtime.shutdown().await;
}

#[tokio::test]
async fn voice_scope_is_non_reusable_and_reminder_permission_is_independent() {
    let (db, _, runtime) = harness().await;
    let caller = voice(&db).await;
    let job = runtime
        .engine
        .submit(request("Research"), &caller, Some("scope-key"))
        .await
        .unwrap()
        .job;
    assert_eq!(job.plugin_choices, json!([]));
    // Recreate the same public identities within the same timestamp: row incarnations change.
    sqlx::query("DELETE FROM devices WHERE device_id='voice-device'")
        .execute(db.pool())
        .await
        .unwrap();
    sqlx::query("DELETE FROM agents WHERE key='voice-agent'")
        .execute(db.pool())
        .await
        .unwrap();
    let new = voice(&db).await;
    assert_eq!(
        runtime
            .engine
            .submit(request("Another"), &caller, None)
            .await
            .unwrap_err(),
        JobError::AuthorityRevoked
    );
    let second = runtime
        .engine
        .submit(request("Research"), &new, Some("scope-key"))
        .await
        .unwrap()
        .job;
    assert_ne!(job.id, second.id);
    assert_ne!(job.requester_scope, second.requester_scope);
    assert_eq!(
        runtime.engine.get(&job.id).await.unwrap().request.goal,
        "Research"
    );
    runtime.shutdown().await;
}

#[tokio::test]
async fn settings_cas_provider_sources_and_frozen_job_contract() {
    let (db, _, runtime) = harness().await;
    let before = runtime.engine.settings().await.unwrap();
    assert_eq!(before.llm_providers[0].r#ref, "config:llm:planner");
    let first = runtime
        .engine
        .submit(request("Pinned research"), &JobRequester::Admin, None)
        .await
        .unwrap()
        .job;
    assert!(
        !serde_json::to_string(&first)
            .unwrap()
            .contains("BACKGROUND_SECRET_SENTINEL")
    );
    let bad = BackgroundSettingsUpdate {
        enabled: false,
        orchestrator_llm_provider: Some("config:tts:planner".into()),
        worker_llm_provider: None,
    };
    assert_eq!(
        runtime
            .engine
            .update_settings(bad, before.revision)
            .await
            .unwrap_err(),
        JobError::InvalidArguments
    );
    let valid = before.settings;
    runtime
        .engine
        .update_settings(valid.clone(), before.revision)
        .await
        .unwrap();
    assert_eq!(
        runtime
            .engine
            .update_settings(valid, before.revision)
            .await
            .unwrap_err(),
        JobError::RevisionConflict
    );
    // DB enforces immutable request/snapshot even if a future executor has a bug.
    assert!(
        sqlx::query("UPDATE jobs SET plugin_choices_json='[{}]' WHERE id=?")
            .bind(&first.id)
            .execute(db.pool())
            .await
            .is_err()
    );
    let mut input = request("Impossible crawler");
    input.required_capabilities = vec!["social.search".into()];
    assert_eq!(
        runtime
            .engine
            .submit(input, &JobRequester::Admin, None)
            .await
            .unwrap_err(),
        JobError::CapabilityUnavailable
    );
    runtime.shutdown().await;
}
