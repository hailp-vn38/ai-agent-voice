use std::{
    collections::HashMap,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};
use voice_agent_server::{
    config::AppConfig,
    database::{
        DesiredProvider, provider_config,
        secrets::{SecretRef, SecretResolveError, SecretResolver, SecretValue},
    },
    lifecycle::AdmissionGate,
    providers::{DatabaseRuntimeFailure, materialize_provider},
    services::provider_runtime::{
        FactoryMaterializer, ProviderRuntimeManager, ResourceKey, RuntimeError, RuntimeLimits,
        RuntimeMaterializer, RuntimeResource,
    },
    workers::{ProviderRuntimeAdmission, WorkerSupervisor},
};
struct Secrets(AtomicUsize);
impl SecretResolver for Secrets {
    fn resolve(&self, _: &SecretRef) -> Result<SecretValue, SecretResolveError> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Err(SecretResolveError::Unavailable)
    }
}
fn config() -> AppConfig {
    toml::from_str(
        r#"
[server]
bind="127.0.0.1:0"
public_ws_url="ws://127.0.0.1:0/voice/v1/"
[provider_defaults]
vad="vad"
asr="asr"
llm="llm"
tts="tts"
"#,
    )
    .unwrap()
}
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
    use voice_agent_server::providers::DiagnosticRuntimeKind;
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
    use std::{collections::HashMap, sync::Arc, time::Duration};
    use voice_agent_server::{
        lifecycle::AdmissionGate,
        services::provider_runtime::{FactoryMaterializer, ProviderRuntimeManager, RuntimeLimits},
        workers::WorkerSupervisor,
    };
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
        .admit_diagnostic(
            voice_agent_server::providers::DiagnosticRuntimeKind::Llm,
            "remote",
        )
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
    use voice_agent_server::{
        audio::PcmF32Mono, providers::deployment_provider_snapshot,
        services::provider_diagnostic::ProviderDiagnosticOperation,
    };
    let path = std::env::var("PROVIDER_QUALIFICATION_CONFIG").expect("qualification config");
    let config = AppConfig::parse_and_resolve(path).expect("qualification config valid");
    let row = deployment_provider_snapshot(&config, "asr", "gipformer_vi").unwrap();
    let catalog = materialize_provider(&config, &row, &Secrets(AtomicUsize::new(0))).unwrap();
    for samples in [1, 960, 16_000] {
        let mut operation = catalog
            .asr_diagnostic("gipformer_vi", PcmF32Mono::new(vec![0.0; samples], 16_000))
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
fn native_resource_identity_normalizes_defaults_and_isolates_execution_and_credentials() {
    use std::{collections::HashMap, sync::Arc};
    use voice_agent_server::{
        services::provider_runtime::{FactoryMaterializer, RuntimeMaterializer},
        workers::WorkerSupervisor,
    };
    let mut cfg = config();
    cfg.deployment.model_manifest =
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../models/manifest.toml");
    cfg.runtime.onnx.library = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../runtime/onnxruntime/libonnxruntime.dylib");
    let builder = FactoryMaterializer::new(
        Arc::new(cfg.clone()),
        Arc::new(Secrets(AtomicUsize::new(0))),
        HashMap::from([("gipformer_sherpa_offline".into(), 1024)]),
        Arc::new(WorkerSupervisor::start_many(vec![], vec![])),
    )
    .unwrap();
    let mut row = DesiredProvider {
        id: 1,
        key: "a".into(),
        kind: "asr".into(),
        adapter: "gipformer_sherpa_offline".into(),
        revision: 1,
        config_json: r#"{}"#.into(),
        secret_ref: None,
    };
    let first = builder.resource_key(&row).unwrap().unwrap();
    row.config_json =
        voice_agent_server::database::provider_config::validate_raw(&row.adapter, &row.config_json)
            .unwrap();
    row.id = 2;
    row.key = "b".into();
    row.revision = 99;
    assert_eq!(builder.resource_key(&row).unwrap(), Some(first.clone()));
    let mut value: serde_json::Value = serde_json::from_str(&row.config_json).unwrap();
    value["num_threads"] = serde_json::json!(2);
    row.config_json = value.to_string();
    assert!(
        builder.resource_key(&row).is_err(),
        "DB thread override must be rejected"
    );
    value.as_object_mut().unwrap().remove("num_threads");
    row.config_json = value.to_string();
    assert_eq!(builder.resource_key(&row).unwrap(), Some(first));
    cfg.runtime
        .onnx
        .threads
        .insert("gipformer_sherpa_offline".into(), 2);
    let other = FactoryMaterializer::new(
        Arc::new(cfg),
        Arc::new(Secrets(AtomicUsize::new(0))),
        HashMap::from([("gipformer_sherpa_offline".into(), 1024)]),
        Arc::new(WorkerSupervisor::start_many(vec![], vec![])),
    )
    .unwrap();
    assert_ne!(
        other.resource_key(&row).unwrap(),
        builder.resource_key(&row).unwrap(),
        "server execution settings must isolate backing resources"
    );
    row.secret_ref = Some("CREDENTIAL".into());
    assert!(builder.resource_key(&row).unwrap().is_none());
}

#[test]
fn zerotts_resource_identity_shares_voices_but_isolates_delivery_mode() {
    use std::{collections::HashMap, sync::Arc};
    use voice_agent_server::{
        services::provider_runtime::{FactoryMaterializer, RuntimeMaterializer},
        workers::WorkerSupervisor,
    };

    let mut cfg = config();
    cfg.deployment.model_manifest =
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../models/manifest.toml");
    cfg.runtime.onnx.library = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../runtime/onnxruntime/libonnxruntime.dylib");
    let builder = FactoryMaterializer::new(
        Arc::new(cfg),
        Arc::new(Secrets(AtomicUsize::new(0))),
        HashMap::from([("zerotts_onnx".into(), 1024)]),
        Arc::new(WorkerSupervisor::start_many(vec![], vec![])),
    )
    .unwrap();
    let row = |key: &str, voice: &str, delivery_mode: &str| DesiredProvider {
        id: 1,
        key: key.into(),
        kind: "tts".into(),
        adapter: "zerotts_onnx".into(),
        revision: 1,
        config_json: format!(
            r#"{{"voice":"{voice}","language":"vi-VN","delivery_mode":"{delivery_mode}"}}"#
        ),
        secret_ref: None,
    };

    let maichi = builder
        .resource_key(&row("maichi", "maichi", "stream"))
        .unwrap();
    let baotrang = builder
        .resource_key(&row("baotrang", "baotrang", "stream"))
        .unwrap();
    let file = builder
        .resource_key(&row("maichi-file", "maichi", "file"))
        .unwrap();
    assert_eq!(maichi, baotrang);
    assert_ne!(maichi, file);
    assert!(
        builder
            .resource_key(&row("unsupported", "not-a-zerotts-voice", "stream"))
            .is_err()
    );
}

#[test]
fn local_runtime_identity_uses_adapter_owned_physical_specs() {
    use std::{collections::HashMap, sync::Arc};
    use voice_agent_server::{
        services::provider_runtime::{FactoryMaterializer, RuntimeMaterializer},
        workers::WorkerSupervisor,
    };

    let mut cfg = config();
    cfg.deployment.model_manifest =
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../models/manifest.toml");
    cfg.runtime.onnx.library = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../runtime/onnxruntime/libonnxruntime.dylib");
    let builder = FactoryMaterializer::new(
        Arc::new(cfg),
        Arc::new(Secrets(AtomicUsize::new(0))),
        HashMap::from([
            ("silero_onnx".into(), 1024),
            ("zipformer_sherpa".into(), 1024),
            ("gipformer_sherpa_offline".into(), 1024),
        ]),
        Arc::new(WorkerSupervisor::start_many(vec![], vec![])),
    )
    .unwrap();
    let row = |kind: &str, adapter: &str, config_json: &str| DesiredProvider {
        // Deployment snapshots may include server-owned local settings; database provider
        // configuration intentionally cannot.
        id: 0,
        key: "local".into(),
        kind: kind.into(),
        adapter: adapter.into(),
        revision: 1,
        config_json: config_json.into(),
        secret_ref: None,
    };

    let silero_a = builder
        .resource_key(&row(
            "vad",
            "silero_onnx",
            r#"{"speech_threshold":0.5,"end_silence_ms":400}"#,
        ))
        .unwrap();
    let silero_b = builder
        .resource_key(&row(
            "vad",
            "silero_onnx",
            r#"{"speech_threshold":0.7,"end_silence_ms":800}"#,
        ))
        .unwrap();
    assert_eq!(silero_a, silero_b, "VAD endpoint policy is a logical view");

    let zipformer_greedy = builder
        .resource_key(&row(
            "asr",
            "zipformer_sherpa",
            r#"{"decoding_method":"greedy_search"}"#,
        ))
        .unwrap();
    let zipformer_beam = builder
        .resource_key(&row(
            "asr",
            "zipformer_sherpa",
            r#"{"decoding_method":"modified_beam_search"}"#,
        ))
        .unwrap();
    assert_ne!(
        zipformer_greedy, zipformer_beam,
        "Zipformer constructs a recognizer with its decoding method"
    );

    let gipformer_small_beam = builder
        .resource_key(&row(
            "asr",
            "gipformer_sherpa_offline",
            r#"{"max_active_paths":4}"#,
        ))
        .unwrap();
    let gipformer_large_beam = builder
        .resource_key(&row(
            "asr",
            "gipformer_sherpa_offline",
            r#"{"max_active_paths":8}"#,
        ))
        .unwrap();
    assert_ne!(
        gipformer_small_beam, gipformer_large_beam,
        "Gipformer constructs a recognizer with max_active_paths"
    );
}

#[test]
fn kokoro_runtime_plan_excludes_preload_but_keeps_native_selection_and_g2p_identity() {
    use voice_agent_server::providers::compiled_local_runtime_adapter_registry;

    let planner = compiled_local_runtime_adapter_registry()
        .get("kokoro_vi_onnx")
        .expect("compiled Kokoro planner");
    let plan = |voice: &str, speed_percent: u16, preload: bool| {
        planner
            .physical_plan(serde_json::json!({
                "model": "kokoro_vi_contextbox",
                "num_threads": 2,
                "voice": voice,
                "language": "vi-VN",
                "speed_percent": speed_percent,
                "preload": preload,
            }))
            .unwrap()
    };
    let key = |plan: &voice_agent_server::services::provider_runtime::LocalRuntimePlan,
               g2p_fingerprint| {
        plan.resource_key("artifact".into(), [1; 32], Some(g2p_fingerprint), 2, 1)
            .unwrap()
    };

    let default = plan("mai_linh", 100, false);
    let preloaded = plan("mai_linh", 100, true);
    let other_voice = plan("thanh_dat", 100, false);
    let other_speed = plan("mai_linh", 120, false);
    assert_eq!(key(&default, [2; 32]), key(&preloaded, [2; 32]));
    assert_ne!(key(&default, [2; 32]), key(&other_voice, [2; 32]));
    assert_ne!(key(&default, [2; 32]), key(&other_speed, [2; 32]));
    assert_ne!(key(&default, [2; 32]), key(&default, [3; 32]));
}

#[test]
fn native_resource_identity_uses_execution_content_not_only_its_path() {
    use std::{collections::HashMap, sync::Arc};
    use voice_agent_server::{
        services::provider_runtime::{FactoryMaterializer, RuntimeMaterializer},
        workers::WorkerSupervisor,
    };
    let path = std::env::temp_dir().join(format!(
        "provider-runtime-execution-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::write(&path, b"execution-one").unwrap();
    let mut cfg = config();
    cfg.deployment.model_manifest =
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../models/manifest.toml");
    cfg.runtime.onnx.library = path.clone();
    let row = DesiredProvider {
        id: 1,
        key: "asr".into(),
        kind: "asr".into(),
        adapter: "gipformer_sherpa_offline".into(),
        revision: 1,
        config_json: r#"{}"#.into(),
        secret_ref: None,
    };
    let build = |cfg: AppConfig| {
        FactoryMaterializer::new(
            Arc::new(cfg),
            Arc::new(Secrets(AtomicUsize::new(0))),
            HashMap::from([("gipformer_sherpa_offline".into(), 1024)]),
            Arc::new(WorkerSupervisor::start_many(vec![], vec![])),
        )
        .unwrap()
    };
    let first = build(cfg.clone()).resource_key(&row).unwrap();
    std::fs::write(&path, b"execution-two").unwrap();
    let second = build(cfg).resource_key(&row).unwrap();
    std::fs::remove_file(path).unwrap();
    assert_ne!(first, second);
}

#[test]
fn qualified_manifest_receipt_is_frozen_for_the_materializer_lifetime() {
    use sha2::{Digest, Sha256};
    use std::{collections::HashMap, sync::Arc};
    use voice_agent_server::{
        config::ProviderRuntimeConfig,
        services::provider_runtime::{
            FactoryMaterializer, RuntimeError, RuntimeLimits, RuntimeMaterializer,
        },
        workers::WorkerSupervisor,
    };
    let path = std::env::temp_dir().join(format!(
        "provider-runtime-manifest-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    let original = b"qualified manifest";
    std::fs::write(&path, original).unwrap();
    let mut cfg = config();
    cfg.deployment.model_manifest = path.clone();
    let estimates = HashMap::from([("openai".into(), 1024)]);
    cfg.provider_runtime = Some(ProviderRuntimeConfig {
        limits: RuntimeLimits {
            max_parallel_loads: 1,
            max_pending_loads: 1,
            max_waiters: 1,
            max_resident_bytes: 1024,
            max_resources: 1,
            max_version_entries: 1,
            admission_timeout_ms: 1,
            failure_cooldown_ms: 1,
            idle_ttl_ms: 1,
        },
        estimated_peak_bytes: estimates.clone(),
        measured_manifest_sha256: Sha256::digest(original)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect(),
        startup_timeout_ms: 1000,
    });
    let builder = FactoryMaterializer::new(
        Arc::new(cfg),
        Arc::new(Secrets(AtomicUsize::new(0))),
        estimates,
        Arc::new(WorkerSupervisor::start_many(vec![], vec![])),
    )
    .unwrap();
    std::fs::write(&path, b"changed manifest").unwrap();
    let row = DesiredProvider {
        id: 1,
        key: "llm".into(),
        kind: "llm".into(),
        adapter: "openai".into(),
        revision: 1,
        config_json: r#"{"base_url":"https://example.test/v1","model":"fixture"}"#.into(),
        secret_ref: None,
    };
    assert!(matches!(
        builder.estimated_peak_bytes(&row),
        Err(RuntimeError::Configuration)
    ));
    std::fs::remove_file(path).unwrap();
}

#[test]
fn runtime_configuration_requires_complete_explicit_budget_and_rejects_unknown_fields() {
    use voice_agent_server::config::ProviderRuntimeConfig;
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
measured_manifest_sha256="receipt"
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

/// Delegates production identity and estimation to `FactoryMaterializer` while counting native
/// builds, so tests can prove physical sharing without installed ZeroTTS graphs.
struct CountingLocalFactory {
    inner: FactoryMaterializer,
    builds: Arc<AtomicUsize>,
}

impl RuntimeMaterializer for CountingLocalFactory {
    fn resource_key(
        &self,
        snapshot: &DesiredProvider,
    ) -> Result<Option<ResourceKey>, RuntimeError> {
        self.inner.resource_key(snapshot)
    }
    fn estimated_peak_bytes(&self, snapshot: &DesiredProvider) -> Result<u64, RuntimeError> {
        self.inner.estimated_peak_bytes(snapshot)
    }
    fn logical_capacity(&self, snapshot: &DesiredProvider) -> Result<usize, RuntimeError> {
        self.inner.logical_capacity(snapshot)
    }
    fn global_capacity(&self, snapshot: &DesiredProvider) -> Result<Option<usize>, RuntimeError> {
        self.inner.global_capacity(snapshot)
    }
    fn build(
        &self,
        _: &DesiredProvider,
        _: ProviderRuntimeAdmission,
    ) -> Result<Arc<dyn RuntimeResource>, RuntimeError> {
        self.builds.fetch_add(1, Ordering::SeqCst);
        Ok(Arc::new(UnloadedResource))
    }
}

struct UnloadedResource;

impl RuntimeResource for UnloadedResource {
    fn resource_key(&self) -> Option<ResourceKey> {
        None
    }
    fn unload(&self) -> bool {
        true
    }
}

fn zerotts_factory(workers_tts: usize) -> FactoryMaterializer {
    let mut cfg = zerotts_config();
    cfg.workers.tts.max_workers = workers_tts;
    FactoryMaterializer::new(
        Arc::new(cfg),
        Arc::new(Secrets(AtomicUsize::new(0))),
        HashMap::from([("zerotts_onnx".into(), 1024)]),
        Arc::new(WorkerSupervisor::start_many(vec![], vec![])),
    )
    .unwrap()
}

fn zerotts_config() -> AppConfig {
    let mut cfg: AppConfig = toml::from_str(
        r#"
[server]
bind="127.0.0.1:0"
public_ws_url="ws://127.0.0.1:0/voice/v1/"
[provider_defaults]
vad="vad"
asr="asr"
llm="llm"
tts="tts"
[providers.tts.instances.zerotts_maichi]
adapter="zerotts_onnx"
voice="maichi"
delivery_mode="stream"
"#,
    )
    .unwrap();
    cfg.deployment.model_manifest =
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../models/manifest.toml");
    cfg.runtime.onnx.library = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../runtime/onnxruntime/libonnxruntime.dylib");
    cfg
}

fn zerotts_manager(
    factory: FactoryMaterializer,
    builds: Arc<AtomicUsize>,
) -> Arc<ProviderRuntimeManager> {
    ProviderRuntimeManager::new(
        RuntimeLimits {
            max_parallel_loads: 1,
            max_pending_loads: 1,
            max_waiters: 4,
            max_resident_bytes: 4096,
            max_resources: 2,
            max_version_entries: 8,
            admission_timeout_ms: 1000,
            failure_cooldown_ms: 10,
            idle_ttl_ms: 60_000,
        },
        Arc::new(CountingLocalFactory {
            inner: factory,
            builds,
        }),
        AdmissionGate::open(),
    )
    .unwrap()
}

fn zerotts_provider(id: i64, key: &str, voice: &str, delivery_mode: &str) -> DesiredProvider {
    DesiredProvider {
        id,
        key: key.into(),
        kind: "tts".into(),
        adapter: "zerotts_onnx".into(),
        revision: 1,
        config_json: format!(
            r#"{{"voice":"{voice}","language":"vi-VN","delivery_mode":"{delivery_mode}"}}"#
        ),
        secret_ref: None,
    }
}

#[tokio::test]
async fn switching_the_logical_voice_never_rematerializes_the_zerotts_runtime() {
    let builds = Arc::new(AtomicUsize::new(0));
    let manager = zerotts_manager(zerotts_factory(2), Arc::clone(&builds));

    // An Agent's home binds Template A -> maichi and later switches to Template B -> hamy. The two
    // templates differ only in the logical voice, so the switch must alias the resident runtime.
    let template_a = manager
        .acquire(zerotts_provider(1, "tts-maichi", "maichi", "stream"))
        .await
        .unwrap();
    assert_eq!(builds.load(Ordering::SeqCst), 1);
    drop(template_a);

    let template_b = manager
        .acquire(zerotts_provider(2, "tts-hamy", "hamy", "stream"))
        .await
        .unwrap();
    assert_eq!(
        builds.load(Ordering::SeqCst),
        1,
        "switching template voice must not rebuild the ZeroTTS runtime"
    );
    assert_eq!(manager.accounting().resources, 1);
    drop(template_b);
    assert_eq!(
        manager.evict_idle().await.unwrap(),
        1,
        "one physical runtime stays resident until it is idle-drained"
    );
}

#[tokio::test]
async fn a_different_zerotts_delivery_mode_builds_a_second_physical_runtime() {
    let builds = Arc::new(AtomicUsize::new(0));
    let manager = zerotts_manager(zerotts_factory(2), Arc::clone(&builds));

    let stream = manager
        .acquire(zerotts_provider(1, "tts-stream", "maichi", "stream"))
        .await
        .unwrap();
    let file = manager
        .acquire(zerotts_provider(2, "tts-file", "hamy", "file"))
        .await
        .unwrap();
    assert_eq!(
        builds.load(Ordering::SeqCst),
        2,
        "codec session topology differs, so delivery mode must isolate the runtime"
    );
    assert_eq!(manager.accounting().resources, 2);
    drop((stream, file));
}
