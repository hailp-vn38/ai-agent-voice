use super::harness::*;

/// One executor runs a Device MCP call and an External MCP call in the exact order the model
/// returned them, and the continuation is handed back one paired result per call, in that order.
#[tokio::test]
async fn a_mixed_round_runs_its_calls_in_model_order_and_pairs_every_result() {
    let external =
        ExternalServer::start(ExternalScript::with_tools(vec![external_tool("Forecast")])).await;
    let (llm, observed) = ScriptedLlm::new(vec![
        Round::ToolCalls(vec![
            (
                "call-1",
                DEVICE_LLM_NAME,
                serde_json::json!({"brightness": 1}),
            ),
            (
                "call-2",
                EXTERNAL_LLM_NAME,
                serde_json::json!({"city": "Hanoi"}),
            ),
        ]),
        Round::Text("the lamp is on and it is 21 degrees"),
    ]);
    let url = database_url();
    let voice = start(config(url.clone(), LlmToolsConfig::default()), llm).await;
    seed_reviewed(&voice.base, &url, &external.url).await;

    let mut peer = Peer::open(&voice.base).await;
    peer.run_turn().await;
    peer.wait_for_turn_end().await;

    assert_eq!(
        peer.device_calls,
        vec![DeviceCall {
            name: DEVICE_TOOL.to_owned(),
            arguments: serde_json::json!({"brightness": 1}),
        }],
        "a Device MCP call is made once, with the original wire name the model never sees"
    );
    assert_eq!(
        external.script().calls(),
        vec!["Forecast"],
        "an External MCP call is made with its original wire name"
    );
    assert!(peer.turn_closed, "the turn reached its normal boundary");

    let observed = observed
        .lock()
        .expect("the observation mailbox is not poisoned");
    // Both origins are advertised in one round, which is the only reason the round was buffered.
    let offered = &observed.offered[0];
    assert!(offered.contains(&DEVICE_LLM_NAME.to_owned()), "{offered:?}");
    assert!(
        offered.contains(&EXTERNAL_LLM_NAME.to_owned()),
        "{offered:?}"
    );

    let continuation = &observed.requests[1];
    assert_eq!(
        assistant_call_ids(continuation),
        vec![vec!["call-1".to_owned(), "call-2".to_owned()]],
        "the whole round is replayed to the model in the order it asked for it"
    );
    let pairs = completed_pairs(continuation);
    assert_eq!(
        pair_ids(&pairs),
        vec!["call-1", "call-2"],
        "each result keeps its own tool_call_id and its position"
    );
    assert!(pairs[0].1.contains("device:self.lamp.on"), "{}", pairs[0].1);
    assert!(pairs[1].1.contains("external:Forecast"), "{}", pairs[1].1);
    assert!(
        pairs
            .iter()
            .all(|(_, content)| !content.contains("s3cr3t-bearer")),
        "no result carries a credential"
    );
}

/// The same executor also runs a session-local action, and the exit action's direct speech is
/// spoken only after every call before it in the model order has finished.  The exit action is the
/// session-local builtin the switch action shares its dispatch arm with.
#[tokio::test]
async fn a_session_local_action_runs_in_the_same_order_as_the_mcp_calls_around_it() {
    let external =
        ExternalServer::start(ExternalScript::with_tools(vec![external_tool("Forecast")])).await;
    let (llm, observed) = ScriptedLlm::new(vec![Round::ToolCalls(vec![
        ("call-1", EXTERNAL_LLM_NAME, serde_json::json!({})),
        (
            "call-2",
            EXIT_TOOL_NAME,
            serde_json::json!({"say_goodbye": "Tạm biệt!"}),
        ),
        ("call-3", DEVICE_LLM_NAME, serde_json::json!({})),
    ])]);
    let url = database_url();
    let voice = start(config(url.clone(), LlmToolsConfig::default()), llm).await;
    seed_reviewed(&voice.base, &url, &external.url).await;

    let mut peer = Peer::open(&voice.base).await;
    peer.run_turn().await;
    peer.wait_for_turn_end().await;

    assert_eq!(
        external.script().calls(),
        vec!["Forecast"],
        "the call in front of the action runs before the action"
    );
    assert_eq!(
        peer.device_calls,
        vec![DeviceCall {
            name: DEVICE_TOOL.to_owned(),
            arguments: serde_json::json!({}),
        }],
        "the call the model placed after the action still runs, because model order is the whole \
         order and the action only decides what the client hears"
    );
    assert!(
        peer.direct_spoken
            .iter()
            .any(|text| text.contains("Tạm biệt")),
        "the action's own answer is what the client hears: {:?}",
        peer.direct_spoken
    );
    assert!(
        peer.spoken.is_empty(),
        "tool speech must not impersonate the LLM"
    );
    assert_eq!(peer.playback, ["start", "sentence_start", "audio", "stop"]);
    let observed = observed
        .lock()
        .expect("the observation mailbox is not poisoned");
    assert_eq!(
        observed.requests.len(),
        1,
        "a direct answer ends the turn, so no continuation is built behind it"
    );
}
