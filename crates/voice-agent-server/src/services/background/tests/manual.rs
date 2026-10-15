use super::*;

#[tokio::test]
async fn plugin_only_core_operation_completes_without_background_models_or_agents() {
    let config = Arc::new(config());
    let db = Arc::new(Database::connect(&config.database).await.unwrap());
    let runtime = BackgroundRuntime::foundation_harness(db.clone(), config)
        .await
        .unwrap();
    // Background defaults disabled. Manual run is deliberately independent.
    assert!(!runtime.engine.settings().await.unwrap().settings.enabled);
    let receipt = runtime
        .engine
        .submit_plugin_operation(
            "core.data",
            "data.json_query",
            json!({"json_text":"{\"answer\":42}","pointer":"/answer"}),
        )
        .await
        .unwrap();
    assert_eq!(receipt.job.execution_mode, "plugin_operation_v3");
    assert!(receipt.job.snapshot.orchestrator.reference.is_empty());
    assert!(receipt.job.agent_runs.is_empty());
    let completed = wait_for(&runtime.engine, &receipt.job.id, |job| {
        matches!(job.status.as_str(), "succeeded" | "failed" | "cancelled")
    })
    .await;
    assert_eq!(completed.status, "succeeded", "{:?}", completed.error_code);
    assert_eq!(completed.steps.len(), 1);
    assert_eq!(completed.steps[0].status, "succeeded");
    assert_eq!(completed.result, Some(json!({"value":42})));
    assert!(completed.agent_runs.is_empty());
    let llm_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM job_events WHERE job_id=? AND event_type LIKE 'llm_%'",
    )
    .bind(&receipt.job.id)
    .fetch_one(db.pool())
    .await
    .unwrap();
    assert_eq!(llm_count, 0);
    runtime.shutdown().await;
}

#[tokio::test]
async fn plugin_only_disabled_core_is_denied_before_submission() {
    let (db, _, runtime) = harness().await;
    db.set_core_plugin_enabled("core.data", 1, false)
        .await
        .unwrap();
    let failure = runtime
        .engine
        .submit_plugin_operation(
            "core.data",
            "data.json_query",
            json!({"json_text":"{}","pointer":""}),
        )
        .await
        .unwrap_err();
    assert_eq!(failure, JobError::CorePluginDisabled);
    runtime.shutdown().await;
}

#[tokio::test]
async fn plugin_only_job_pins_config_revision_and_delete_removes_pin() {
    let (db, _, runtime) = harness().await;
    let contract = runtime
        .engine
        .plugins
        .value_settings_contract("core.data")
        .await
        .unwrap();
    db.write_plugin_config(
        "core.data",
        0,
        &contract.schema_digest,
        None,
        &json!({}),
        &json!({}),
    )
    .await
    .unwrap();
    let receipt = runtime
        .engine
        .submit_plugin_operation(
            "core.data",
            "data.json_query",
            json!({"json_text":"{\"x\":1}","pointer":"/x"}),
        )
        .await
        .unwrap();
    let pinned: Option<i64> = sqlx::query_scalar(
        "SELECT revision FROM job_plugin_config_pins WHERE job_id=? AND plugin_key='core.data'",
    )
    .bind(&receipt.job.id)
    .fetch_optional(db.pool())
    .await
    .unwrap();
    assert_eq!(pinned, Some(1));
    db.write_plugin_config(
        "core.data",
        1,
        &contract.schema_digest,
        None,
        &json!({}),
        &json!({}),
    )
    .await
    .unwrap();
    let pinned_after: Option<i64> = sqlx::query_scalar(
        "SELECT revision FROM job_plugin_config_pins WHERE job_id=? AND plugin_key='core.data'",
    )
    .bind(&receipt.job.id)
    .fetch_optional(db.pool())
    .await
    .unwrap();
    assert_eq!(pinned_after, Some(1));
    wait_for(&runtime.engine, &receipt.job.id, |job| {
        matches!(job.status.as_str(), "succeeded" | "failed" | "cancelled")
    })
    .await;
    runtime.engine.delete(&receipt.job.id).await.unwrap();
    let left: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM job_plugin_config_pins WHERE job_id=?")
            .bind(&receipt.job.id)
            .fetch_one(db.pool())
            .await
            .unwrap();
    assert_eq!(left, 0);
    runtime.shutdown().await;
}

