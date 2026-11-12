use super::*;
use crate::{
    audio::VadSegmenterConfig,
    config::AppConfig,
    config::EffectiveProviderBindings,
    database::{AdmittedAssignment, AdmittedProviderBinding},
    providers::{
        RuntimeCatalog, asr::UnavailableAsr, llm::UnavailableLlm, tts::UnavailableTts,
        vad::UnavailableVad,
    },
    session::profile::resolve_effective_session_profile,
    workers::{
        AsrWorkerRuntime, LlmRuntime, TtsWorkerRuntime, VadWorkerRuntime, WorkerRuntimeConfig,
    },
};
use std::{collections::HashMap, sync::Arc};

fn deployment() -> AppConfig {
    toml::from_str(
        r#"
            [server]
            bind = "127.0.0.1:0"
            public_ws_url = "ws://127.0.0.1:0/voice/v1/"

            [provider_defaults]
            vad = "vad"
            asr = "asr"
            llm = "llm"
            tts = "tts"
            "#,
    )
    .expect("the fixture configuration is valid")
}

fn worker() -> WorkerRuntimeConfig {
    WorkerRuntimeConfig {
        max_workers: 1,
        voice_reserved_capacity: 1,
        command_capacity: 1,
        final_timeout: std::time::Duration::from_secs(1),
        cleanup_grace: std::time::Duration::from_secs(1),
    }
}

/// Carries a second LLM and VAD instance so a switch is observable as a runtime change and not
/// only as a prompt change.
pub(super) fn catalog() -> RuntimeCatalog {
    let segmenter = VadSegmenterConfig::default();
    RuntimeCatalog {
        speaker: HashMap::new(),
        vad: HashMap::from([
            (
                "vad".to_owned(),
                crate::providers::LoadedVad {
                    runtime: Arc::new(VadWorkerRuntime::new(Arc::new(UnavailableVad), worker())),
                    segmenter,
                    pre_roll_samples: 4_800,
                },
            ),
            (
                "alternate-vad".to_owned(),
                crate::providers::LoadedVad {
                    runtime: Arc::new(VadWorkerRuntime::new(Arc::new(UnavailableVad), worker())),
                    segmenter,
                    pre_roll_samples: 9_600,
                },
            ),
        ]),
        asr: HashMap::from([(
            "asr".to_owned(),
            Arc::new(AsrWorkerRuntime::new(Arc::new(UnavailableAsr), worker())),
        )]),
        llm: HashMap::from([
            (
                "llm".to_owned(),
                Arc::new(LlmRuntime::new(
                    Arc::new(UnavailableLlm),
                    1,
                    std::time::Duration::from_secs(1),
                )),
            ),
            (
                "alternate".to_owned(),
                Arc::new(LlmRuntime::new(
                    Arc::new(UnavailableLlm),
                    1,
                    std::time::Duration::from_secs(1),
                )),
            ),
        ]),
        tts: HashMap::from([(
            "tts".to_owned(),
            Arc::new(TtsWorkerRuntime::new(Arc::new(UnavailableTts), worker())),
        )]),
    }
}

fn candidate(
    template_id: i64,
    key: &str,
    prompt: &str,
    llm: &str,
    vad: &str,
) -> AdmittedAssignment {
    AdmittedAssignment {
        template_id,
        template_key: key.to_owned(),
        template_name: key.to_owned(),
        language: "vi-VN".to_owned(),
        prompt: prompt.to_owned(),
        template_enabled: true,
        template_revision: 2,
        is_default: template_id == 1,
        assignment_enabled: true,
        bindings: [("vad", vad), ("asr", "asr"), ("llm", llm), ("tts", "tts")]
            .into_iter()
            .map(|(provider_type, provider_key)| AdmittedProviderBinding {
                provider_type: provider_type.to_owned(),
                provider_key: provider_key.to_owned(),
                provider_enabled: true,
                snapshot: None,
            })
            .collect(),
    }
}

