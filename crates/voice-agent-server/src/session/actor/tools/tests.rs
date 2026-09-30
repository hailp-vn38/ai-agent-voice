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
        command_capacity: 1,
        final_timeout: std::time::Duration::from_secs(1),
        cleanup_grace: std::time::Duration::from_secs(1),
    }
}

/// Carries a second LLM and VAD instance so a switch is observable as a runtime change and not
/// only as a prompt change.
fn catalog() -> RuntimeCatalog {
    let segmenter = VadSegmenterConfig::default();
    RuntimeCatalog {
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
        vision: HashMap::new(),
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
            })
            .collect(),
    }
}

/// A session admitted to the default Template and one switchable candidate.
fn admitted_actor(catalog: &RuntimeCatalog) -> SessionActor {
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

#[test]
fn a_successful_switch_installs_the_candidate_runetimes_and_advances_the_revision() {
    let catalog = catalog();
    let mut actor = admitted_actor(&catalog);
    let initial_llm = Arc::clone(&actor.llm_runtime);
    let alternate_llm = Arc::clone(&catalog.llm["alternate"]);
    assert_eq!(actor.profile_revision(), 1);

    actor.apply_template_switch("usable");

    assert_eq!(actor.profile_revision(), 2);
    assert_eq!(actor.profile.system_prompt, "usable prompt");
    assert_eq!(actor.profile.language, "vi-VN");
    assert!(
        Arc::ptr_eq(&actor.llm_runtime, &alternate_llm)
            && !Arc::ptr_eq(&actor.llm_runtime, &initial_llm),
        "a switch installs the candidate's already-loaded runtime and never reuses the old one"
    );
}

#[test]
fn a_switch_to_a_candidate_this_session_never_admitted_changes_nothing() {
    let catalog = catalog();
    let mut actor = admitted_actor(&catalog);
    let initial_llm = Arc::clone(&actor.llm_runtime);

    actor.apply_template_switch("never-admitted");

    assert_eq!(actor.profile_revision(), 1);
    assert_eq!(actor.profile.system_prompt, "primary prompt");
    assert!(
        Arc::ptr_eq(&actor.llm_runtime, &initial_llm),
        "a rejected switch must not disturb the active runtimes"
    );
}

/// A worker lease belongs to the runtime that granted it, so a switch that rebases capture
/// onto another already-loaded VAD must close the old lease there and re-arm, never hand it
/// to the new runtime or leave an Auto client without capture.
#[test]
fn a_switch_that_rebases_the_vad_runtime_rearms_capture_instead_of_failing_closed() {
    let catalog = catalog();
    let mut actor = admitted_actor(&catalog);
    actor.start_listening(crate::protocol::ListenMode::Auto);
    assert!(
        actor.vad_session.is_some(),
        "Auto mode arms a VAD capture cycle"
    );

    actor.apply_template_switch("usable");
    assert_eq!(actor.phase, SessionPhase::Listening);

    actor.complete_recognition();

    assert_eq!(
        actor.phase,
        SessionPhase::Listening,
        "an Auto session must keep capturing after a VAD rebasing"
    );
    assert!(
        actor.vad_session.is_some(),
        "capture must be re-armed, not left holding a lease the candidate's runtime never granted"
    );
    assert!(
        Arc::ptr_eq(
            &actor.vad_runtime,
            &catalog.vad["alternate-vad"].runtime.clone()
        ),
        "the re-armed cycle must belong to the candidate's own runtime"
    );
    assert_eq!(actor.pre_roll_samples, 9_600);
}

/// The overwhelmingly common switch keeps VAD, ASR and TTS on the same instances. Their leases
/// and mailboxes must then be left completely alone.
#[test]
fn a_switch_that_keeps_the_capture_runtimes_leaves_their_leases_open() {
    let catalog = catalog();
    let profile = resolve_effective_session_profile(
        1,
        2,
        "agent",
        &[
            candidate(1, "primary", "primary prompt", "llm", "vad"),
            candidate(7, "usable", "usable prompt", "llm", "vad"),
        ],
        &deployment(),
        &catalog,
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
        })
        .expect("the default bindings resolve");
    let (control_tx, _control_rx) = mpsc::channel(4);
    let (audio_tx, _audio_rx) = mpsc::channel(4);
    let mut actor = SessionActor::new_with_runtimes_and_limiter(
        "session".to_owned(),
        control_tx,
        audio_tx,
        16,
        20,
        SessionRuntimes {
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
    .expect("the admitted prompt is within the bound");
    actor.start_listening(crate::protocol::ListenMode::Auto);
    let lease = actor.vad_session.as_ref().map(|(lease, _)| *lease);

    actor.apply_template_switch("usable");

    assert_eq!(actor.profile.revision, 2);
    assert_eq!(
        actor.vad_session.as_ref().map(|(open, _)| *open),
        lease,
        "an unchanged capture runtime must keep the lease the session already holds"
    );
}

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
            allow_http_lan: false,
            allowed_hosts: vec!["mcp.internal.test".into()],
            allowed_cidrs: vec![],
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

fn session_with_external_mcp(external_mcp: SessionExternalMcp) -> SessionActor {
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

/// A round that starts after the application closed its gate executes nothing at all.
///
/// The gate is what shutdown depends on, so this is the property that makes the drain
/// meaningful: a session that never observed the shutdown signal still cannot put a call on the
/// network once the application has stopped accepting work.
#[test]
fn a_closed_gate_starts_no_tool_round_at_all() {
    let gate = AdmissionGate::open();
    let mut actor =
        session_with_external_mcp(admitted_external_mcp()).with_admission_gate(Arc::clone(&gate));
    actor
        .begin_active_turn()
        .expect("an active turn is admitted");
    actor
        .commit_user_text("turn on the light".to_owned())
        .expect("the accepted user text is committed to the turn");

    gate.close();
    actor.start_tool_batch(vec![ToolCall {
        id: "call-1".to_owned(),
        name: "external.home_assistant.light_turn_on".to_owned(),
        arguments: serde_json::json!({}),
    }]);

    assert!(
        actor.tool_batch.is_none(),
        "no round is opened, so no call can be dispatched from it"
    );
    assert!(
        tool_round(&actor).is_empty(),
        "nothing was called, so there is no ToolResult standing in for a call"
    );
}

/// A round that was already running when the gate closed cannot use the next slot in its own
/// budget to start another call.
///
/// This is the half the round-start check cannot cover: the call already on the network was
/// permitted, and it still keeps its own paired result.  Only what would have been sent after
/// the gate closed is refused.
#[tokio::test]
async fn a_closed_gate_stops_a_round_that_was_already_running() {
    let server = GatedServer::start().await;
    let telemetry = Arc::new(crate::telemetry::RecordingTelemetry::default());
    let gate = AdmissionGate::open();
    let mut actor =
        session_with_external_mcp(gated_snapshot(&server, Arc::clone(&telemetry)).await)
            .with_admission_gate(Arc::clone(&gate));
    actor
        .begin_active_turn()
        .expect("an active turn is admitted");
    actor
        .commit_user_text("what is the forecast".to_owned())
        .expect("the accepted user text is committed to the turn");

    let call = |id: &str| ToolCall {
        id: id.to_owned(),
        name: "external.weather.forecast".to_owned(),
        arguments: serde_json::json!({}),
    };
    actor.start_tool_batch(vec![call("call-1"), call("call-2")]);
    server.wait_until_held().await;

    // The gate closes while the first call is still on the network.
    gate.close();
    server.release();

    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            actor.drain_external_call_completions();
            if !tool_round(&actor).is_empty() {
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("the call that was already in flight reports its outcome");
    actor.drain_external_call_completions();

    assert_eq!(
        tool_round(&actor),
        vec!["call-1".to_owned()],
        "the call that had already been sent keeps its paired result, and the second is not \
             paired because it never ran"
    );
    assert!(
        actor.tool_batch.is_none(),
        "the refused call terminalized the round rather than leaving it half-executed"
    );
    assert_eq!(
        counted(&telemetry, crate::telemetry::EXTERNAL_MCP_TOOL_CALLS_TOTAL),
        1,
        "exactly one request reached the network"
    );
}

/// A Voice Session runs with exactly the External MCP snapshot admission resolved: the
/// namespaced names it may call, and the original wire name each one is called with.
#[test]
fn a_session_installs_the_admitted_external_mcp_catalog() {
    let actor = session_with_external_mcp(admitted_external_mcp());
    let installed = actor.session_external_mcp();
    assert_eq!(installed.tool_count(), 1);
    let (server, tool) = installed
        .find("external.home_assistant.light_turn_on")
        .expect("the admitted tool is routable");
    assert_eq!(server.server_key, "home-assistant");
    assert_eq!(server.call_timeout, std::time::Duration::from_secs(1));
    assert_eq!(tool.original_name, "Light/Turn-On");
    // A Device MCP name, or a name from a server this session never admitted, resolves to
    // nothing: routing goes through the origin, not through the visible name.
    assert!(installed.find("self.light_turn_on").is_none());
    assert!(installed.find("external.other.light_turn_on").is_none());
    assert!(installed.find("external.home_assistant.dim").is_none());
}

/// Nothing a session holds renders a credential: the handle owns it and prints none of it.
#[test]
fn a_sessions_external_mcp_snapshot_renders_without_a_credential() {
    let installed = admitted_external_mcp();
    let rendered = format!("{:?}", installed);
    assert!(rendered.contains("external.home_assistant.light_turn_on"));
    assert!(!rendered.contains("s3cr3t"), "{rendered}");
    assert!(!rendered.contains("WEATHER_TOKEN"), "{rendered}");
}

/// The published names are the guide's fixed namespace, and the tool behind one keeps the
/// original wire name it is actually called with.
#[test]
fn external_tool_names_are_namespaced_and_keep_their_wire_name() {
    assert_eq!(
        normalize_external_tool_segment("Home-Assistant").as_deref(),
        Some("home_assistant")
    );
    let published = ExternalToolCatalog::publish(
        "external.home_assistant",
        vec![(
            "Light/Turn-On".to_owned(),
            "turns a light on".to_owned(),
            serde_json::json!({"type": "object"}),
        )],
    )
    .expect("a single tool publishes");
    let tool: &ResolvedExternalTool = &published.tools()[0];
    assert_eq!(tool.llm_name, "external.home_assistant.light_turn_on");
    assert_eq!(tool.original_name, "Light/Turn-On");
}

#[test]
fn a_session_without_a_catalog_is_never_offered_the_switch_tool() {
    let catalog = catalog();
    let bound = catalog
        .resolve(&EffectiveProviderBindings {
            vad: "vad".to_owned(),
            asr: "asr".to_owned(),
            llm: "llm".to_owned(),
            tts: "tts".to_owned(),
            vision: None,
        })
        .expect("the default bindings resolve");
    let (control_tx, _control_rx) = mpsc::channel(4);
    let (audio_tx, _audio_rx) = mpsc::channel(4);
    let actor = SessionActor::new_with_runtimes_and_limiter(
        "session".to_owned(),
        control_tx,
        audio_tx,
        16,
        20,
        SessionRuntimes {
            asr: bound.asr,
            vad: bound.vad,
            llm: bound.llm,
            tts: bound.tts,
            active_turn_limiter: Arc::new(ActiveTurnLimiter::new(1)),
            vad_segmenter_config: bound.vad_segmenter,
            pre_roll_samples: bound.vad_pre_roll_samples,
        },
    )
    .expect("the session audio runtime initializes");

    assert!(
        actor
            .available_llm_tools()
            .iter()
            .all(|tool| tool.name != SWITCH_TEMPLATE_TOOL_NAME),
        "a session with no admission catalog cannot offer a switch"
    );
}

// ---------------------------------------------------------------------------
// The late-response window
// ---------------------------------------------------------------------------

/// A local External MCP server whose `tools/call` answer is held until the test opens it.
///
/// A gate rather than a sleep is what makes the window below reachable on purpose: the test
/// chooses the instant the response becomes available, instead of hoping a timer lines up.
struct GatedServer {
    url: String,
    held: Arc<tokio::sync::Notify>,
    opener: watch::Sender<bool>,
    #[allow(dead_code)]
    task: tokio::task::JoinHandle<()>,
}

impl GatedServer {
    async fn start() -> Self {
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
                                "content": [{"type": "text", "text": "late"}],
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

    fn release(&self) {
        let _ = self.opener.send(true);
    }
}

/// A snapshot whose calls report into a sink the test can read.
async fn gated_snapshot(
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
            allow_http_lan: true,
            allowed_hosts: vec![],
            allowed_cidrs: vec!["127.0.0.0/8".into()],
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
    SessionExternalMcp::new(vec![resolved_server("weather", "Forecast", client)])
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

/// A response that lands after the round that started it is gone becomes nothing at all.
///
/// This is the window a socket test cannot order: in production the call's task posts its
/// completion into the session's bounded mailbox, and an abort arriving before the next drain
/// takes the round away first.  The test reproduces that end state exactly — the round is gone
/// before the response is even allowed to exist — so the executor has only one thing it could
/// possibly do with what arrives, and the assertion is on what it does instead.
#[tokio::test]
async fn a_response_that_lands_after_its_round_is_discarded_and_acts_on_nothing() {
    let server = GatedServer::start().await;
    let telemetry = Arc::new(crate::telemetry::RecordingTelemetry::default());
    let mut actor =
        session_with_external_mcp(gated_snapshot(&server, Arc::clone(&telemetry)).await);
    actor
        .begin_active_turn()
        .expect("an active turn is admitted");
    actor
        .commit_user_text("what is the forecast".to_owned())
        .expect("the accepted user text is committed to the turn");

    actor.start_tool_batch(vec![ToolCall {
        id: "call-1".to_owned(),
        name: "external.weather.forecast".to_owned(),
        arguments: serde_json::json!({}),
    }]);
    assert!(
        actor
            .tool_batch
            .as_ref()
            .is_some_and(|batch| batch.in_flight.is_some()),
        "the round waits for exactly one call, and never more than one"
    );

    server.wait_until_held().await;
    // The turn ends while the call is still in flight and its answer still held, so the
    // response cannot have been applied by anything: the round it belonged to no longer exists.
    actor.cancel_tool_turn();
    assert!(actor.advance_generation());
    server.release();
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            actor.drain_external_call_completions();
            if counted(
                &telemetry,
                crate::telemetry::EXTERNAL_TOOL_LATE_RESPONSE_DISCARDED_TOTAL,
            ) > 0
            {
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(1)).await;
        }
    })
    .await
    .expect("the late response reaches the session's mailbox");

    assert_eq!(
        counted(
            &telemetry,
            crate::telemetry::EXTERNAL_TOOL_LATE_RESPONSE_DISCARDED_TOTAL
        ),
        1,
        "the late response is counted once, against the server that produced it"
    );
    assert!(
        actor.tool_batch.is_none(),
        "a discarded response starts no round, so nothing is left to finish"
    );
    assert!(
        tool_round(&actor).is_empty(),
        "a discarded response produces no ToolResult in the prompt base snapshot"
    );
    let turn_id = actor.current_turn_id().expect("the turn is still open");
    let history = actor
        .dialogue_history
        .messages_for_prompt(turn_id)
        .expect("the turn's history renders");
    assert!(
        history
            .iter()
            .all(|message| !matches!(message, ChatMessage::ToolResult { .. })),
        "a discarded response produces no completed round in history either"
    );
}

/// An interrupted turn drops the call it had in flight rather than waiting for it.
///
/// The turn's own cancellation is what the executor observes, so this goes through the same
/// `cancel_llm` seam a barge-in, a client abort and a shutdown all take.
#[tokio::test]
async fn an_interrupted_turn_drops_its_in_flight_call_without_producing_a_result() {
    let server = GatedServer::start().await;
    let telemetry = Arc::new(crate::telemetry::RecordingTelemetry::default());
    let mut actor =
        session_with_external_mcp(gated_snapshot(&server, Arc::clone(&telemetry)).await);
    actor
        .begin_active_turn()
        .expect("an active turn is admitted");

    actor.start_tool_batch(vec![ToolCall {
        id: "call-1".to_owned(),
        name: "external.weather.forecast".to_owned(),
        arguments: serde_json::json!({}),
    }]);
    server.wait_until_held().await;

    // Cancellation first, then the round: the order every interruption path uses.
    actor.cancel_llm();
    actor.cancel_tool_turn();
    server.release();
    await_count(
        &telemetry,
        crate::telemetry::EXTERNAL_TOOL_CALL_CANCELLED_TOTAL,
    )
    .await;
    actor.drain_external_call_completions();

    assert_eq!(
        counted(
            &telemetry,
            crate::telemetry::EXTERNAL_TOOL_CALL_CANCELLED_TOTAL
        ),
        1,
        "the in-flight call is reported as dropped exactly once"
    );
    assert_eq!(
        counted(
            &telemetry,
            crate::telemetry::EXTERNAL_TOOL_LATE_RESPONSE_DISCARDED_TOTAL
        ),
        0,
        "a call dropped by cancellation is never answered, so nothing is ever discarded"
    );
    assert!(
        tool_round(&actor).is_empty(),
        "a dropped call produces no ToolResult, so no continuation can be built on one"
    );
}
