use std::time::Duration;

use super::harness::*;

/// An interrupted turn starts no further call and produces no ToolResult, no continuation and no
/// speech.  The call it had in flight is dropped, so the response the server was still holding has
/// nowhere to go.
#[tokio::test]
async fn an_interrupted_turn_drops_its_in_flight_call_and_never_continues() {
    let gate = CallGate::new();
    let external = ExternalServer::start(ExternalScript {
        gate: Some(gate.clone()),
        ..ExternalScript::with_tools(vec![external_tool("Forecast")])
    })
    .await;
    let (llm, observed) = ScriptedLlm::new(vec![
        Round::ToolCalls(vec![
            ("call-1", EXTERNAL_LLM_NAME, serde_json::json!({})),
            ("call-2", DEVICE_LLM_NAME, serde_json::json!({})),
        ]),
        Round::Text("never reached"),
    ]);
    let url = database_url();
    let voice = start(config(url.clone(), LlmToolsConfig::default()), llm).await;
    seed_reviewed(&voice.base, &url, &external.url).await;

    let mut peer = Peer::open(&voice.base).await;
    peer.run_turn().await;
    // Wait for the call to actually be in flight before interrupting, so this measures a
    // cancellation of a live call rather than of a round that never started one.
    gate.wait_until_held(1).await;
    peer.abort().await;
    tokio::time::sleep(Duration::from_millis(100)).await;
    gate.open();
    peer.pump(Duration::from_millis(500)).await;

    assert!(
        peer.device_calls.is_empty(),
        "the call the model placed behind the cancelled one never starts"
    );
    assert!(
        external.script().calls().len() <= 1,
        "no retry follows the cancelled call"
    );
    let observed = observed
        .lock()
        .expect("the observation mailbox is not poisoned");
    assert_eq!(
        observed.requests.len(),
        1,
        "an interrupted turn never asks the model anything else, so no ToolResult is dangling"
    );
    assert!(
        peer.spoken.is_empty(),
        "an interrupted turn speaks nothing: {:?}",
        peer.spoken
    );
    assert!(
        !peer.turn_closed,
        "an interrupted turn never reaches a normal boundary, so a half-finished exchange is not \
         reported as a completed turn"
    );
}
