use crate::{
    config::{AppConfig, SileroOnnxConfig, VadInstanceConfig},
    database::{
        Database, DeviceAdmissionError,
        history::{HistoryArchive, HistoryWriter, HistoryWriterMetrics, TranscriptCapture},
        secrets::{EnvSecretResolver, SecretResolver},
    },
    lifecycle::{AdmissionGate, Readiness, RuntimeLifecycle},
    providers::{
        DatabaseRuntimeSnapshot, LoadedProviders, ProviderCatalog, ProviderSet, RuntimeCatalog,
    },
    services::provider_diagnostic::{ProviderDiagnosticLimiter, ProviderDiagnosticService},
    session::{
        ActiveTurnLimiter, EffectiveSessionProfile, ProfileUnavailable, WriterOutcomeProbe,
        resolve_effective_session_profile,
    },
    telemetry::TracingTelemetry,
    tools::external_mcp::{ExternalMcpManager, ExternalMcpSnapshot, SessionExternalMcp},
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
    /// One process-wide diagnostic boundary shared by all future Admin provider-test routes.
    pub provider_diagnostics: Arc<ProviderDiagnosticService>,
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
    /// The application-owned lifecycle: one admission/work gate, the drain registry, and the
    /// ordered shutdown that uses them.  A session holds nothing of it but its own registration.
    pub lifecycle: Arc<RuntimeLifecycle>,
    /// Test-only; see [`WriterOutcomeProbe`]. Production leaves it unset.
    pub writer_outcome_probe: Option<Arc<dyn WriterOutcomeProbe>>,
}

impl AppState {
    /// The admission/work gate every new piece of work passes before it starts.
    pub fn admission_gate(&self) -> &Arc<AdmissionGate> {
        self.lifecycle.gate()
    }

    /// Registers this connection with the drain registry before it begins work.
    ///
    /// A session that never registers cannot be observed draining and cannot be issued a
    /// controlled close, so registration happens here rather than anywhere the session decides it
    /// has become worth counting.
    pub fn register_session(&self) -> crate::lifecycle::DrainRegistration {
        self.lifecycle.drain().register()
    }

    /// Whether this process can accept a *new* Voice connection right now.
    ///
    /// This is the whole of readiness, and what it deliberately does not do is the point: it never
    /// resolves a Device, never runs full admission, never discovers or calls an External MCP
    /// server, and never reloads a provider model or a secret.  Those are per-session costs with
    /// per-session failure modes, and folding any of them in here would make an orchestrator's
    /// health check the most expensive thing the process does.
    ///
    /// The checks are, in order: the application lifecycle is still accepting work, startup and
    /// schema are authoritative, the runtimes the deployment requires are present, the admission
    /// resolver is operational, and the database answers when an active feature needs it.  The
    /// database question is one `SELECT 1` — the pool can still produce a connection and the file
    /// is still there — so a busy database or a degraded archive shows up here as reachable while
    /// an actually-unreachable one does not.
    pub async fn readiness(&self) -> Readiness {
        if !self.lifecycle.gate().is_open() {
            return Readiness::ShuttingDown;
        }
        if self.database.is_none() && self.config.database.enabled {
            // The database is configured but this process never opened one, so nothing about the
            // schema is authoritative.  Startup refused to bind in that case; reaching here means
            // a test or an embedder constructed the state directly, and it is still not ready.
            return Readiness::StartupIncomplete;
        }
        if self.required_runtime_missing() {
            return Readiness::RequiredRuntimeUnavailable;
        }
        if let Some(database) = self.admission_database()
            && database.is_reachable().await.is_err()
        {
            return Readiness::DatabaseUnreachable;
        }
        Readiness::Ready
    }

    /// The database a new Voice connection's admission depends on, or `None` when admission does not
    /// depend on one.
    ///
    /// Database-backed admission is the only feature whose failure refuses a *new* Voice Session, so
    /// it is the only one this reaches for.  The Admin API and the Persistent Transcript also need
    /// the database, but their failure is coarse for the caller rather than process-wide: an admin
    /// request that cannot reach it is answered 503 and an archive write is dropped, neither of
    /// which is a reason to pull the Voice listener out of an orchestrator's rotation.
    fn admission_database(&self) -> Option<&Arc<Database>> {
        if !self.config.database.devices.admission_enabled {
            return None;
        }
        self.database.as_ref()
    }

    /// Whether a provider instance this deployment's own server defaults require is missing.
    ///
    /// This is a map lookup against the already-loaded RuntimeCatalog.  It prepares nothing, opens
    /// no model and resolves no secret — a required provider that could not be materialized never
    /// reached the listener in the first place, so this only catches state assembled some other
    /// way.
    fn required_runtime_missing(&self) -> bool {
        let providers = &self.config.effective_agent.providers;
        self.runtimes.resolve(providers).is_err()
    }

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