/// A session admitted to the default Template and one switchable candidate.
pub(super) fn admitted_actor(catalog: &RuntimeCatalog) -> SessionActor {
    let profile = resolve_effective_session_profile(
        1,
        2,
        "agent",
        &[
            candidate(1, "primary", "primary prompt", "llm", "vad"),
            candidate(7, "usable", "usable prompt", "alternate", "alternate-vad"),
        ],
        &deployment(),
        catalog,
    )
    .expect("the default template resolves against the loaded catalog");
    let admitted = profile.into_admitted_profile();
    let (active, switch_catalog) = (admitted.active, admitted.switch_catalog);
    let bound = catalog
        .resolve(&EffectiveProviderBindings {
            vad: "vad".to_owned(),
            asr: "asr".to_owned(),
            llm: "llm".to_owned(),
            tts: "tts".to_owned(),
            vision: None,
            speaker: None,
        })
        .expect("the default bindings resolve");
    let (control_tx, _control_rx) = mpsc::channel(4);
    let (audio_tx, _audio_rx) = mpsc::channel(4);
    SessionActor::new_with_runtimes_and_limiter(
        "session".to_owned(),
        control_tx,
        audio_tx,
        16,
        20,
        SessionRuntimes {
            job_submit: None,
            asr: bound.asr,
            vad: bound.vad,
            llm: bound.llm,
            tts: bound.tts,
            active_turn_limiter: Arc::new(ActiveTurnLimiter::new(1)),
            vad_segmenter_config: bound.vad_segmenter,
            pre_roll_samples: bound.vad_pre_roll_samples,
        },
    )
    .expect("the session audio runtime initializes")
    .with_effective_profile(active, switch_catalog, admitted.external_mcp, 4_096)
    .expect("the admitted prompt is within the bound")
}

/// A worker lease belongs to the runtime that granted it, so a switch that rebases capture
/// onto another already-loaded VAD must close the old lease there and re-arm, never hand it
/// to the new runtime or leave an Auto client without capture.
///
/// The overwhelmingly common switch keeps VAD, ASR and TTS on the same instances. Their leases
/// and mailboxes must then be left completely alone.
use crate::tools::external_mcp::{
    ExternalToolCatalog, ResolvedExternalMcp, ResolvedExternalTool, SessionExternalMcp,
    normalize_external_tool_segment,
};

/// A server handle exactly as admission produces one, from a client that is already resolved.
///
/// The credential, if there is one, lives inside the client handle and nowhere the session can
/// reach — which is the whole reason the executor is handed this and not a resolver.
fn resolved_server(
    server_key: &str,
    original_name: &str,
    client: crate::tools::external_mcp::ExternalMcpClient,
) -> ResolvedExternalMcp {
    // Admission names the namespace from the normalized key, never from the raw one, so a
    // hand-built handle here is named the same way a resolved one would be.
    let namespace = format!(
        "external.{}",
        normalize_external_tool_segment(server_key).expect("a server key normalizes")
    );
    let published = ExternalToolCatalog::publish(
        &namespace,
        vec![(
            original_name.to_owned(),
            format!("{original_name} tool"),
            serde_json::json!({"type": "object"}),
        )],
    )
    .expect("a single tool publishes");
    ResolvedExternalMcp {
        server_key: server_key.to_owned(),
        namespace,
        client: Arc::new(client),
        tools: Arc::from(published.tools().to_vec()),
        call_timeout: std::time::Duration::from_secs(1),
        limiter: Arc::new(crate::tools::external_mcp::ExternalMcpCallLimiter::new(16)),
    }
}

/// A catalog resolved once against an allowlisted name, without any network work.
fn admitted_external_mcp() -> SessionExternalMcp {
    struct Fixed;
    impl crate::database::secrets::SecretResolver for Fixed {
        fn resolve(
            &self,
            _: &crate::database::secrets::SecretRef,
        ) -> Result<
            crate::database::secrets::SecretValue,
            crate::database::secrets::SecretResolveError,
        > {
            Ok(crate::database::secrets::SecretValue::new("s3cr3t".into()))
        }
    }
    let reference = crate::database::secrets::SecretRef::parse("WEATHER_TOKEN".into())
        .expect("an opaque reference parses");
    let client = crate::tools::external_mcp::ExternalMcpClient::connect(
        "home-assistant",
        "https://mcp.internal.test/rpc",
        "{}",
        "bearer",
        None,
        Some(&reference),
        std::time::Duration::from_secs(1),
        std::time::Duration::from_secs(1),
        reqwest::Client::new(),
        crate::config::ExternalMcpNetworkConfig {
            allowed_hosts: vec!["mcp.internal.test".into()],
        },
        &crate::config::ExternalMcpLimitsConfig::default(),
        Arc::new(crate::telemetry::TracingTelemetry),
        &Fixed,
    )
    .expect("an allowlisted destination produces a client");
    SessionExternalMcp::new(vec![resolved_server(
        "home-assistant",
        "Light/Turn-On",
        client,
    )])
}

