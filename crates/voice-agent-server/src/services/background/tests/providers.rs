use super::*;

#[tokio::test]
async fn job_provider_preflight_reloads_stored_credentials_after_idle_eviction() {
    use super::super::models::{JobModels, ManagedModels};
    use crate::{
        database::{
            credentials::CredentialCipher,
            secrets::{SecretResolver, SecretValue},
        },
        lifecycle::AdmissionGate,
        services::provider_runtime::{FactoryMaterializer, ProviderRuntimeManager, RuntimeLimits},
        workers::WorkerSupervisor,
    };

    let config = Arc::new(config());
    let db = Arc::new(Database::connect(&config.database).await.unwrap());
    let cipher = Arc::new(CredentialCipher::new(1, &[7; 32]).unwrap());
    let credential = cipher
        .seal(
            &SecretValue::new("fixture-api-key".into()),
            "provider:worker",
        )
        .unwrap();
    sqlx::query("INSERT INTO providers(key,name,type,adapter,config_json,credential_json,created_at,updated_at) VALUES('worker','Worker','llm','openai',?,?,0,0)")
        .bind(r#"{"base_url":"https://example.test/v1","model":"fixture"}"#)
        .bind(credential)
        .execute(db.pool()).await.unwrap();
    let builder = FactoryMaterializer::new(
        config.clone(),
        cipher,
        std::collections::HashMap::from([("openai".into(), 4096)]),
        Arc::new(WorkerSupervisor::start_many(vec![], vec![])),
    )
    .unwrap();
    let manager = ProviderRuntimeManager::new(
        RuntimeLimits {
            max_parallel_loads: 1,
            max_pending_loads: 1,
            max_waiters: 4,
            max_resident_bytes: 8192,
            max_resources: 2,
            max_version_entries: 8,
            admission_timeout_ms: 1000,
            failure_cooldown_ms: 10,
            idle_ttl_ms: 600000,
        },
        Arc::new(builder),
        AdmissionGate::open(),
    )
    .unwrap();
    // Startup uses the canonical credential reference; preflight must also use it on reload.
    let snapshot = db
        .enabled_provider_rows()
        .await
        .unwrap()
        .into_iter()
        .find(|row| row.key == "worker")
        .unwrap();
    drop(manager.acquire(snapshot).await.unwrap());
    let models = ManagedModels {
        database: db.clone(),
        config: config.clone(),
        manager: manager.clone(),
        quarantine: Default::default(),
    };
    let pin = db.background_provider_pin(&config, "worker").await.unwrap();
    assert_eq!(manager.evict_idle().await.unwrap(), 1);
    let result = models
        .ready(&pin, super::super::engine::now().unwrap() + 5)
        .await;
    assert!(
        result.is_ok(),
        "Job preflight after idle eviction: {result:?}"
    );
    assert!(
        manager
            .shutdown_until(tokio::time::Instant::now() + std::time::Duration::from_secs(1))
            .await
    );
}

#[tokio::test]
async fn database_and_config_providers_with_same_key_are_distinct_immutable_choices() {
    let (db, config, runtime) = harness().await;
    let inserted=sqlx::query("INSERT INTO providers(key,name,type,adapter,config_json,created_at,updated_at) VALUES('planner','Database planner','llm','openai','{}',0,0)")
        .execute(db.pool()).await.unwrap().last_insert_rowid();
    let revision = runtime.engine.settings().await.unwrap().revision;
    runtime
        .engine
        .update_settings(
            BackgroundSettingsUpdate {
                enabled: true,
                orchestrator_llm_provider: Some("planner".into()),
                worker_llm_provider: Some("config:llm:planner".into()),
            },
            revision,
        )
        .await
        .unwrap();
    let job = runtime
        .engine
        .submit(request("Explicit pair"), &JobRequester::Admin, None)
        .await
        .unwrap()
        .job;
    assert_eq!(job.snapshot.orchestrator.reference, "planner");
    assert_eq!(job.snapshot.orchestrator.incarnation, inserted);
    assert_eq!(job.snapshot.worker.reference, "config:llm:planner");
    assert_eq!(job.snapshot.worker.incarnation, 0);
    runtime.shutdown().await;
    sqlx::query("DELETE FROM providers WHERE key='planner'")
        .execute(db.pool())
        .await
        .unwrap();
    sqlx::query("INSERT INTO providers(key,name,type,adapter,config_json,created_at,updated_at) VALUES('planner','Replacement','llm','openai','{}',0,0)")
        .execute(db.pool()).await.unwrap();
    let restarted = BackgroundRuntime::foundation_harness(db, config)
        .await
        .unwrap();
    let historical = restarted.engine.get(&job.id).await.unwrap();
    assert_eq!(historical.snapshot.orchestrator.incarnation, inserted);
    assert_eq!(
        historical.error_code.as_deref(),
        Some("configuration_changed")
    );
    restarted.shutdown().await;
}

#[tokio::test]
async fn concurrent_idempotent_requests_commit_exactly_one_job() {
    let (_, _, runtime) = harness().await;
    let mut requests = Vec::new();
    for _ in 0..8 {
        let engine = runtime.engine.clone();
        requests.push(tokio::spawn(async move {
            engine
                .submit(
                    request("Same intent"),
                    &JobRequester::Admin,
                    Some("concurrent-once"),
                )
                .await
                .unwrap()
        }));
    }
    let mut id = None;
    let mut first = 0;
    for request in requests {
        let receipt = request.await.unwrap();
        if !receipt.replayed {
            first += 1;
        }
        if let Some(id) = &id {
            assert_eq!(&receipt.job.id, id);
        } else {
            id = Some(receipt.job.id);
        }
    }
    assert_eq!(first, 1);
    assert_eq!(runtime.engine.list(1, 30, None, None).await.unwrap().1, 1);
    runtime.shutdown().await;
}
