use crate::{
    config::{AppConfig, SileroOnnxConfig, VadInstanceConfig},
    database::{
        Database, DeviceAdmissionError,
        secrets::{EnvSecretResolver, SecretResolver},
    },
    providers::{
        DatabaseRuntimeSnapshot, LoadedProviders, ProviderCatalog, ProviderSet, RuntimeCatalog,
    },
    session::{
        ActiveTurnLimiter, EffectiveSessionProfile, ProfileUnavailable, WriterOutcomeProbe,
        resolve_effective_session_profile,
    },
    workers::{
        AsrWorkerRuntime, LlmRuntime, TtsWorkerRuntime, VadWorkerRuntime, WorkerRuntimeConfig,
        WorkerSupervisor,
    },
};
use std::{collections::HashMap, sync::Arc, time::Duration};
use tokio_util::sync::CancellationToken;

/// Application-owned state. Production sessions resolve a fixed runtime snapshot from catalogs.
#[derive(Clone)]
pub struct AppState {
    pub config: Arc<AppConfig>,
    pub providers: Arc<ProviderCatalog>,
    pub runtimes: Arc<RuntimeCatalog>,
    pub worker_supervisor: Arc<WorkerSupervisor>,
    pub active_turn_limiter: Arc<ActiveTurnLimiter>,
    pub database: Option<Arc<Database>>,
    pub database_runtime_snapshot: Option<Arc<DatabaseRuntimeSnapshot>>,
    pub secret_resolver: Arc<dyn SecretResolver>,
    pub shutdown: CancellationToken,
    /// Test-only; see [`WriterOutcomeProbe`]. Production leaves it unset.
    pub writer_outcome_probe: Option<Arc<dyn WriterOutcomeProbe>>,
}

impl AppState {
    pub fn bound_vision_runtime(&self) -> Option<Arc<crate::workers::VisionRuntime>> {
        let id = self.config.effective_agent.providers.vision.as_deref()?;
        self.runtimes.vision(id).ok()
    }

    /// Deterministic public-HTTP seam: it injects only the already-built Vision runtime,
    /// leaving the unrelated Voice Session providers unavailable.
    pub fn with_vision_runtime_for_test(
        mut self,
        instance_id: impl Into<String>,
        provider: Arc<dyn crate::providers::VisionProvider>,
        concurrency: usize,
        timeout: Duration,
    ) -> Self {
        let instance_id = instance_id.into();
        let config = Arc::make_mut(&mut self.config);
        config.vision.enabled = true;
        config.effective_agent.providers.vision = Some(instance_id.clone());
        Arc::make_mut(&mut self.runtimes).vision.insert(
            instance_id,
            Arc::new(crate::workers::VisionRuntime::new(
                provider,
                concurrency,
                timeout,
            )),
        );
        self
    }

    /// Deterministic admission seam: registers one extra already-loaded LLM runtime under an
    /// explicit instance id so a stored Template can bind a runtime other than the server default.
    pub fn with_llm_runtime_for_test(
        mut self,
        instance_id: impl Into<String>,
        provider: Arc<dyn crate::providers::LlmProvider>,
        concurrency: usize,
        timeout: Duration,
    ) -> Self {
        Arc::make_mut(&mut self.runtimes).llm.insert(
            instance_id.into(),
            Arc::new(LlmRuntime::new(provider, concurrency, timeout)),
        );
        self
    }