pub(super) fn session_with_external_mcp(external_mcp: SessionExternalMcp) -> SessionActor {
    let catalog = catalog();
    let profile = resolve_effective_session_profile(
        1,
        2,
        "agent",
        &[candidate(1, "primary", "primary prompt", "llm", "vad")],
        &deployment(),
        &catalog,
    )
    .expect("the default template resolves against the loaded catalog");
    let admitted = profile
        .with_external_mcp(external_mcp)
        .into_admitted_profile();
    let bound = catalog
        .resolve(&EffectiveProviderBindings {
            vad: "vad".to_owned(),
            asr: "asr".to_owned(),
            llm: "llm".to_owned(),
            tts: "tts".to_owned(),
            vision: None,
            speaker: None,
        })
        .expect("the default bindings resolve");
    let (control_tx, _control_rx) = mpsc::channel(4);
    let (audio_tx, _audio_rx) = mpsc::channel(4);
    SessionActor::new_with_runtimes_and_limiter(
        "session".to_owned(),
        control_tx,
        audio_tx,
        16,
        20,
        SessionRuntimes {
            job_submit: None,
            asr: bound.asr,
            vad: bound.vad,
            llm: bound.llm,
            tts: bound.tts,
            active_turn_limiter: Arc::new(ActiveTurnLimiter::new(1)),
            vad_segmenter_config: bound.vad_segmenter,
            pre_roll_samples: bound.vad_pre_roll_samples,
        },
    )
    .expect("the session audio runtime initializes")
    .with_effective_profile(
        admitted.active,
        admitted.switch_catalog,
        admitted.external_mcp,
        4_096,
    )
    .expect("the admitted prompt is within the bound")
}

// A round that starts after the application closed its gate executes nothing at all.
//
// The gate is what shutdown depends on, so this is the property that makes the drain
// meaningful: a session that never observed the shutdown signal still cannot put a call on the
// network once the application has stopped accepting work.
//
// A round that was already running when the gate closed cannot use the next slot in its own
// budget to start another call.
//
// This is the half the round-start check cannot cover: the call already on the network was
// permitted, and it still keeps its own paired result.  Only what would have been sent after
// the gate closed is refused.

// A Voice Session runs with exactly the External MCP snapshot admission resolved: the
// namespaced names it may call, and the original wire name each one is called with.

// Nothing a session holds renders a credential: the handle owns it and prints none of it.

// The published names are the guide's fixed namespace, and the tool behind one keeps the
// original wire name it is actually called with.

// ---------------------------------------------------------------------------
// The late-response window
// ---------------------------------------------------------------------------

/// A local External MCP server whose `tools/call` answer is held until the test opens it.
///
/// A gate rather than a sleep is what makes the window below reachable on purpose: the test
/// chooses the instant the response becomes available, instead of hoping a timer lines up.
pub(super) struct GatedServer {
    url: String,
    held: Arc<tokio::sync::Notify>,
    opener: watch::Sender<bool>,
    #[allow(dead_code)]
    task: tokio::task::JoinHandle<()>,
}

impl GatedServer {
    async fn start() -> Self {
        Self::answering("late").await
    }

