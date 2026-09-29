use crate::{
    config::{AppConfig, SileroOnnxConfig, VadInstanceConfig},
    database::{
        Database, DeviceAdmissionError,
        history::{HistoryArchive, HistoryWriter, HistoryWriterMetrics, TranscriptCapture},
        secrets::{EnvSecretResolver, SecretResolver},
    },
    providers::{
        DatabaseRuntimeSnapshot, LoadedProviders, ProviderCatalog, ProviderSet, RuntimeCatalog,
    },
    session::{
        ActiveTurnLimiter, EffectiveSessionProfile, ProfileUnavailable, WriterOutcomeProbe,
        resolve_effective_session_profile,
    },
    tools::external_mcp::{ExternalMcpManager, ExternalMcpSnapshot, SessionExternalMcp},
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
    /// Application-owned External MCP resolution: one shared transport and one process-global
    /// per-server call limiter for every session.  `None` only when the shared transport could
    /// not be built, which leaves every bound server fail-soft unavailable.
    pub external_mcp: Option<Arc<ExternalMcpManager>>,
    /// The optional Persistent Transcript: the one archival writer every session shares plus the
    /// retention job.  `None` only when the database is off, in which case no session can archive.
    pub history: Option<Arc<HistoryArchive>>,
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
    ///
    /// External MCP is the one part that is fail-soft in V1: its snapshot is resolved after the
    /// profile and a server that cannot be reached, validated or budgeted simply contributes no
    /// tools, because an unavailable optional capability must never refuse a Voice Session.
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
        let profile = resolve_effective_session_profile(
            graph.device_db_id,
            graph.agent.id,
            &graph.agent.key,
            &graph.assignments,
            &self.config,
            &self.runtimes,
        )
        .map_err(|ProfileUnavailable| SessionProfileAdmissionError::ProfileUnavailable)?;
        Ok(profile.with_external_mcp(self.resolve_external_mcp(graph.agent.id).await))
    }

    /// The fresh External MCP snapshot for one Agent, or an empty one.
    ///
    /// Every failure on this path is fail-soft by contract: a database that cannot answer the
    /// binding read, a transport that could not be built, and a server that will not resolve all
    /// produce a session with fewer tools, never a rejected admission.
    async fn resolve_external_mcp(&self, agent_id: i64) -> SessionExternalMcp {
        let Some(manager) = self.external_mcp.as_ref() else {
            return SessionExternalMcp::default();
        };
        let Some(database) = self.database.as_ref() else {
            return SessionExternalMcp::default();
        };
        let servers = match database.agent_mcp_servers(agent_id).await {
            Ok(servers) => servers,
            Err(_) => {
                tracing::warn!(
                    event = "external_mcp_bindings_unavailable",
                    reason = "database_unavailable",
                    "No External MCP binding could be read; admitting without External MCP tools"
                );
                return SessionExternalMcp::default();
            }
        };
        let ExternalMcpSnapshot { servers, .. } = manager
            .resolve_snapshot(&servers, self.secret_resolver.as_ref())
            .await;
        SessionExternalMcp::new(servers)
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
        let external_mcp = shared_external_mcp(&config);
        // The archive exists whenever the database does, even with capture off: retention keeps
        // running over whatever an earlier deployment already stored.
        let history = database.as_ref().map(|database| {
            Arc::new(HistoryArchive::start(
                database,
                &config.database.history,
                shutdown.clone(),
            ))
        });
        Self {
            active_turn_limiter: Arc::new(ActiveTurnLimiter::new(config.limits.max_active_turns)),
            config: Arc::new(config),
            providers: Arc::new(loaded.providers),
            runtimes: Arc::new(loaded.runtimes),
            worker_supervisor: supervisor,
            database: database.map(Arc::new),
            database_runtime_snapshot: database_runtime_snapshot.map(Arc::new),
            secret_resolver,
            external_mcp,
            history,
            shutdown,
            writer_outcome_probe: None,
        }
    }

    /// Binds one admitted connection's Persistent Transcript capture, or `None`.
    ///
    /// Capture needs the opt-in, the database, and an admission identity: an archive row belongs to
    /// the admitted Device and Agent, and a session admitted without the database has neither.
    /// Deciding it here keeps the WebSocket boundary from knowing any of that, and the answer never
    /// depends on a query — turning capture on is a configuration decision, not a live lookup.
    pub fn transcript_capture(
        &self,
        session_id: &str,
        profile: &EffectiveSessionProfile,
    ) -> Option<TranscriptCapture> {
        if !self.config.database.history.enabled {
            return None;
        }
        TranscriptCapture::new(
            self.history_writer()?,
            session_id,
            profile.device_db_id,
            profile.agent_id,
        )
    }

    /// The archival writer this process owns, or `None` when the database is off.
    pub fn history_writer(&self) -> Option<&HistoryWriter> {
        self.history.as_ref().map(|archive| archive.writer())
    }

    /// The archive's bounded counters.  An operator or a test reads the archive's behaviour
    /// through these; nothing else about the writer is observable.
    pub fn history_metrics(&self) -> Option<Arc<HistoryWriterMetrics>> {
        self.history
            .as_ref()
            .map(|archive| Arc::clone(archive.metrics()))
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
        Self::from_provider_set_with_database_resolver_and_shutdown(
            config,
            providers,
            database,
            Arc::new(EnvSecretResolver),
            shutdown,
        )
    }

    /// Same seam for deployments that own secret resolution.  External MCP resolves its
    /// credentials through this same abstraction, so a deployment that swaps the resolver changes
    /// provider and MCP credentials together rather than one without the other.
    pub fn from_provider_set_with_database_resolver_and_shutdown(
        config: AppConfig,
        providers: Arc<ProviderSet>,
        database: Option<Database>,
        secret_resolver: Arc<dyn SecretResolver>,
        shutdown: CancellationToken,
    ) -> Self {
        let (config, loaded) = loaded_from_provider_set(config, &providers);
        Self::new_with_database_runtime_snapshot_resolver_and_shutdown(
            config,
            loaded,
            database,
            None,
            secret_resolver,
            shutdown,
        )
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

/// Builds the one process-wide External MCP transport and call limiter.
///
/// External MCP is optional in V1, so a transport that cannot be built degrades to "no External
/// MCP tools" instead of failing a startup that has nothing else to do with the failure.  The
/// reason is logged; the destination and its configuration are not.
fn shared_external_mcp(config: &AppConfig) -> Option<Arc<ExternalMcpManager>> {
    match ExternalMcpManager::new(&config.mcp.external) {
        Ok(manager) => Some(Arc::new(manager)),
        Err(error) => {
            tracing::warn!(
                event = "external_mcp_transport_unavailable",
                reason = %error,
                "The shared External MCP transport could not be built; every bound server stays unavailable"
            );
            None
        }
    }
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