#[tokio::test]
async fn repeated_plugin_run_key_reuses_the_first_job_and_rejects_changed_arguments() {
    let (db, _, runtime) = harness().await;
    let key = "same-manual-attempt";
    let args = json!({"json_text":"{\"answer\":1}","pointer":"/answer"});
    let first = runtime
        .engine
        .submit_plugin_operation_with_bindings(
            "core.data",
            "data.json_query",
            args.clone(),
            vec![],
            Some(key),
        )
        .await
        .unwrap();
    assert!(!first.replayed);
    let second = runtime
        .engine
        .submit_plugin_operation_with_bindings(
            "core.data",
            "data.json_query",
            args,
            vec![],
            Some(key),
        )
        .await
        .unwrap();
    assert!(second.replayed);
    assert_eq!(first.job.id, second.job.id);
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM jobs WHERE idempotency_key=?")
        .bind(key)
        .fetch_one(db.pool())
        .await
        .unwrap();
    assert_eq!(count, 1);
    assert_eq!(
        runtime
            .engine
            .submit_plugin_operation_with_bindings(
                "core.data",
                "data.json_query",
                json!({"json_text":"{\"answer\":2}","pointer":"/answer"}),
                vec![],
                Some(key),
            )
            .await
            .unwrap_err(),
        JobError::IdempotencyConflict
    );
    runtime.shutdown().await;
}

#[tokio::test]
async fn stale_settings_schema_is_reconfigurable_without_mutating_old_versions() {
    let (db, _, runtime) = harness().await;
    let original = runtime
        .engine
        .plugins
        .value_settings_contract("core.data")
        .await
        .unwrap();
    db.write_plugin_config(
        "core.data",
        0,
        "old-schema",
        None,
        &json!({"legacy":"value"}),
        &json!({}),
    )
    .await
    .unwrap();
    let view = runtime
        .engine
        .plugins
        .value_settings_view("core.data")
        .await
        .unwrap();
    assert_eq!(view["revision"], 1);
    assert_eq!(view["needs_reconfiguration"], true);
    assert_eq!(view["values"], json!({}));
    let updated = runtime
        .engine
        .plugins
        .save_value_settings("core.data", 1, json!({}))
        .await
        .unwrap();
    assert_eq!(updated["revision"], 2);
    assert_eq!(updated["needs_reconfiguration"], false);
    let old:Option<String> = sqlx::query_scalar(
        "SELECT schema_digest FROM plugin_config_versions WHERE plugin_key='core.data' AND revision=1"
    ).fetch_optional(db.pool()).await.unwrap();
    assert_eq!(old.as_deref(), Some("old-schema"));
    assert_eq!(
        db.plugin_config_version("core.data")
            .await
            .unwrap()
            .unwrap()
            .schema_digest,
        original.schema_digest
    );
    runtime.shutdown().await;
}

#[tokio::test]
async fn plugin_only_runs_in_production_runtime_without_managed_llm_models() {
    let config = Arc::new(config());
    let db = Arc::new(Database::connect(&config.database).await.unwrap());
    let runtime = BackgroundRuntime::open(
        db.clone(),
        config,
        tokio_util::sync::CancellationToken::new(),
    )
    .await
    .unwrap();
    assert!(!runtime.engine.runtime_ready()); // Agent jobs remain unavailable.
    let receipt = runtime
        .engine
        .submit_plugin_operation(
            "core.data",
            "data.json_query",
            json!({"json_text":"{\"answer\":7}","pointer":"/answer"}),
        )
        .await
        .unwrap();
    let completed = wait_for(&runtime.engine, &receipt.job.id, |job| {
        matches!(job.status.as_str(), "succeeded" | "failed" | "cancelled")
    })
    .await;
    assert_eq!(completed.status, "succeeded", "{:?}", completed.error_code);
    assert!(completed.agent_runs.is_empty());
    let violations: Vec<sqlx::sqlite::SqliteRow> = sqlx::query("PRAGMA foreign_key_check")
        .fetch_all(db.pool())
        .await
        .unwrap();
    assert!(
        violations.is_empty(),
        "Manual admission must preserve all SQLite foreign keys"
    );
    runtime.shutdown().await;
}