    /// `answer` is the text of the tool result, once the test opens the gate.
    pub(super) async fn answering(answer: &'static str) -> Self {
        let held = Arc::new(tokio::sync::Notify::new());
        let (opener, open) = watch::channel(false);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("a loopback listener binds");
        let address = listener.local_addr().expect("the listener has an address");
        let route_held = Arc::clone(&held);
        let route_open = open.clone();
        let router = axum::Router::new().route(
            "/mcp",
            axum::routing::post(move |body: axum::body::Bytes| {
                let held = Arc::clone(&route_held);
                let mut open = route_open.clone();
                async move {
                    use axum::response::IntoResponse;

                    let request: serde_json::Value =
                        serde_json::from_slice(&body).unwrap_or_default();
                    let id = request.get("id").cloned();
                    let method = request.get("method").and_then(serde_json::Value::as_str);
                    if id.is_none()
                        || method.is_some_and(|method| method.starts_with("notifications/"))
                    {
                        return axum::http::StatusCode::ACCEPTED.into_response();
                    }
                    let result = match method {
                        Some("initialize") => serde_json::json!({
                            "protocolVersion": "2024-11-05",
                            "capabilities": {"tools": {}},
                            "serverInfo": {"name": "gated", "version": "1"}
                        }),
                        Some("tools/list") => serde_json::json!({"tools": [{
                            "name": "Forecast",
                            "description": "forecast",
                            "inputSchema": {"type": "object", "properties": {},
                                            "additionalProperties": false}
                        }]}),
                        Some("tools/call") => {
                            held.notify_one();
                            while !*open.borrow_and_update() {
                                open.changed().await.ok();
                            }
                            serde_json::json!({
                                "content": [{"type": "text", "text": answer}],
                                "isError": false
                            })
                        }
                        _ => serde_json::json!({}),
                    };
                    let mut document = serde_json::json!({"jsonrpc": "2.0", "result": result});
                    if let Some(id) = id {
                        document["id"] = id;
                    }
                    (
                        axum::http::StatusCode::OK,
                        [(
                            axum::http::header::CONTENT_TYPE,
                            "application/json".to_owned(),
                        )],
                        document.to_string(),
                    )
                        .into_response()
                }
            }),
        );
        let task = tokio::spawn(async move {
            axum::serve(listener, router).await.ok();
        });
        Self {
            url: format!("http://{address}/mcp"),
            held,
            opener,
            task,
        }
    }

    async fn wait_until_held(&self) {
        tokio::time::timeout(std::time::Duration::from_secs(5), self.held.notified())
            .await
            .expect("the fixture receives the in-flight tools/call within its bound");
    }

    pub(super) fn release(&self) {
        let _ = self.opener.send(true);
    }
}

/// A snapshot whose calls report into a sink the test can read.
pub(super) async fn gated_snapshot(
    server: &GatedServer,
    telemetry: Arc<crate::telemetry::RecordingTelemetry>,
) -> SessionExternalMcp {
    let mut client = crate::tools::external_mcp::ExternalMcpClient::connect(
        "weather",
        &server.url,
        "{}",
        "none",
        None,
        None,
        std::time::Duration::from_secs(1),
        std::time::Duration::from_secs(2),
        reqwest::Client::new(),
        crate::config::ExternalMcpNetworkConfig {
            allowed_hosts: vec![],
        },
        &crate::config::ExternalMcpLimitsConfig::default(),
        telemetry,
        &crate::database::secrets::EnvSecretResolver,
    )
    .expect("an allowlisted loopback destination produces a client");
    client
        .initialize()
        .await
        .expect("the fixture completes the RMCP initialize lifecycle before a call");
    let database = crate::database::Database::connect(&crate::config::DatabaseConfig {
        url: "sqlite::memory:".into(),
        max_connections: 1,
        ..Default::default()
    })
    .await
    .expect("the fixture database migrates");
    sqlx::raw_sql(
        r#"
INSERT INTO agents (id,key,name,created_at,updated_at) VALUES (2,'agent','Agent',1,1);
INSERT INTO mcp_servers (id,key,name,url,created_at,updated_at) VALUES (1,'weather','Weather','http://fixture.test/mcp',1,1);
INSERT INTO agent_mcp_bindings (agent_id,mcp_server_id,created_at) VALUES (2,1,1);
INSERT INTO external_tool_observations (server_id,original_name,description,input_schema,fingerprint,server_revision,observed_at) VALUES (1,'Forecast','forecast','{}','fixture',1,1);
INSERT INTO agent_external_tool_allowlist (agent_id,server_id,original_name,fingerprint,allowed,sensitive) VALUES (2,1,'Forecast','fixture',1,0);
"#,
    )
    .execute(database.pool())
    .await
    .expect("the fixture allowlist is admitted");
    let mut catalog = SessionExternalMcp::new(vec![resolved_server("weather", "Forecast", client)]);
    catalog.guard = Some(crate::database::tool_security::ExternalToolGuard {
        database,
        agent_id: 2,
        close: Arc::new(tokio_util::sync::CancellationToken::new()),
        contracts: Arc::new(std::collections::HashMap::from([(
            ("weather".to_owned(), "Forecast".to_owned()),
            (1, "fixture".to_owned()),
        )])),
    });
    catalog
}

