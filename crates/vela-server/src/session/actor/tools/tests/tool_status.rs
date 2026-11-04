use super::*;

#[test]
fn builtin_tool_status_is_visible_only_to_opted_in_clients() {
    use crate::tools::builtin::EXIT_TOOL_NAME;

    for enabled in [false, true] {
        let (tx, mut rx) = mpsc::channel(8);
        let mut actor =
            session_with_external_mcp(admitted_external_mcp()).with_tool_status(enabled);
        actor.control_tx = tx;
        let turn_id = actor.begin_active_turn().expect("admitted turn");
        actor.start_tool_batch(vec![ToolCall {
            id: "provider-secret-call-id".into(),
            name: EXIT_TOOL_NAME.into(),
            arguments: serde_json::json!({}),
        }]);
        let status = std::iter::from_fn(|| rx.try_recv().ok())
            .filter_map(|message| match message {
                OutboundMessage::TurnText {
                    generation,
                    turn_id: reported_turn_id,
                    text,
                } if generation == actor.generation && reported_turn_id == turn_id => {
                    serde_json::from_str::<serde_json::Value>(&text).ok()
                }
                _ => None,
            })
            .filter(|value| value["type"] == "tool")
            .collect::<Vec<_>>();
        if enabled {
            assert_eq!(
                status.len(),
                2,
                "a rejected builtin argument still has a terminal status"
            );
            assert_eq!(status[0]["state"], "started");
            assert_eq!(status[1]["state"], "failed");
            for frame in status {
                assert_eq!(frame["turn_id"], turn_id.get());
                assert_eq!(frame["name"], EXIT_TOOL_NAME);
                assert!(frame.get("call_id").is_none());
                assert!(frame.get("arguments").is_none());
                assert!(frame.get("reason").is_none());
            }
        } else {
            assert!(status.is_empty(), "legacy clients receive no tool status");
        }
    }
}

#[test]
fn tool_status_backpressure_does_not_prevent_tool_result_recording() {
    use crate::tools::builtin::EXIT_TOOL_NAME;
    let (tx, mut rx) = mpsc::channel(1);
    let mut actor = session_with_external_mcp(admitted_external_mcp()).with_tool_status(true);
    actor.control_tx = tx;
    actor.begin_active_turn().expect("admitted turn");
    actor.start_tool_batch(vec![ToolCall {
        id: "x".into(),
        name: EXIT_TOOL_NAME.into(),
        arguments: serde_json::json!({}),
    }]);
    let first = rx
        .try_recv()
        .expect("started entered the bounded control queue");
    let OutboundMessage::TurnText { text, .. } = first else {
        panic!("tool status must be turn-scoped")
    };
    let frame: serde_json::Value = serde_json::from_str(&text).expect("JSON");
    assert_eq!(frame["state"], "started");
    assert!(
        actor.tool_batch.is_none(),
        "the call finishes even if its terminal status was dropped"
    );
}
