use super::harness::*;

/// A refusal from the remote is a fact about one call: it terminalizes that call with a typed,
/// content-free result, the round's remaining calls still run, and the turn continues.
#[tokio::test]
async fn a_refused_external_call_is_typed_and_does_not_stop_the_round() {
    let external = ExternalServer::start(ExternalScript {
        call_status: Some(401),
        ..ExternalScript::with_tools(vec![external_tool("Forecast")])
    })
    .await;
    let (llm, observed) = ScriptedLlm::new(vec![
        Round::ToolCalls(vec![
            ("call-1", EXTERNAL_LLM_NAME, serde_json::json!({})),
            ("call-2", DEVICE_LLM_NAME, serde_json::json!({})),
        ]),
        Round::Text("the lamp is on and the forecast is unavailable"),
    ]);
    let url = database_url();
    let voice = start(config(url.clone(), LlmToolsConfig::default()), llm).await;
    seed_reviewed(&voice.base, &url, &external.url).await;

    let mut peer = Peer::open(&voice.base).await;
    peer.run_turn().await;
    peer.wait_for_turn_end().await;

    assert_eq!(
        peer.device_calls.len(),
        1,
        "a refused call does not stop the sibling call behind it in the same round"
    );
    assert!(
        peer.turn_closed,
        "a refused call is not a reason to close the Voice Session"
    );
    let observed = observed
        .lock()
        .expect("the observation mailbox is not poisoned");
    let pairs = completed_pairs(&observed.requests[1]);
    assert_eq!(pair_ids(&pairs), vec!["call-1", "call-2"]);
    assert_eq!(
        pairs[0].1, r#"{"error":"external_tool_auth_failed"}"#,
        "the result names the bounded class and nothing about what the server said"
    );
    assert!(pairs[1].1.contains("device:self.lamp.on"), "{}", pairs[1].1);
    assert_eq!(
        external.script().calls(),
        vec!["Forecast"],
        "one attempt, and no retry after the refusal"
    );
}
