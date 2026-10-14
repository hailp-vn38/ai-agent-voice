use super::*;
use crate::services::background::plugins::FrozenChoice;
use std::io::Read;

#[tokio::test]
async fn expired_attempt_cannot_publish_success_or_artifacts() {
    let mut config = config();
    config.background.limits.job_deadline_seconds = 1;
    config.background.limits.step_timeout_seconds = 1;
    let config = Arc::new(config);
    let db = Arc::new(Database::connect(&config.database).await.unwrap());
    let runtime = BackgroundRuntime::foundation_harness(db.clone(), config)
        .await
        .unwrap();
    enable(&runtime.engine).await;
    let job = runtime
        .engine
        .submit(request("Expires during worker"), &JobRequester::Admin, None)
        .await
        .unwrap()
        .job;
    sqlx::query("UPDATE jobs SET status='running',plan_revision=1 WHERE id=?")
        .bind(&job.id)
        .execute(db.pool())
        .await
        .unwrap();
    sqlx::query("INSERT INTO job_steps(job_id,step_key,plan_revision,status,attempt,attempt_id,attempt_state,created_at,updated_at) VALUES(?,'one',1,'running',1,'attempt-1','started',0,0)")
        .bind(&job.id).execute(db.pool()).await.unwrap();
    tokio::time::sleep(std::time::Duration::from_secs(2)).await;
    assert_eq!(
        db.finish_plugin_success(
            &job.id,
            "one",
            "attempt-1",
            &json!({"ok":true}),
            &[],
            (1024, true)
        )
        .await
        .unwrap_err(),
        JobError::DeadlineExceeded
    );
    db.finish_plugin_failure(
        &job.id,
        "one",
        "attempt-1",
        "deadline_exceeded",
        false,
        true,
    )
    .await
    .unwrap();
    let final_job = runtime.engine.get(&job.id).await.unwrap();
    assert_eq!(final_job.status, "failed");
    assert!(final_job.artifacts.is_empty());
    runtime.shutdown().await;
}

#[tokio::test]
async fn one_persisted_plugin_attempt_cannot_replay_after_terminal_checkpoint() {
    let (db, _, runtime) = harness().await;
    let zip = include_bytes!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../plugins/fixture-research-1.0.0.zip"
    ));
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(zip)).unwrap();
    let mut manifest = String::new();
    archive
        .by_name("plugin.toml")
        .unwrap()
        .read_to_string(&mut manifest)
        .unwrap();
    let json = serde_json::to_string(&toml::from_str::<toml::Value>(&manifest).unwrap()).unwrap();
    sqlx::query("INSERT INTO plugin_installations(plugin_id,version,package_digest,installation_digest,status,revision,operations_json,created_at,updated_at) VALUES('fixture-research','1.0.0',?,?,'ready',1,?,0,0)")
        .bind("a".repeat(64)).bind(format!("sha256:{}", "b".repeat(64)))
        .bind(json).execute(db.pool()).await.unwrap();
    let job = runtime
        .engine
        .submit(request("Fixture attempt"), &JobRequester::Admin, None)
        .await
        .unwrap()
        .job;
    let choice: FrozenChoice = serde_json::from_value(
        job.plugin_choices.as_array().unwrap().iter()
            .find(|v| v["plugin_id"] == "fixture-research" && v["operation"] == "collect_topic")
            .unwrap().clone()
    ).unwrap();
    db.test_plan_one_step(&job.id, "collect", &choice, &json!({"topic":"AI"}))
        .await
        .unwrap();
    db.reserve_plugin_attempt(&job.id, "collect", "attempt-1")
        .await
        .unwrap();
    assert_eq!(
        db.reserve_plugin_attempt(&job.id, "collect", "attempt-2")
            .await
            .unwrap_err(),
        JobError::Terminal
    );
    db.request_plugin_start(&job.id, "collect", "attempt-1", &choice)
        .await
        .unwrap();
    db.mark_plugin_started(&job.id, "collect", "attempt-1")
        .await
        .unwrap();
    db.finish_plugin_success(
        &job.id,
        "collect",
        "attempt-1",
        &json!({"summary":"done"}),
        &[],
        (1024, true),
    )
    .await
    .unwrap();
    assert_eq!(
        runtime.engine.get(&job.id).await.unwrap().status,
        "succeeded"
    );
    assert_eq!(
        db.finish_plugin_failure(&job.id, "collect", "attempt-1", "late", true, true)
            .await
            .unwrap_err(),
        JobError::Terminal
    );
    runtime.shutdown().await;
}