    /// Deterministic public-HTTP seam for Admin provider diagnostics. It installs an
    /// already-loaded LLM runtime and the immutable startup snapshot a database-backed provider
    /// test compares against; it never materializes a provider from an Admin request.
    pub fn with_database_llm_runtime_for_test(
        mut self,
        instance_id: impl Into<String>,
        provider: Arc<dyn crate::providers::LlmProvider>,
        concurrency: usize,
        timeout: Duration,
        desired_revision: i64,
    ) -> Self {
        let instance_id = instance_id.into();
        Arc::make_mut(&mut self.runtimes).llm.insert(
            instance_id.clone(),
            Arc::new(LlmRuntime::new(provider, concurrency, timeout)),
        );
        self.database_runtime_snapshot = Some(Arc::new(DatabaseRuntimeSnapshot::from_states([(
            instance_id,
            crate::providers::DatabaseRuntimeState {
                provider_id: 0,
                desired_revision,
                status: crate::providers::DatabaseRuntimeStatus::Loaded,
                failure: None,
            },
        )])));
        self.provider_diagnostics = Arc::new(ProviderDiagnosticService::new(
            Arc::clone(&self.runtimes),
            self.database_runtime_snapshot.clone(),
            self.database.clone(),
            ProviderDiagnosticLimiter::new(self.config.api.provider_tests.max_concurrency),
            Duration::from_millis(self.config.api.provider_tests.timeout_ms),
        ));
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
        if !self.lifecycle.gate().is_open() {
            // The application has stopped accepting work.  This is checked here rather than only
            // at the WebSocket boundary so that no caller can start a database admission after
            // shutdown has begun, whatever route it took to reach this seam.
            return Err(SessionProfileAdmissionError::ShuttingDown);
        }
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
        let lifecycle = RuntimeLifecycle::from_config(&config);
        Self::new_with_database_and_shutdown(config, loaded, database, lifecycle)
    }

    pub fn new_with_database_and_shutdown(
        config: AppConfig,
        loaded: LoadedProviders,
        database: Option<Database>,
        lifecycle: Arc<RuntimeLifecycle>,
    ) -> Self {
        Self::new_with_database_runtime_snapshot_and_shutdown(
            config, loaded, database, None, lifecycle,
        )
    }

    pub fn new_with_database_runtime_snapshot_and_shutdown(
        config: AppConfig,
        loaded: LoadedProviders,
        database: Option<Database>,
        database_runtime_snapshot: Option<DatabaseRuntimeSnapshot>,
        lifecycle: Arc<RuntimeLifecycle>,
    ) -> Self {
        Self::new_with_database_runtime_snapshot_resolver_and_shutdown(
            config,
            loaded,
            database,
            database_runtime_snapshot,
            Arc::new(EnvSecretResolver),
            lifecycle,
        )
    }

    pub fn new_with_database_runtime_snapshot_resolver_and_shutdown(
        config: AppConfig,
        loaded: LoadedProviders,
        database: Option<Database>,
        database_runtime_snapshot: Option<DatabaseRuntimeSnapshot>,
        secret_resolver: Arc<dyn SecretResolver>,
        lifecycle: Arc<RuntimeLifecycle>,
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
        // The gate lives in the lifecycle rather than in the manager, so a limiter refuses a permit
        // for exactly the same reason the WebSocket boundary refuses a connection.
        let external_mcp = shared_external_mcp(&config, lifecycle.gate());
        // The archive exists whenever the database does, even with capture off: retention keeps
        // running over whatever an earlier deployment already stored.
        let history = database.as_ref().map(|database| {
            Arc::new(HistoryArchive::start(
                database,
                &config.database.history,
                lifecycle.stopping().clone(),
            ))
        });
        let config = Arc::new(config);
        let runtimes = Arc::new(loaded.runtimes);
        let database_runtime_snapshot = database_runtime_snapshot.map(Arc::new);
        let provider_diagnostics = Arc::new(ProviderDiagnosticService::new(
            Arc::clone(&runtimes),
            database_runtime_snapshot.clone(),
            database.as_ref().cloned().map(Arc::new),
            ProviderDiagnosticLimiter::new(config.api.provider_tests.max_concurrency),
            Duration::from_millis(config.api.provider_tests.timeout_ms),
        ));
        let state = Self {
            active_turn_limiter: Arc::new(ActiveTurnLimiter::new(config.limits.max_active_turns)),
            config,
            providers: Arc::new(loaded.providers),
            runtimes,
            provider_diagnostics,
            worker_supervisor: supervisor,
            database: database.map(Arc::new),
            database_runtime_snapshot,
            secret_resolver,
            external_mcp,
            history,
            lifecycle,
            writer_outcome_probe: None,
        };
        // The archive that owns the writer was only just built, so this is the first moment the
        // shutdown sequence can observe whether there is anything left to flush.  A deployment
        // with capture off registers nothing and its shutdown waits for nothing.
        if let Some(metrics) = state.history_metrics() {
            state.lifecycle.observe_history_writer(metrics);
        }
        state
    }