fn counted(telemetry: &Arc<crate::telemetry::RecordingTelemetry>, metric: &str) -> usize {
    telemetry
        .recorded()
        .iter()
        .filter(|entry| entry.metric == metric)
        .count()
}

/// Waits for one observation, which is the exit condition rather than a delay chosen in
/// advance of knowing how long the observation takes.
async fn await_count(telemetry: &Arc<crate::telemetry::RecordingTelemetry>, metric: &str) {
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            if counted(telemetry, metric) > 0 {
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(1)).await;
        }
    })
    .await
    .unwrap_or_else(|_| panic!("{metric} is observed within its own budget"));
}

/// The completed tool pairs the turn is holding, in order.
fn tool_round(actor: &SessionActor) -> Vec<String> {
    actor
        .llm_messages
        .iter()
        .filter_map(|message| match message {
            ChatMessage::ToolResult { tool_call_id, .. } => Some(tool_call_id.clone()),
            _ => None,
        })
        .collect()
}

// A response that lands after the round that started it is gone becomes nothing at all.
//
// This is the window a socket test cannot order: in production the call's task posts its
// completion into the session's bounded mailbox, and an abort arriving before the next drain
// takes the round away first.  The test reproduces that end state exactly — the round is gone
// before the response is even allowed to exist — so the executor has only one thing it could
// possibly do with what arrives, and the assertion is on what it does instead.
//
// An interrupted turn drops the call it had in flight rather than waiting for it.
//
// The turn's own cancellation is what the executor observes, so this goes through the same
// `cancel_llm` seam a barge-in, a client abort and a shutdown all take.

struct SwitchBuilder(RuntimeCatalog);
struct SwitchResource(RuntimeCatalog);
impl crate::services::provider_runtime::RuntimeResource for SwitchResource {
    fn unload(&self) -> bool {
        true
    }
    fn runtimes(&self) -> Option<RuntimeCatalog> {
        Some(self.0.clone())
    }
}
impl crate::services::provider_runtime::RuntimeMaterializer for SwitchBuilder {
    fn estimated_peak_bytes(
        &self,
        _: &crate::database::DesiredProvider,
    ) -> Result<u64, crate::services::provider_runtime::RuntimeError> {
        Ok(1)
    }
    fn logical_capacity(
        &self,
        _: &crate::database::DesiredProvider,
    ) -> Result<usize, crate::services::provider_runtime::RuntimeError> {
        Ok(1)
    }
    fn build(
        &self,
        snapshot: &crate::database::DesiredProvider,
        _: Option<crate::services::provider_runtime::PreparedRuntime>,
        _: crate::workers::ProviderRuntimeAdmission,
    ) -> Result<
        Arc<dyn crate::services::provider_runtime::RuntimeResource>,
        crate::services::provider_runtime::RuntimeError,
    > {
        let mut catalog = RuntimeCatalog::default();
        let key = snapshot.key.clone();
        match snapshot.kind.as_str() {
            "vad" => {
                catalog.vad.insert(key, self.0.vad[&snapshot.key].clone());
            }
            "asr" => {
                catalog.asr.insert(key, self.0.asr[&snapshot.key].clone());
            }
            "llm" => {
                catalog.llm.insert(key, self.0.llm[&snapshot.key].clone());
            }
            "tts" => {
                catalog.tts.insert(key, self.0.tts[&snapshot.key].clone());
            }
            "speaker" => {
                catalog
                    .speaker
                    .insert(key, self.0.speaker[&snapshot.key].clone());
            }
            _ => return Err(crate::services::provider_runtime::RuntimeError::Configuration),
        }
        Ok(Arc::new(SwitchResource(catalog)))
    }
}
fn managed_switch_actor() -> (
    SessionActor,
    Arc<crate::services::provider_runtime::ProviderRuntimeManager>,
) {
    let catalog = catalog();
    let manager = switch_manager(catalog.clone());
    let mut actor = admitted_actor(&catalog);
    let mut assignment = candidate(7, "cold", "cold prompt", "alternate", "alternate-vad");
    for (index, binding) in assignment.bindings.iter_mut().enumerate() {
        binding.snapshot = Some(Arc::new(crate::database::DesiredProvider {
            id: index as i64 + 1,
            key: binding.provider_key.clone(),
            kind: binding.provider_type.clone(),
            adapter: "fixture".into(),
            revision: 2,
            config_json: "{}".into(),
            secret_ref: None,
        }));
    }
    actor
        .switch_catalog
        .cold
        .push(crate::session::ConfiguredTemplateProfile::from_assignment(&assignment).unwrap());
    actor.switch_catalog.manager = Some(manager.clone());
    (actor, manager)
}