/// Run explicitly on a Linux deployment with the shared Installer and Runner sockets.
/// This is the P2 acceptance gate, not a local unit-test simulation.
#[tokio::test]
#[ignore = "requires deployed shared Installer/Runner and local test volumes"]
async fn fixture_python_runs_through_job_step_and_checked_artifact() {
    let mut config = config();
    config.background.plugin_installer_socket =
        Some(std::env::var("AGENT_P2_INSTALLER_SOCKET").unwrap().into());
    config.background.plugin_runner_socket =
        Some(std::env::var("AGENT_P2_RUNNER_SOCKET").unwrap().into());
    config.background.plugin_installations_dir =
        Some(std::env::var("AGENT_P2_INSTALLATIONS_DIR").unwrap().into());
    config.background.plugin_staging_dir =
        Some(std::env::var("AGENT_P2_STAGING_DIR").unwrap().into());
    config.background.plugin_attempts_dir =
        Some(std::env::var("AGENT_P2_ATTEMPTS_DIR").unwrap().into());
    config.background.limits.step_timeout_seconds = 10;
    let config = Arc::new(config);
    let db = Arc::new(Database::connect(&config.database).await.unwrap());
    let runtime = BackgroundRuntime::foundation_harness(db.clone(), config)
        .await
        .unwrap();
    let zip = include_bytes!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../plugins/fixture-research-1.0.0.zip"
    ));
    let installed = runtime.engine.plugins.install(zip).await.unwrap();
    runtime
        .engine
        .plugins
        .activate(&installed.id, installed.revision)
        .await
        .unwrap();
    enable(&runtime.engine).await;
    let mut input = request("Research fixture AI posts");
    input.required_capabilities = vec!["fixture.collect".into()];
    input.required_sources = vec!["fixture".into()];
    let job = runtime
        .engine
        .submit(input, &JobRequester::Admin, None)
        .await
        .unwrap()
        .job;
    // A valid ZIP with trailing bytes has a new digest but must not publish while pinned.
    let replacement = [zip.as_slice(), b"reinstall-pin-regression"].concat();
    let replacement_digest = {
        use sha2::{Digest, Sha256};
        Sha256::digest(&replacement)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>()
    };
    let replacement_path = std::env::temp_dir()
        .join("plugins/packages/fixture-research/1.0.0")
        .join(replacement_digest);
    assert!(!replacement_path.exists());
    assert_eq!(
        runtime
            .engine
            .plugins
            .install(&replacement)
            .await
            .unwrap_err(),
        crate::services::background::plugins::PluginError::InUse
    );
    assert!(!replacement_path.exists());
    let choice: FrozenChoice = serde_json::from_value(
        job.plugin_choices.as_array().unwrap().iter()
            .find(|v| v["plugin_id"] == "fixture-research" && v["operation"] == "collect_topic")
            .unwrap().clone()
    ).unwrap();
    let args = json!({"topic":"AI", "limit":2});
    db.test_plan_one_step(&job.id, "collect", &choice, &args)
        .await
        .unwrap();
    let result = runtime
        .engine
        .execute_plugin_step(&job.id, "collect", &choice, args)
        .await
        .unwrap();
    assert_eq!(result.status, "succeeded");
    assert_eq!(result.steps[0].attempt_state.as_deref(), Some("finished"));
    assert_eq!(result.artifacts.len(), 2);
    for artifact in result.artifacts {
        let bytes = runtime
            .engine
            .read_artifact(&job.id, &artifact.id)
            .await
            .unwrap();
        assert_eq!(bytes.len(), artifact.size_bytes as usize);
        assert!(!bytes.is_empty());
    }
    let timed = runtime
        .engine
        .submit(
            request("Force worker termination"),
            &JobRequester::Admin,
            None,
        )
        .await
        .unwrap()
        .job;
    let choice: FrozenChoice = serde_json::from_value(
        timed
            .plugin_choices
            .as_array()
            .unwrap()
            .iter()
            .find(|value| value["operation"] == "simulate_failure")
            .unwrap()
            .clone(),
    )
    .unwrap();
    let args = json!({"mode":"sleep","seconds":30});
    db.test_plan_one_step(&timed.id, "sleep", &choice, &args)
        .await
        .unwrap();
    let failed = runtime
        .engine
        .execute_plugin_step(&timed.id, "sleep", &choice, args)
        .await
        .unwrap();
    assert_eq!(failed.status, "failed");
    assert_eq!(
        failed.steps[0].attempt_state.as_deref(),
        Some("unknown_outcome")
    );
    assert_eq!(
        failed.steps[0].error_code.as_deref(),
        Some("worker_timeout")
    );
    assert_eq!(
        runtime
            .engine
            .plugins
            .get("fixture-research")
            .await
            .unwrap()
            .status,
        "ready"
    );
    runtime.shutdown().await;
}

