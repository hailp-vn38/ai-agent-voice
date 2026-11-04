use std::time::Duration;

use super::harness::*;

/// The whole round is checked before call one, so a model that asks for more calls than the
/// deployment allows has none of them executed — not even the ones that came first in the list.
#[tokio::test]
async fn a_round_over_the_call_cap_executes_no_call_at_all() {
    let external =
        ExternalServer::start(ExternalScript::with_tools(vec![external_tool("Forecast")])).await;
    let (llm, observed) = ScriptedLlm::new(vec![Round::ToolCalls(vec![
        ("call-1", DEVICE_LLM_NAME, serde_json::json!({})),
        ("call-2", DEVICE_LLM_NAME, serde_json::json!({})),
        ("call-3", EXTERNAL_LLM_NAME, serde_json::json!({})),
    ])]);
    let url = database_url();
    let voice = start(
        config(
            url.clone(),
            LlmToolsConfig {
                max_calls_per_round: 2,
                ..Default::default()
            },
        ),
        llm,
    )
    .await;
    seed_reviewed(&voice.base, &url, &external.url).await;

    let mut peer = Peer::open(&voice.base).await;
    peer.run_turn().await;
    peer.pump(Duration::from_millis(600)).await;

    assert!(
        peer.device_calls.is_empty(),
        "the first call in the list must not run merely because it came first: {:?}",
        peer.device_calls
    );
    assert!(
        external.script().calls().is_empty(),
        "an over-cap round sends nothing to an External MCP server either"
    );
    let observed = observed
        .lock()
        .expect("the observation mailbox is not poisoned");
    assert_eq!(
        observed.requests.len(),
        1,
        "the turn ends without a continuation, so no AssistantToolCall is left dangling"
    );
    assert!(
        peer.spoken.is_empty(),
        "a refused round produces no speech: {:?}",
        peer.spoken
    );
}

/// The round allowance is checked before the next round's request is sent, so a model that keeps
/// asking for tools runs out of rounds rather than running forever.
#[tokio::test]
async fn a_turn_over_the_round_cap_stops_before_another_round_starts() {
    let external =
        ExternalServer::start(ExternalScript::with_tools(vec![external_tool("Forecast")])).await;
    let (llm, observed) = ScriptedLlm::new(vec![
        Round::ToolCalls(vec![("call-1", DEVICE_LLM_NAME, serde_json::json!({}))]),
        Round::ToolCalls(vec![("call-2", DEVICE_LLM_NAME, serde_json::json!({}))]),
        Round::ToolCalls(vec![("call-3", DEVICE_LLM_NAME, serde_json::json!({}))]),
    ]);
    let url = database_url();
    let voice = start(
        config(
            url.clone(),
            LlmToolsConfig {
                max_rounds_per_turn: 1,
                ..Default::default()
            },
        ),
        llm,
    )
    .await;
    seed_reviewed(&voice.base, &url, &external.url).await;

    let mut peer = Peer::open(&voice.base).await;
    peer.run_turn().await;
    peer.pump(Duration::from_millis(600)).await;

    let observed = observed
        .lock()
        .expect("the observation mailbox is not poisoned");
    assert_eq!(
        observed.requests.len(),
        2,
        "one tool round ran, and exactly one continuation was allowed to start"
    );
    let completed = completed_pairs(&observed.requests[1]);
    assert_eq!(
        pair_ids(&completed),
        vec!["call-1"],
        "the completed round is kept, so the turn's history still holds its paired result"
    );
}

/// The Tool Execution Budget is spent from the first call of the turn, and a call that outlives it
/// ends the turn rather than being continued.
///
/// The Device MCP origin is here for the same reason the External one is: the budget belongs to the
/// turn, not to a transport, so a call that spends it must stop the turn whichever transport spent
/// it.  The peer never answers, so what expires is the turn's time to use the request rather than
/// anything the transport did.
#[tokio::test]
async fn a_device_call_that_outlives_the_execution_budget_ends_the_turn_too() {
    let external =
        ExternalServer::start(ExternalScript::with_tools(vec![external_tool("Forecast")])).await;
    let (llm, observed) = ScriptedLlm::new(vec![
        Round::ToolCalls(vec![("call-1", DEVICE_LLM_NAME, serde_json::json!({}))]),
        Round::Text("never reached"),
    ]);
    let url = database_url();
    let voice = start(
        config(
            url.clone(),
            LlmToolsConfig {
                execution_budget_ms: 250,
                ..Default::default()
            },
        ),
        llm,
    )
    .await;
    seed_reviewed(&voice.base, &url, &external.url).await;

    let mut peer = Peer::open(&voice.base).await;
    peer.answer_device_calls = false;
    peer.run_turn().await;
    peer.pump(Duration::from_secs(2)).await;

    assert_eq!(
        peer.device_calls.len(),
        1,
        "the call started inside the budget, so it was sent"
    );
    let observed = observed
        .lock()
        .expect("the observation mailbox is not poisoned");
    assert_eq!(
        observed.requests.len(),
        1,
        "a turn that ran out of budget gets no continuation, whichever transport spent it"
    );
    assert!(
        peer.spoken.is_empty(),
        "a turn that ran out of budget speaks nothing: {:?}",
        peer.spoken
    );
}

/// The same budget, spent by the other origin.
///
/// The External origin is here because the budget is the turn's, not a transport's: a call that
/// spends it must stop the turn whichever transport spent it, and a test that only ever exhausted
/// it one way would not notice if the other way diverged.
#[tokio::test]
async fn an_external_call_that_outlives_the_execution_budget_ends_the_turn() {
    let gate = CallGate::new();
    let external = ExternalServer::start(ExternalScript {
        gate: Some(gate.clone()),
        ..ExternalScript::with_tools(vec![external_tool("Forecast")])
    })
    .await;
    let (llm, observed) = ScriptedLlm::new(vec![
        Round::ToolCalls(vec![("call-1", EXTERNAL_LLM_NAME, serde_json::json!({}))]),
        Round::Text("never reached"),
    ]);
    let url = database_url();
    let voice = start(
        config(
            url.clone(),
            LlmToolsConfig {
                execution_budget_ms: 250,
                ..Default::default()
            },
        ),
        llm,
    )
    .await;
    seed_reviewed(&voice.base, &url, &external.url).await;

    let mut peer = Peer::open(&voice.base).await;
    peer.run_turn().await;
    // The gate is opened only after the budget has had time to expire, so the call cannot be
    // answered inside it.  The request still went out, which is what makes this a call that was
    // started and then outlived its turn rather than one that was refused before it started.
    gate.wait_until_held(1).await;
    tokio::time::sleep(Duration::from_millis(600)).await;
    gate.open();
    peer.pump(Duration::from_millis(600)).await;

    assert_eq!(
        external.script().calls(),
        vec!["Forecast"],
        "the call started inside the budget, so it was sent"
    );
    let observed = observed
        .lock()
        .expect("the observation mailbox is not poisoned");
    assert_eq!(
        observed.requests.len(),
        1,
        "a turn that ran out of budget gets no continuation"
    );
    assert!(
        peer.spoken.is_empty(),
        "a turn that ran out of budget speaks nothing: {:?}",
        peer.spoken
    );
}
