use super::*;
use crate::tools::system_mcp::{SessionSystemMcp, tests::fixture};
use std::sync::Arc;

async fn actor() -> (
    SessionActor,
    tokio::sync::mpsc::Receiver<OutboundMessage>,
    Arc<crate::database::Database>,
) {
    let (core, context, db) = fixture(true, 4096).await;
    let tools = core.list_tools(&context);
    let (tx, rx) = mpsc::channel(16);
    let mut actor = tests::session_with_external_mcp(Default::default())
        .with_system_mcp(Some(SessionSystemMcp {
            core,
            context,
            tools,
        }))
        .with_tool_status(true);
    actor.control_tx = tx;
    actor.mcp.result_delivery = crate::config::McpResultDelivery::Silent;
    actor.begin_active_turn().unwrap();
    actor.commit_user_text("check system".into()).unwrap();
    (actor, rx, db)
}

#[tokio::test]
async fn system_results_use_llm_continuation_even_when_device_delivery_is_silent() {
    let (mut actor, mut rx, db) = actor().await;
    assert!(actor.offers_answer_changing_tools());
    assert_eq!(
        actor
            .available_llm_tools()
            .iter()
            .filter(|tool| tool.name.starts_with("system_") || tool.name.starts_with("mcp_"))
            .count(),
        6
    );
    actor.start_tool_batch(vec![
        ToolCall {
            id: "first".into(),
            name: "mcp_list_servers".into(),
            arguments: serde_json::json!({}),
        },
        ToolCall {
            id: "second".into(),
            name: "mcp_get_server".into(),
            arguments: serde_json::json!({"server_key":"missing"}),
        },
    ]);
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        while actor.tool_batch.is_some() {
            actor.drain_async_call_completions();
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    let results: Vec<_> = actor
        .llm_messages
        .iter()
        .filter_map(|message| match message {
            ChatMessage::ToolResult {
                tool_call_id,
                content,
            } => Some((
                tool_call_id.as_str(),
                serde_json::from_str::<serde_json::Value>(content).unwrap(),
            )),
            _ => None,
        })
        .collect();
    assert_eq!(results.len(), 2);
    assert_eq!(results[0].0, "first");
    assert_eq!(results[0].1["ok"], true);
    let data: serde_json::Value =
        serde_json::from_str(results[0].1["content"].as_str().unwrap()).unwrap();
    assert_eq!(data["items"], serde_json::json!([]));
    assert_eq!(results[1].0, "second");
    assert_eq!(results[1].1["code"], "resource_not_found");
    let states: Vec<_> = std::iter::from_fn(|| rx.try_recv().ok())
        .filter_map(|message| match message {
            OutboundMessage::TurnText { text, .. } => {
                let frame: serde_json::Value = serde_json::from_str(&text).unwrap();
                (frame["type"] == "tool").then(|| frame["state"].as_str().unwrap().to_owned())
            }
            _ => None,
        })
        .collect();
    assert_eq!(states, ["started", "completed", "started", "failed"]);
    let caller = &actor.system_mcp.as_ref().unwrap().context.caller;
    let crate::tools::system_mcp::SystemCaller::WebAssistant { agent_id, .. } = caller;
    db.tool_security.invalidate_agent(*agent_id);
    assert!(
        actor
            .system_mcp
            .as_ref()
            .unwrap()
            .context
            .revoked
            .is_cancelled()
    );
}

#[tokio::test]
async fn abort_discards_system_completion_and_cancels_waiting_read() {
    let (mut actor, mut rx, db) = actor().await;
    let publication = db.tool_security.publication.write().await;
    actor.start_tool_batch(vec![ToolCall {
        id: "late".into(),
        name: "mcp_list_servers".into(),
        arguments: serde_json::json!({}),
    }]);
    let turn = actor.turn.as_ref().unwrap();
    let (turn_id, generation) = (turn.turn_id, turn.generation);
    let cancellation = turn.cancellation.clone();
    actor.on_client_message(crate::protocol::ClientMessage::Abort { session_id: None });
    assert!(cancellation.is_cancelled());
    assert!(actor.tool_batch.is_none());
    drop(publication);
    actor
        .async_calls_tx
        .try_send(AsyncCallCompletion {
            turn_id,
            generation,
            call_id: "late".into(),
            server_key: String::new(),
            outcome: AsyncToolOutcome::System(Ok(serde_json::json!({"late":true}))),
        })
        .unwrap_or_else(|_| panic!("bounded mailbox available"));
    actor.drain_async_call_completions();
    assert!(
        actor
            .llm_messages
            .iter()
            .all(|message| !matches!(message, ChatMessage::ToolResult { .. }))
    );
    assert!(
        std::iter::from_fn(|| rx.try_recv().ok()).all(|message| match message {
            OutboundMessage::TurnText { text, .. } => !text.contains("completed"),
            _ => true,
        })
    );
}
