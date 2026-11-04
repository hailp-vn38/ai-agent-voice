use super::*;

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
            actor.drain_async_call_completions();
            if !tool_round(&actor).is_empty() {
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("the call that was already in flight reports its outcome");
    actor.drain_async_call_completions();

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
            actor.drain_async_call_completions();
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
    actor.drain_async_call_completions();

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