    /// Resolves the one Effective Session Profile this connection may use.  Database-backed
    /// admission is fail-closed: an unknown Device is denied and any resolution failure is coarse,
    /// so a broken intended configuration can never be masked by deployment defaults.
    pub async fn resolve_session_profile(
        &self,
        device_id: &str,
    ) -> Result<EffectiveSessionProfile, SessionProfileAdmissionError> {
        if !self.config.database.devices.admission_enabled {
            return EffectiveSessionProfile::server_default(&self.config)
                .map_err(|_| SessionProfileAdmissionError::ProfileUnavailable);
        }
        let database = self
            .database
            .as_ref()
            .ok_or(SessionProfileAdmissionError::AdmissionUnavailable)?;
        let graph = database
            .admit_device(device_id, &self.config.database.devices)
            .await
            .map_err(|error| match error {
                DeviceAdmissionError::Denied => SessionProfileAdmissionError::Denied,
                DeviceAdmissionError::Unavailable => {
                    SessionProfileAdmissionError::AdmissionUnavailable
                }
            })?;
        resolve_effective_session_profile(
            graph.device_db_id,
            graph.agent.id,
            &graph.agent.key,
            &graph.assignments,
            &self.config,
            &self.runtimes,
        )
        .map_err(|ProfileUnavailable| SessionProfileAdmissionError::ProfileUnavailable)
    }
    pub fn new(config: AppConfig, loaded: LoadedProviders) -> Self {
        Self::new_with_database(config, loaded, None)
    }

    pub fn new_with_database(
        config: AppConfig,
        loaded: LoadedProviders,
        database: Option<Database>,
    ) -> Self {
        Self::new_with_database_and_shutdown(config, loaded, database, CancellationToken::new())
    }

    pub fn new_with_database_and_shutdown(
        config: AppConfig,
        loaded: LoadedProviders,
        database: Option<Database>,
        shutdown: CancellationToken,
    ) -> Self {
        Self::new_with_database_runtime_snapshot_and_shutdown(
            config, loaded, database, None, shutdown,
        )
    }

    pub fn new_with_database_runtime_snapshot_and_shutdown(
        config: AppConfig,
        loaded: LoadedProviders,
        database: Option<Database>,
        database_runtime_snapshot: Option<DatabaseRuntimeSnapshot>,
        shutdown: CancellationToken,
    ) -> Self {
        Self::new_with_database_runtime_snapshot_resolver_and_shutdown(
            config,
            loaded,
            database,
            database_runtime_snapshot,
            Arc::new(EnvSecretResolver),
            shutdown,
        )
    }

    pub fn new_with_database_runtime_snapshot_resolver_and_shutdown(
        config: AppConfig,
        loaded: LoadedProviders,
        database: Option<Database>,
        database_runtime_snapshot: Option<DatabaseRuntimeSnapshot>,
        secret_resolver: Arc<dyn SecretResolver>,
        shutdown: CancellationToken,
    ) -> Self {
        let supervisor = Arc::new(WorkerSupervisor::start_many(
            loaded.runtimes.asr.values().cloned().collect(),
            loaded
                .runtimes
                .vad
                .values()
                .map(|vad| Arc::clone(&vad.runtime))
                .collect(),
        ));
        Self {
            active_turn_limiter: Arc::new(ActiveTurnLimiter::new(config.limits.max_active_turns)),
            config: Arc::new(config),
            providers: Arc::new(loaded.providers),
            runtimes: Arc::new(loaded.runtimes),
            worker_supervisor: supervisor,
            database: database.map(Arc::new),
            database_runtime_snapshot: database_runtime_snapshot.map(Arc::new),
            secret_resolver,
            shutdown,
            writer_outcome_probe: None,
        }
    }

    /// Installs the test-only writer outcome probe; see [`WriterOutcomeProbe`].
    pub fn with_writer_outcome_probe(mut self, probe: Arc<dyn WriterOutcomeProbe>) -> Self {
        self.writer_outcome_probe = Some(probe);
        self
    }

    /// Compatibility constructor for deterministic test routers. Production uses `new`.
    pub fn from_provider_set(config: AppConfig, providers: Arc<ProviderSet>) -> Self {
        Self::from_provider_set_with_database(config, providers, None)
    }

    pub fn from_provider_set_with_database(
        config: AppConfig,
        providers: Arc<ProviderSet>,
        database: Option<Database>,
    ) -> Self {
        Self::from_provider_set_with_database_and_shutdown(
            config,
            providers,
            database,
            CancellationToken::new(),
        )
    }

    pub fn from_provider_set_with_database_and_shutdown(
        config: AppConfig,
        providers: Arc<ProviderSet>,
        database: Option<Database>,
        shutdown: CancellationToken,
    ) -> Self {
        let (config, loaded) = loaded_from_provider_set(config, &providers);
        Self::new_with_database_and_shutdown(config, loaded, database, shutdown)
    }
}