    /// Binds one admitted connection's Persistent Transcript capture, or `None`.
    ///
    /// Two things decide it, and neither is a query.  The writer exists only when the deployment
    /// opted in, so asking for it *is* the capture policy rather than a second copy of it.  An
    /// admission identity has to exist too: an archive row belongs to the admitted Device and Agent,
    /// and a session admitted without the database has neither.  Deciding both here keeps the
    /// WebSocket boundary from knowing either.
    pub fn transcript_capture(
        &self,
        session_id: &str,
        profile: &EffectiveSessionProfile,
    ) -> Option<TranscriptCapture> {
        TranscriptCapture::new(
            self.history_writer()?,
            session_id,
            profile.device_db_id,
            profile.agent_id,
        )
    }

    /// The archival writer this process owns, or `None`.
    ///
    /// `None` covers both halves of the opt-in: no database, and no capture.  A session that cannot
    /// be handed a writer has no way to enqueue a record, which is the whole boundary.
    pub fn history_writer(&self) -> Option<&HistoryWriter> {
        self.history.as_ref().and_then(|archive| archive.writer())
    }

    /// The archival writer's bounded counters, or `None` when there is no writer to count for.  An
    /// operator or a test reads the archive's behaviour through these; nothing else about the
    /// writer is observable.
    pub fn history_metrics(&self) -> Option<Arc<HistoryWriterMetrics>> {
        Some(Arc::clone(self.history_writer()?.metrics()))
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
        let lifecycle = RuntimeLifecycle::from_config(&config);
        Self::from_provider_set_with_database_and_shutdown(config, providers, database, lifecycle)
    }

    pub fn from_provider_set_with_database_and_shutdown(
        config: AppConfig,
        providers: Arc<ProviderSet>,
        database: Option<Database>,
        lifecycle: Arc<RuntimeLifecycle>,
    ) -> Self {
        Self::from_provider_set_with_database_resolver_and_shutdown(
            config,
            providers,
            database,
            Arc::new(EnvSecretResolver),
            lifecycle,
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
        lifecycle: Arc<RuntimeLifecycle>,
    ) -> Self {
        let (config, loaded) = loaded_from_provider_set(config, &providers);
        Self::new_with_database_runtime_snapshot_resolver_and_shutdown(
            config,
            loaded,
            database,
            None,
            secret_resolver,
            lifecycle,
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
    /// The application is shutting down and is no longer admitting anybody.  A session that was
    /// already admitted keeps running; only a *new* one is refused, which is why this is its own
    /// class rather than a database failure a client could retry through.
    #[error("server_is_shutting_down")]
    ShuttingDown,
}

/// Builds the one process-wide External MCP transport and call limiter.
///
/// External MCP is optional in V1, so a transport that cannot be built degrades to "no External
/// MCP tools" instead of failing a startup that has nothing else to do with the failure.  The
/// reason is logged; the destination and its configuration are not.
///
/// The limiter carries the application admission gate: once shutdown has begun it refuses a new
/// outbound permit, so a Tool-round work item that was queued before the gate closed still cannot
/// put a request on the network afterwards.
fn shared_external_mcp(
    config: &AppConfig,
    gate: &Arc<AdmissionGate>,
) -> Option<Arc<ExternalMcpManager>> {
    match ExternalMcpManager::new_with_telemetry_and_gate(
        &config.mcp.external,
        Arc::new(TracingTelemetry),
        Arc::clone(gate),
    ) {
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
            voice_reserved_capacity: 1,
            command_capacity: config.workers.vad.command_queue_capacity,
            final_timeout: Duration::from_millis(config.workers.vad.reset_timeout_ms),
            cleanup_grace: Duration::from_millis(config.workers.vad.cleanup_grace_ms),
        },
    ));
    let asr_runtime = Arc::new(AsrWorkerRuntime::new(
        Arc::clone(&asr),
        WorkerRuntimeConfig {
            max_workers: config.workers.asr.max_workers,
            voice_reserved_capacity: 1,
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
            voice_reserved_capacity: 1,
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
