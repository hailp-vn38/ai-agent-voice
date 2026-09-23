use crate::{
    config::AppConfig,
    providers::ProviderSet,
    session::ActiveTurnLimiter,
    workers::{
        AsrWorkerRuntime, LlmRuntime, TtsWorkerRuntime, VadWorkerRuntime, WorkerRuntimeConfig,
        WorkerSupervisor,
    },
};
use std::{sync::Arc, time::Duration};

/// Application-owned state and inference runtimes shared by all connections.
#[derive(Clone)]
pub struct AppState {
    pub config: Arc<AppConfig>,
    pub providers: Arc<ProviderSet>,
    pub asr_runtime: Arc<AsrWorkerRuntime>,
    pub vad_runtime: Arc<VadWorkerRuntime>,
    pub llm_runtime: Arc<LlmRuntime>,
    pub tts_runtime: Arc<TtsWorkerRuntime>,
    pub worker_supervisor: Arc<WorkerSupervisor>,
    pub active_turn_limiter: Arc<ActiveTurnLimiter>,
}

impl AppState {
    /// Builds the application-owned inference runtimes from validated config.
    pub fn new(config: AppConfig, providers: Arc<ProviderSet>) -> Self {
        let asr_runtime = Arc::new(AsrWorkerRuntime::new(
            providers.asr_provider(),
            WorkerRuntimeConfig {
                max_workers: config.workers.asr.max_workers,
                command_capacity: config.workers.asr.command_queue_capacity,
                final_timeout: Duration::from_millis(config.workers.asr.final_timeout_ms),
                cleanup_grace: Duration::from_millis(config.workers.asr.cleanup_grace_ms),
            },
        ));
        let vad_runtime = Arc::new(VadWorkerRuntime::new(
            providers.vad_provider(),
            WorkerRuntimeConfig {
                max_workers: config.workers.vad.max_workers,
                command_capacity: config.workers.vad.command_queue_capacity,
                final_timeout: Duration::from_millis(config.workers.vad.reset_timeout_ms),
                cleanup_grace: Duration::from_millis(config.workers.vad.cleanup_grace_ms),
            },
        ));
        let worker_supervisor = Arc::new(WorkerSupervisor::start(
            Arc::clone(&asr_runtime),
            Arc::clone(&vad_runtime),
        ));
        let llm_runtime = Arc::new(LlmRuntime::new(
            providers.llm_provider(),
            config.limits.llm_concurrency,
            Duration::from_millis(
                config
                    .providers
                    .llm
                    .openai
                    .as_ref()
                    .expect("validated OpenAI config")
                    .timeout_ms,
            ),
        ));
        let tts_runtime = Arc::new(TtsWorkerRuntime::new(
            providers.tts_provider(),
            WorkerRuntimeConfig {
                max_workers: config.workers.tts.max_workers,
                command_capacity: config.workers.tts.command_queue_capacity,
                final_timeout: Duration::from_millis(config.tts.timeout_ms),
                cleanup_grace: Duration::from_millis(config.workers.tts.cleanup_grace_ms),
            },
        ));
        let active_turn_limiter = Arc::new(ActiveTurnLimiter::new(config.limits.max_active_turns));
        Self {
            config: Arc::new(config),
            providers,
            asr_runtime,
            vad_runtime,
            llm_runtime,
            tts_runtime,
            worker_supervisor,
            active_turn_limiter,
        }
    }
}