/// Bounded admission failure.  The reason stays internal; the client only sees the coarse class.
/// The two unavailable classes stay distinct because the integration guide names them as separate
/// internal diagnostics: a broken database versus an unresolvable Agent profile.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum SessionProfileAdmissionError {
    #[error("device_not_admitted")]
    Denied,
    #[error("device_resolution_failed")]
    AdmissionUnavailable,
    #[error("agent_profile_resolution_failed")]
    ProfileUnavailable,
}

/// Publishes an injected ProviderSet as already-loaded runtime under the single `test` instance
/// id.  Deterministic routers use it so no model is ever prepared in a test process.
pub(super) fn loaded_from_provider_set(
    mut config: AppConfig,
    providers: &ProviderSet,
) -> (AppConfig, LoadedProviders) {
    let id = "test".to_owned();
    config.effective_agent.providers.vad = id.clone();
    config.effective_agent.providers.asr = id.clone();
    config.effective_agent.providers.llm = id.clone();
    config.effective_agent.providers.tts = id.clone();
    config
        .providers
        .vad
        .instances
        .entry(id.clone())
        .or_insert_with(|| VadInstanceConfig::SileroOnnx(SileroOnnxConfig::default()));
    let vad = providers.vad_provider();
    let asr = providers.asr_provider();
    let llm = providers.llm_provider();
    let tts = providers.tts_provider();
    let vad_runtime = Arc::new(VadWorkerRuntime::new(
        Arc::clone(&vad),
        WorkerRuntimeConfig {
            max_workers: config.workers.vad.max_workers,
            command_capacity: config.workers.vad.command_queue_capacity,
            final_timeout: Duration::from_millis(config.workers.vad.reset_timeout_ms),
            cleanup_grace: Duration::from_millis(config.workers.vad.cleanup_grace_ms),
        },
    ));
    let asr_runtime = Arc::new(AsrWorkerRuntime::new(
        Arc::clone(&asr),
        WorkerRuntimeConfig {
            max_workers: config.workers.asr.max_workers,
            command_capacity: config.workers.asr.command_queue_capacity,
            final_timeout: Duration::from_millis(config.workers.asr.final_timeout_ms),
            cleanup_grace: Duration::from_millis(config.workers.asr.cleanup_grace_ms),
        },
    ));
    let llm_runtime = Arc::new(LlmRuntime::new(
        Arc::clone(&llm),
        config.limits.llm_concurrency,
        Duration::from_millis(60_000),
    ));
    let tts_runtime = Arc::new(TtsWorkerRuntime::new(
        Arc::clone(&tts),
        WorkerRuntimeConfig {
            max_workers: config.workers.tts.max_workers,
            command_capacity: config.workers.tts.command_queue_capacity,
            final_timeout: Duration::from_millis(config.tts.timeout_ms),
            cleanup_grace: Duration::from_millis(config.workers.tts.cleanup_grace_ms),
        },
    ));
    let (segmenter, pre_roll) =
        crate::providers::vad_timing(config.providers.vad.instances[&id].silero_onnx());
    (
        config,
        LoadedProviders {
            providers: ProviderCatalog {
                vad: HashMap::from([(id.clone(), vad)]),
                asr: HashMap::from([(id.clone(), asr)]),
                llm: HashMap::from([(id.clone(), llm)]),
                tts: HashMap::from([(id.clone(), tts)]),
                vision: HashMap::new(),
            },
            runtimes: RuntimeCatalog {
                vad: HashMap::from([(
                    id.clone(),
                    crate::providers::LoadedVad {
                        runtime: vad_runtime,
                        segmenter,
                        pre_roll_samples: pre_roll,
                    },
                )]),
                asr: HashMap::from([(id.clone(), asr_runtime)]),
                llm: HashMap::from([(id.clone(), llm_runtime)]),
                tts: HashMap::from([(id, tts_runtime)]),
                vision: HashMap::new(),
            },
        },
    )
}
