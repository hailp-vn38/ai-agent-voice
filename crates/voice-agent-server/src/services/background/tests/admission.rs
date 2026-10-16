use super::*;
use crate::services::background::plugins::PluginError;

#[tokio::test]
async fn scheduler_checks_frozen_shared_plugin_installation() {
    let (db, _, runtime) = harness().await;
    let manifest: toml::Value = toml::from_str(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../plugins/fixture-research-1.0.0/plugin.toml"
    )))
    .unwrap();
    let installation = "b".repeat(64);
    sqlx::query("INSERT INTO plugin_installations(plugin_id,version,package_digest,installation_digest,status,revision,operations_json,created_at,updated_at) VALUES('fixture-research','1.0.0',?,?,'ready',1,?,0,0)")
        .bind("a".repeat(64))
        .bind(&installation)
        .bind(serde_json::to_string(&manifest).unwrap())
        .execute(db.pool()).await.unwrap();
    let created = runtime
        .engine
        .submit(
            request("Shared runtime eligibility"),
            &JobRequester::Admin,
            None,
        )
        .await
        .unwrap()
        .job;
    assert_eq!(
        created
            .plugin_choices
            .as_array()
            .unwrap()
            .iter()
            .find(|v| v["plugin_id"] == "fixture-research")
            .unwrap()["installation_digest"],
        installation
    );
    let job = db.job_row(&created.id).await.unwrap();
    assert_eq!(
        scheduler::eligible(
            &runtime.engine,
            &job,
            crate::database::unix_seconds().unwrap()
        )
        .await,
        Ok(())
    );

    sqlx::query(
        "UPDATE plugin_installations SET installation_digest=? WHERE plugin_id='fixture-research'",
    )
    .bind("c".repeat(64))
    .execute(db.pool())
    .await
    .unwrap();
    assert_eq!(
        scheduler::eligible(
            &runtime.engine,
            &job,
            crate::database::unix_seconds().unwrap()
        )
        .await,
        Err(JobError::CapabilityUnavailable)
    );
    runtime.shutdown().await;
}

#[tokio::test]
async fn ready_plugin_choices_freeze_at_submit_and_pin_nonterminal_installation() {
    let (db, _, runtime) = harness().await;
    let manifest: toml::Value = toml::from_str(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../plugins/fixture-research-1.0.0/plugin.toml"
    )))
    .unwrap();
    let operations = serde_json::to_string(&manifest).unwrap();
    sqlx::query("INSERT INTO plugin_installations(plugin_id,version,package_digest,installation_digest,status,revision,operations_json,created_at,updated_at) VALUES('fixture-research','1.0.0',? ,?,'ready',1,?,0,0)")
        .bind("a".repeat(64)).bind(format!("sha256:{}", "b".repeat(64))).bind(operations)
        .execute(db.pool()).await.unwrap();
    let mut input = request("Collect fixture posts");
    input.required_capabilities = vec!["fixture.collect".into()];
    input.required_sources = vec!["fixture".into()];
    let created = runtime
        .engine
        .submit(input.clone(), &JobRequester::Admin, None)
        .await
        .unwrap()
        .job;
    assert_eq!(created.plugin_choices.as_array().unwrap().len(), 10);
    assert_eq!(created.plugin_choices[0]["plugin_id"], "core");
    assert_eq!(
        created
            .plugin_choices
            .as_array()
            .unwrap()
            .iter()
            .filter(|choice| choice["plugin_id"] == "fixture-research")
            .count(),
        2
    );
    assert_eq!(
        runtime
            .engine
            .plugins
            .uninstall("fixture-research", 1)
            .await
            .unwrap_err(),
        PluginError::InUse
    );
    // A pinned Job must reject installation replacement before any filesystem publication.
    let mut published = false;
    let replacement = db
        .publish_plugin_installation(
            "fixture-research",
            "2.0.0",
            &"c".repeat(64),
            &"d".repeat(64),
            "{}",
            || {
                published = true;
                async { Ok(()) }
            },
        )
        .await;
    assert_eq!(replacement.err(), Some(PluginError::InUse));
    assert!(!published);
    runtime
        .engine
        .plugins
        .disable("fixture-research", 1)
        .await
        .unwrap();
    assert_eq!(
        runtime
            .engine
            .submit(input, &JobRequester::Admin, None)
            .await
            .unwrap_err(),
        JobError::CapabilityUnavailable
    );
    assert_eq!(
        runtime
            .engine
            .get(&created.id)
            .await
            .unwrap()
            .plugin_choices,
        created.plugin_choices
    );
    runtime.shutdown().await;
}

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
    assert_eq!(
        job.plugin_choices,
        json!(crate::tools::core::frozen_choices()
            .into_iter()
            .filter(|choice| choice["operation"] != "tts.synthesize")
            .collect::<Vec<_>>())
    );
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
