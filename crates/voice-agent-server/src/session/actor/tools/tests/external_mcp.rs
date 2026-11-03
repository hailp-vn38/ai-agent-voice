use super::*;

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

#[test]
fn a_sessions_external_mcp_snapshot_renders_without_a_credential() {
    let installed = admitted_external_mcp();
    let rendered = format!("{:?}", installed);
    assert!(rendered.contains("external.home_assistant.light_turn_on"));
    assert!(!rendered.contains("s3cr3t"), "{rendered}");
    assert!(!rendered.contains("WEATHER_TOKEN"), "{rendered}");
}

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
            speaker: None,
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
    .expect("the session audio runtime initializes");

    assert!(
        actor
            .available_llm_tools()
            .iter()
            .all(|tool| tool.name != SWITCH_TEMPLATE_TOOL_NAME),
        "a session with no admission catalog cannot offer a switch"
    );
}

#[tokio::test]
async fn server_external_mcp_reports_tool_lifecycle_without_client_device_mcp() {
    let server = GatedServer::start().await;
    let telemetry = Arc::new(crate::telemetry::RecordingTelemetry::default());
    let (tx, mut rx) = mpsc::channel(8);
    let mut actor =
        session_with_external_mcp(gated_snapshot(&server, Arc::clone(&telemetry)).await)
            .with_tool_status(true);
    actor.control_tx = tx;
    assert!(
        actor.mcp.visible.is_empty(),
        "web assistants do not advertise Device MCP"
    );

    let turn_id = actor
        .begin_active_turn()
        .expect("an active turn is admitted");
    actor
        .commit_user_text("get the forecast".to_owned())
        .expect("accepted text belongs to the active turn");
    actor.start_tool_batch(vec![ToolCall {
        id: "server-mcp-call".to_owned(),
        name: "external.weather.forecast".to_owned(),
        arguments: serde_json::json!({}),
    }]);
    server.wait_until_held().await;
    let started = std::iter::from_fn(|| rx.try_recv().ok())
        .find_map(|message| match message {
            OutboundMessage::TurnText { text, .. } => {
                let value = serde_json::from_str::<serde_json::Value>(&text).ok()?;
                (value["type"] == "tool").then_some(value)
            }
            _ => None,
        })
        .expect("external MCP start status is observable");
    assert_eq!(started["type"], "tool");
    assert_eq!(started["state"], "started");
    assert_eq!(started["name"], "external.weather.forecast");
    assert_eq!(started["turn_id"], turn_id.get());

    server.release();
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        while actor.tool_batch.is_some() {
            actor.drain_async_call_completions();
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("external MCP result completes the tool round");
    let terminal = std::iter::from_fn(|| rx.try_recv().ok())
        .find_map(|message| {
            if let OutboundMessage::TurnText { text, .. } = message {
                let value = serde_json::from_str::<serde_json::Value>(&text).ok()?;
                (value["type"] == "tool").then_some(value)
            } else {
                None
            }
        })
        .expect("external tool terminal status was sent");
    assert_eq!(terminal["state"], "completed");
    assert_eq!(terminal["name"], "external.weather.forecast");
}
