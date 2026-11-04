use super::harness::*;

#[test]
fn single_provider_materialization_rejects_invalid_config_before_secret_resolution() {
    let secrets = Secrets(AtomicUsize::new(0));
    let row = DesiredProvider {
        id: 1,
        key: "llm".into(),
        kind: "llm".into(),
        adapter: "openai".into(),
        revision: 2,
        config_json: r#"{"model":"bad","token":"forbidden"}"#.into(),
        secret_ref: Some("DEPLOYMENT_SECRET".into()),
    };
    assert!(matches!(
        materialize_provider(&config(), &row, &secrets),
        Err(DatabaseRuntimeFailure::Configuration)
    ));
    assert_eq!(secrets.0.load(Ordering::SeqCst), 0);
}

#[test]
fn single_remote_provider_materialization_creates_an_exact_usable_catalog_without_network_io() {
    let secrets = Secrets(AtomicUsize::new(0));
    let row = DesiredProvider {
        id: 1,
        key: "llm".into(),
        kind: "llm".into(),
        adapter: "openai".into(),
        revision: 3,
        config_json: provider_config::validate_raw(
            "openai",
            r#"{"base_url":"https://example.test/v1","model":"new-model"}"#,
        )
        .unwrap(),
        secret_ref: None,
    };
    let catalog = materialize_provider(&config(), &row, &secrets).unwrap();
    assert!(
        catalog
            .admit_diagnostic(DiagnosticRuntimeKind::Llm, "llm")
            .is_ok()
    );
    assert!(
        catalog
            .admit_diagnostic(DiagnosticRuntimeKind::Llm, "different")
            .is_err()
    );
    assert_eq!(secrets.0.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn production_manager_materializer_owns_remote_runtime_and_acknowledged_unload() {
    let config = Arc::new(config());
    let supervisor = Arc::new(WorkerSupervisor::start_many(vec![], vec![]));
    let builder = FactoryMaterializer::new(
        config,
        Arc::new(Secrets(AtomicUsize::new(0))),
        HashMap::from([("openai".into(), 4096)]),
        supervisor,
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
            idle_ttl_ms: 10,
        },
        Arc::new(builder),
        AdmissionGate::open(),
    )
    .unwrap();
    let lease = manager
        .acquire(DesiredProvider {
            id: 1,
            key: "remote".into(),
            kind: "llm".into(),
            adapter: "openai".into(),
            revision: 1,
            config_json: provider_config::validate_raw(
                "openai",
                r#"{"base_url":"https://example.test/v1","model":"fixture"}"#,
            )
            .unwrap(),
            secret_ref: None,
        })
        .await
        .unwrap();
    let catalog = lease.runtimes().unwrap();
    let permit = catalog
        .admit_diagnostic(DiagnosticRuntimeKind::Llm, "remote")
        .unwrap();
    drop(catalog);
    drop(lease);
    assert!(manager.evict_idle().await.is_err());
    assert_eq!(manager.accounting().reserved_bytes, 4096);
    drop(permit);
    assert_eq!(manager.evict_idle().await.unwrap(), 1);
    assert_eq!(manager.accounting().reserved_bytes, 0);
    assert!(
        manager
            .shutdown_until(tokio::time::Instant::now() + Duration::from_secs(1))
            .await
    );
}

#[tokio::test]
#[ignore = "requires acknowledged installed Gipformer and native ONNX runtime; set PROVIDER_QUALIFICATION_CONFIG"]
async fn native_gipformer_single_frame_finishes_without_foreign_exception() {
    let path = std::env::var("PROVIDER_QUALIFICATION_CONFIG").expect("qualification config");
    let config = AppConfig::parse_and_resolve(path).expect("qualification config valid");
    let row = deployment_provider_snapshot(&config, "asr", "gipformer_vi").unwrap();
    let catalog = materialize_provider(&config, &row, &Secrets(AtomicUsize::new(0))).unwrap();
    for samples in [1, 960, 16_000] {
        let mut operation = catalog
            .asr_diagnostic(
                "gipformer_vi",
                vela_server::audio::PcmF32Mono::new(vec![0.0; samples], 16_000),
            )
            .unwrap();
        let output = operation
            .execute(tokio_util::sync::CancellationToken::new())
            .await
            .unwrap();
        assert!(output.len() <= 32_768);
        assert!(operation.await_terminal_acknowledgement().await);
    }
}

#[test]
fn runtime_configuration_requires_complete_explicit_budget_and_rejects_unknown_fields() {
    let profile = r#"
max_parallel_loads=1
max_pending_loads=8
max_waiters=64
max_resident_bytes=8589934592
max_resources=16
max_version_entries=128
admission_timeout_ms=5000
failure_cooldown_ms=1000
idle_ttl_ms=600000
[estimated_peak_bytes]
silero_onnx=134217728
"#;
    let config: ProviderRuntimeConfig = toml::from_str(profile).unwrap();
    assert_eq!(config.limits.max_resident_bytes, 8589934592);
    assert!(
        toml::from_str::<ProviderRuntimeConfig>(
            &profile.replace("max_resident_bytes=8589934592", "")
        )
        .is_err()
    );
    assert!(
        toml::from_str::<ProviderRuntimeConfig>(&format!("unknown_setting=1\n{profile}")).is_err()
    );
}

#[test]
fn checkout_example_configuration_parses_without_resolving_or_printing_credentials() {
    let path =
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../config.example.toml");
    let config = AppConfig::parse_and_resolve(path).expect("checkout example config valid");
    assert!(config.provider_runtime.is_none());
}
