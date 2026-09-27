use crate::{
    config::{AppConfig, SileroOnnxConfig, VadInstanceConfig},
    providers::{LoadedProviders, ProviderCatalog, ProviderSet, RuntimeCatalog},
    session::ActiveTurnLimiter,
    workers::{
        AsrWorkerRuntime, LlmRuntime, TtsWorkerRuntime, VadWorkerRuntime, WorkerRuntimeConfig,
        WorkerSupervisor,
    },
};
use std::{collections::HashMap, sync::Arc, time::Duration};

/// Application-owned state. Production sessions resolve a fixed runtime snapshot from catalogs.
#[derive(Clone)]
pub struct AppState {
    pub config: Arc<AppConfig>,
    pub providers: Arc<ProviderCatalog>,
    pub runtimes: Arc<RuntimeCatalog>,
    pub worker_supervisor: Arc<WorkerSupervisor>,
    pub active_turn_limiter: Arc<ActiveTurnLimiter>,
}

impl AppState {
    pub fn new(config: AppConfig, loaded: LoadedProviders) -> Self {
        let supervisor = Arc::new(WorkerSupervisor::start_many(
            loaded.runtimes.asr.values().cloned().collect(),
            loaded.runtimes.vad.values().cloned().collect(),
        ));
        Self {
            active_turn_limiter: Arc::new(ActiveTurnLimiter::new(config.limits.max_active_turns)),
            config: Arc::new(config),
            providers: Arc::new(loaded.providers),
            runtimes: Arc::new(loaded.runtimes),
            worker_supervisor: supervisor,
        }
    }

    /// Compatibility constructor for deterministic test routers. Production uses `new`.
    pub fn from_provider_set(mut config: AppConfig, providers: Arc<ProviderSet>) -> Self {
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
        Self::new(
            config,
            LoadedProviders {
                providers: ProviderCatalog {
                    vad: HashMap::from([(id.clone(), vad)]),
                    asr: HashMap::from([(id.clone(), asr)]),
                    llm: HashMap::from([(id.clone(), llm)]),
                    tts: HashMap::from([(id.clone(), tts)]),
                },
                runtimes: RuntimeCatalog {
                    vad: HashMap::from([(id.clone(), vad_runtime)]),
                    asr: HashMap::from([(id.clone(), asr_runtime)]),
                    llm: HashMap::from([(id.clone(), llm_runtime)]),
                    tts: HashMap::from([(id, tts_runtime)]),
                },
            },
        )
    }
}
