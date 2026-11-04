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
        7
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
    let crate::tools::system_mcp::SystemCaller::WebAssistant { agent_id, .. } = caller else {
        panic!("expected Studio Assistant")
    };
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

#[tokio::test]
async fn reminder_builtin_tools_enforce_live_grant_and_target_scope() {
    let (_core, _context, db) = fixture(true, 4096).await;
    let agent_id = crate::database::agents::get_agent_by(&db, "studio")
        .await
        .unwrap()
        .id;
    sqlx::query(
        "INSERT INTO devices (device_id, agent_id, kind, enabled, created_at, updated_at) \
         VALUES ('test-device', ?, 'physical', 1, 1000, 1000)",
    )
    .bind(agent_id)
    .execute(db.pool())
    .await
    .unwrap();
    let notifications = crate::database::notifications::DeviceNotifications::with_clock(
        (*db).clone(),
        Arc::new(|| 1_800_000_000_000),
    );
    notifications
        .set_grant("studio", true, 1, "grant-req-1")
        .await
        .unwrap();
    let sessions =
        Arc::new(crate::services::active_device_sessions::ActiveDeviceSessions::default());
    let coordinator = crate::services::notifications::ReminderCoordinator::new(
        notifications.clone(),
        Arc::clone(&sessions),
    );
    let device_row_id = db.device_row_id("test-device").await.unwrap().unwrap();
    let handle = crate::services::notifications::SessionReminderTools {
        coordinator: coordinator.clone(),
        agent_id,
        agent_key: "studio".into(),
        default_timezone: "Asia/Ho_Chi_Minh".into(),
        target: crate::services::notifications::SessionReminderTarget::Physical {
            device_row_id,
            device_id: "test-device".into(),
        },
    };
    let mut actor = tests::session_with_external_mcp(Default::default())
        .with_reminder_tools(Some(handle))
        .with_tool_status(true);
    assert!(actor.offers_answer_changing_tools());
    assert_eq!(
        actor
            .available_llm_tools()
            .iter()
            .filter(|t| crate::tools::builtin::is_reminder_tool_name(&t.name))
            .count(),
        4
    );

    actor.begin_active_turn().unwrap();
    actor
        .commit_user_text("Nhắc tôi kiểm tra máy giặt sau 10 phút".into())
        .unwrap();
    actor.start_tool_batch(vec![
        ToolCall {
            id: "create-1".into(),
            name: "server_create_reminder".into(),
            arguments: serde_json::json!({
                "message": "Kiểm tra máy giặt",
                "delay_seconds": 600
            }),
        },
        ToolCall {
            id: "cross-device".into(),
            name: "server_create_reminder".into(),
            arguments: serde_json::json!({
                "message": "Spoof",
                "delay_seconds": 60,
                "device_id": "other-device"
            }),
        },
        ToolCall {
            id: "list-1".into(),
            name: "server_list_reminders".into(),
            arguments: serde_json::json!({}),
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
    assert_eq!(results.len(), 3);
    assert_eq!(results[0].1["ok"], true);
    let created: serde_json::Value =
        serde_json::from_str(results[0].1["content"].as_str().unwrap()).unwrap();
    assert_eq!(created["status"], "saved");
    assert_eq!(created["device_id"], "test-device");
    assert_eq!(created["due_at_ms"], 1_800_000_600_000i64);
    let reminder_id = created["reminder_id"].as_str().unwrap().to_owned();

    assert_eq!(results[1].1["ok"], false);
    assert_eq!(results[1].1["code"], "unauthorized_target");

    assert_eq!(results[2].1["ok"], true);
    let listed: serde_json::Value =
        serde_json::from_str(results[2].1["content"].as_str().unwrap()).unwrap();
    assert_eq!(listed["total"], 1);

    // Revoke grant mid-session -> subsequent tool calls must fail with reminders_disabled
    notifications
        .set_grant("studio", false, 2, "grant-req-2")
        .await
        .unwrap();
    actor.start_tool_batch(vec![ToolCall {
        id: "cancel-revoked".into(),
        name: "server_cancel_reminder".into(),
        arguments: serde_json::json!({
            "reminder_id": reminder_id,
            "expected_revision": 1
        }),
    }]);
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        while actor.tool_batch.is_some() {
            actor.drain_async_call_completions();
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    let last_result = actor
        .llm_messages
        .iter()
        .rev()
        .find_map(|message| match message {
            ChatMessage::ToolResult { content, .. } => {
                Some(serde_json::from_str::<serde_json::Value>(content).unwrap())
            }
            _ => None,
        })
        .unwrap();
    assert_eq!(last_result["ok"], false);
    assert_eq!(last_result["code"], "reminders_disabled");
}

#[tokio::test]
async fn web_assistant_reminder_tools_require_bound_physical_target() {
    let (_core, _context, db) = fixture(true, 4096).await;
    let agent_id = crate::database::agents::get_agent_by(&db, "studio")
        .await
        .unwrap()
        .id;
    sqlx::query(
        "INSERT INTO devices (device_id, agent_id, kind, enabled, created_at, updated_at) \
         VALUES ('test-device', ?, 'physical', 1, 1000, 1000)",
    )
    .bind(agent_id)
    .execute(db.pool())
    .await
    .unwrap();
    let notifications = crate::database::notifications::DeviceNotifications::with_clock(
        (*db).clone(),
        Arc::new(|| 1_800_000_000_000),
    );
    notifications
        .set_grant("studio", true, 1, "grant-req-1")
        .await
        .unwrap();
    let sessions =
        Arc::new(crate::services::active_device_sessions::ActiveDeviceSessions::default());
    let coordinator = crate::services::notifications::ReminderCoordinator::new(
        notifications,
        Arc::clone(&sessions),
    );
    let bound_target = Arc::new(std::sync::Mutex::new(None));
    let handle = crate::services::notifications::SessionReminderTools {
        coordinator,
        agent_id,
        agent_key: "studio".into(),
        default_timezone: "UTC".into(),
        target: crate::services::notifications::SessionReminderTarget::WebAssistant {
            assistant_device_id: "__system_default_device__".into(),
            bound_target: Arc::clone(&bound_target),
        },
    };
    let err = handle
        .execute(
            "web-sess-1",
            "call-1",
            "server_create_reminder",
            serde_json::json!({"message": "Test", "delay_seconds": 60}),
        )
        .await
        .unwrap_err();
    assert_eq!(err, "target_selection_required");

    let device_row_id = db.device_row_id("test-device").await.unwrap().unwrap();
    *bound_target.lock().unwrap() = Some(crate::services::notifications::BoundReminderTarget {
        device_row_id,
        device_id: "test-device".into(),
        expires_at: tokio::time::Instant::now() + std::time::Duration::from_secs(600),
    });
    let saved = handle
        .execute(
            "web-sess-1",
            "call-2",
            "server_create_reminder",
            serde_json::json!({"message": "Test bound", "delay_seconds": 120}),
        )
        .await
        .unwrap();
    assert_eq!(saved["status"], "saved");
    assert_eq!(saved["device_id"], "test-device");
}

#[tokio::test]
async fn disabled_reminder_tools_hide_legacy_task_reminders_and_reject_calls() {
    let mut actor = tests::session_with_external_mcp(Default::default());
    actor.job_submit = Some(crate::services::task_flow::VoiceTaskSubmit::unbacked_for_test());
    let advertised = actor.available_llm_tools();
    assert!(
        advertised
            .iter()
            .all(|t| !crate::tools::builtin::is_reminder_tool_name(&t.name)
                && !crate::tools::builtin::is_legacy_reminder_tool_name(&t.name)),
        "when reminders_enabled is false, neither server_*_reminder nor legacy reminder_* may be advertised"
    );

    actor.begin_active_turn().unwrap();
    actor
        .commit_user_text("Nhắc tôi uống nước sau 5 phút".into())
        .unwrap();
    actor.start_tool_batch(vec![
        ToolCall {
            id: "legacy-1".into(),
            name: "reminder_create".into(),
            arguments: serde_json::json!({"message": "Uống nước", "delay_seconds": 300}),
        },
        ToolCall {
            id: "builtin-1".into(),
            name: "server_create_reminder".into(),
            arguments: serde_json::json!({"message": "Uống nước", "delay_seconds": 300}),
        },
    ]);
    let results: Vec<_> = actor
        .llm_messages
        .iter()
        .filter_map(|message| match message {
            ChatMessage::ToolResult { content, .. } => {
                Some(serde_json::from_str::<serde_json::Value>(content).unwrap())
            }
            _ => None,
        })
        .collect();
    assert_eq!(results.len(), 2);
    assert_eq!(results[0]["ok"], false);
    assert_eq!(results[0]["code"], "reminders_disabled");
    assert_eq!(results[1]["ok"], false);
    assert_eq!(results[1]["code"], "reminders_disabled");
}