struct CloseGateVad {
    entered: std::sync::mpsc::Sender<()>,
    release: Arc<std::sync::Mutex<std::sync::mpsc::Receiver<()>>>,
}
struct CloseGateVadSession {
    entered: std::sync::mpsc::Sender<()>,
    release: Arc<std::sync::Mutex<std::sync::mpsc::Receiver<()>>>,
}
impl crate::providers::VadProvider for CloseGateVad {
    fn adapter(&self) -> &'static str {
        "close-gate"
    }
    fn open(&self) -> Result<Box<dyn crate::providers::VadSession>, crate::providers::VadError> {
        Ok(Box::new(CloseGateVadSession {
            entered: self.entered.clone(),
            release: self.release.clone(),
        }))
    }
}
impl crate::providers::VadSession for CloseGateVadSession {
    fn push(
        &mut self,
        _: crate::providers::VadInput,
    ) -> Result<crate::providers::VadProbability, crate::providers::VadError> {
        unreachable!()
    }
    fn reset(&mut self) -> Result<(), crate::providers::VadError> {
        Ok(())
    }
    fn close(&mut self) -> Result<(), crate::providers::VadError> {
        self.entered.send(()).unwrap();
        self.release.lock().unwrap().recv().unwrap();
        Ok(())
    }
}

struct HeldSwitchBuilder {
    catalog: RuntimeCatalog,
    entered: tokio::sync::mpsc::UnboundedSender<()>,
    release: std::sync::Mutex<std::sync::mpsc::Receiver<()>>,
}
impl crate::services::provider_runtime::RuntimeMaterializer for HeldSwitchBuilder {
    fn estimated_peak_bytes(
        &self,
        _: &crate::database::DesiredProvider,
    ) -> Result<u64, crate::services::provider_runtime::RuntimeError> {
        Ok(1)
    }
    fn logical_capacity(
        &self,
        _: &crate::database::DesiredProvider,
    ) -> Result<usize, crate::services::provider_runtime::RuntimeError> {
        Ok(1)
    }
    fn build(
        &self,
        _: &crate::database::DesiredProvider,
        _: Option<crate::services::provider_runtime::PreparedRuntime>,
        _: crate::workers::ProviderRuntimeAdmission,
    ) -> Result<
        Arc<dyn crate::services::provider_runtime::RuntimeResource>,
        crate::services::provider_runtime::RuntimeError,
    > {
        self.entered.send(()).unwrap();
        self.release.lock().unwrap().recv().unwrap();
        Ok(Arc::new(SwitchResource(self.catalog.clone())))
    }
}

fn switch_manager(
    catalog: RuntimeCatalog,
) -> Arc<crate::services::provider_runtime::ProviderRuntimeManager> {
    use crate::services::provider_runtime::{ProviderRuntimeManager, RuntimeLimits};
    ProviderRuntimeManager::new(
        RuntimeLimits {
            max_parallel_loads: 1,
            max_pending_loads: 4,
            max_waiters: 8,
            max_resident_bytes: 8,
            max_resources: 8,
            max_version_entries: 16,
            admission_timeout_ms: 1000,
            failure_cooldown_ms: 10,
            idle_ttl_ms: 100,
        },
        Arc::new(SwitchBuilder(catalog)),
        AdmissionGate::open(),
    )
    .unwrap()
}

struct SwitchSpeaker;
impl crate::providers::speaker::SpeakerProvider for SwitchSpeaker {
    fn dimension(&self) -> usize {
        2
    }
    fn extract(
        &mut self,
        _: &crate::audio::PcmF32Mono,
    ) -> Result<Vec<f32>, crate::providers::speaker::SpeakerError> {
        Ok(vec![1.0, 0.0])
    }
}

mod external_mcp;
mod managed_switch;
mod profile_switch;
mod tool_rounds;
mod tool_status;
