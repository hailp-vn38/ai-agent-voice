pub use std::{
    collections::HashMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

pub use vela_server::{
    app::startup_providers,
    config::{
        AppConfig, ProviderRuntimeConfig, TtsInstanceConfig, ZeroTtsDeliveryMode, ZeroTtsOnnxConfig,
    },
    database::{
        DesiredProvider, provider_config,
        secrets::{SecretRef, SecretResolveError, SecretResolver, SecretValue},
    },
    lifecycle::AdmissionGate,
    providers::{
        DatabaseRuntimeFailure, DiagnosticRuntimeKind, compiled_local_runtime_adapter_registry,
        compiled_provider_adapter_registry, configured_physical_replicas,
        deployment_provider_snapshot, materialize_provider,
    },
    services::{
        provider_diagnostic::ProviderDiagnosticOperation,
        provider_runtime::{
            FactoryMaterializer, LocalRuntimePlan, PreparedRuntime, ProviderRuntimeManager,
            ResourceKey, RuntimeError, RuntimeLimits, RuntimeMaterializer, RuntimeResource,
        },
    },
    workers::{ProviderRuntimeAdmission, ProviderWorkloadClass, WorkerSupervisor},
};

pub struct Secrets(pub AtomicUsize);

impl SecretResolver for Secrets {
    fn resolve(&self, _: &SecretRef) -> Result<SecretValue, SecretResolveError> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Err(SecretResolveError::Unavailable)
    }
}

pub fn config() -> AppConfig {
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

/// Delegates production identity and estimation to `FactoryMaterializer` while counting native
/// builds, so tests can prove physical sharing without installed ZeroTTS graphs.
pub struct CountingLocalFactory {
    pub inner: FactoryMaterializer,
    pub builds: Arc<AtomicUsize>,
    pub quotas: Arc<Mutex<Vec<ProviderRuntimeAdmission>>>,
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
    fn physical_capacity(&self, snapshot: &DesiredProvider) -> Result<usize, RuntimeError> {
        self.inner.physical_capacity(snapshot)
    }
    fn global_capacity(&self, snapshot: &DesiredProvider) -> Result<Option<usize>, RuntimeError> {
        self.inner.global_capacity(snapshot)
    }
    fn build(
        &self,
        _: &DesiredProvider,
        _: Option<PreparedRuntime>,
        quota: ProviderRuntimeAdmission,
    ) -> Result<Arc<dyn RuntimeResource>, RuntimeError> {
        self.builds.fetch_add(1, Ordering::SeqCst);
        self.quotas.lock().unwrap().push(quota);
        Ok(Arc::new(UnloadedResource))
    }
}

pub struct UnloadedResource;

impl RuntimeResource for UnloadedResource {
    fn resource_key(&self) -> Option<ResourceKey> {
        None
    }
    fn unload(&self) -> bool {
        true
    }
}

pub fn zerotts_factory(workers_tts: usize) -> FactoryMaterializer {
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

pub fn zerotts_config() -> AppConfig {
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
    cfg.runtime.onnx.library = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/onnx-runtime-fingerprint.txt");
    cfg
}

pub fn zerotts_manager(
    factory: FactoryMaterializer,
    builds: Arc<AtomicUsize>,
    quotas: Arc<Mutex<Vec<ProviderRuntimeAdmission>>>,
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
            quotas,
        }),
        AdmissionGate::open(),
    )
    .unwrap()
}

pub fn zerotts_provider(id: i64, key: &str, voice: &str, delivery_mode: &str) -> DesiredProvider {
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