#[tokio::test]
async fn persisted_evidence_refs_are_step_and_child_scoped() {
    // Mirrors a real fetch_url: plugin artifacts exist in SQLite but the
    // Child only receives references to its own succeeded Steps.
    let (db, _config, runtime) = harness().await;
    let job = runtime.engine.submit(
        request("Summarize fetched article"), &JobRequester::Admin, None
    ).await.unwrap().job;
    sqlx::query("UPDATE jobs SET status='running' WHERE id=?")
        .bind(&job.id).execute(db.pool()).await.unwrap();
    for (step,run,status) in [
        ("a-fetch", "child-a", "succeeded"),
        ("b-fetch", "child-b", "succeeded"),
        ("a-failed", "child-a", "failed"),
    ] {
        sqlx::query("INSERT INTO job_steps(job_id,step_key,plan_revision,status,agent_run_id,created_at,updated_at) VALUES(?,?,0,?,?,0,0)")
            .bind(&job.id).bind(step).bind(status).bind(run)
            .execute(db.pool()).await.unwrap();
    }
    for (id,step) in [
        ("artifact-own", "a-fetch"),
        ("artifact-other", "b-fetch"),
        ("artifact-failed", "a-failed"),
    ] {
        sqlx::query("INSERT INTO job_artifacts(id,job_id,step_key,relative_path,media_type,size_bytes,sha256,created_at) VALUES(?,?,?,?,'text/plain',0,'hash',0)")
            .bind(id).bind(&job.id).bind(step)
            .bind(format!("{}/{id}",job.id))
            .execute(db.pool()).await.unwrap();
    }
    assert_eq!(db.job_step_artifact_refs(&job.id,"a-fetch").await.unwrap(),vec!["artifact-own"]);
    assert!(db.job_step_artifact_refs(&job.id,"a-failed").await.unwrap().is_empty());
    assert_eq!(db.job_child_artifact_refs(&job.id,"child-a").await.unwrap(),vec!["artifact-own"]);
    assert_eq!(db.job_child_artifact_refs(&job.id,"child-b").await.unwrap(),vec!["artifact-other"]);
    runtime.shutdown().await;
}


#[tokio::test]
async fn core_only_json_query_executes_without_plugin_installation_or_container() {
    let (db, _, runtime) = harness().await;
    let job = runtime
        .engine
        .submit(request("Query JSON"), &JobRequester::Admin, None)
        .await
        .unwrap()
        .job;
    let raw = job.plugin_choices.as_array().unwrap().iter()
        .find(|v| v["plugin_id"] == "core" && v["operation"] == "data.json_query")
        .unwrap();
    let choice = crate::services::background::plugins::decode_choice(raw).unwrap();
    let args = json!({"json_text":"{\"hello\":{\"answer\":42}}","pointer":"/hello/answer"});
    db.test_plan_one_step(&job.id, "core_query", &choice, &args)
        .await.unwrap();
    let finished = runtime.engine.execute_plugin_step(
        &job.id, "core_query", &choice, args
    ).await.unwrap();
    assert_eq!(finished.steps[0].status, "succeeded");
    assert_eq!(finished.steps[0].result.as_ref().unwrap()["value"], 42);
    assert_eq!(finished.steps[0].attempt_state.as_deref(), Some("finished"));
    runtime.shutdown().await;
}
