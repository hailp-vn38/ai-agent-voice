use super::*;
use crate::{
    app::admin::Patch,
    config::DatabaseConfig,
    database::{
        Database,
        credentials::CredentialCipher,
        secrets::{SecretRef, SecretResolveError, SecretResolver, SecretValue},
    },
};
use serde_json::json;
use std::sync::Arc;

struct TestSecretResolver {
    cipher: CredentialCipher,
}

impl TestSecretResolver {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            cipher: CredentialCipher::new(1, &[0x42; 32]).expect("valid 32-byte key"),
        })
    }
}

impl SecretResolver for TestSecretResolver {
    fn resolve(&self, reference: &SecretRef) -> Result<SecretValue, SecretResolveError> {
        self.cipher.resolve(reference)
    }

    fn seal(&self, value: &SecretValue, scope: &str) -> Result<String, SecretResolveError> {
        self.cipher.seal(value, scope)
    }
}

async fn setup_hub() -> (
    Arc<NotificationHub>,
    RecordingChannelAdapter,
    RecordingChannelAdapter,
    Database,
) {
    let db = Database::connect(&DatabaseConfig {
        url: "sqlite::memory:".into(),
        max_connections: 1,
        ..Default::default()
    })
    .await
    .expect("in-memory sqlite connect");
    let tg = RecordingChannelAdapter::new(ExternalChannelKind::Telegram);
    let zalo = RecordingChannelAdapter::new(ExternalChannelKind::Zalo);
    let hub = NotificationHub::new(
        db.clone(),
        TestSecretResolver::new(),
        vec![Arc::new(tg.clone()), Arc::new(zalo.clone())],
    );
    (hub, tg, zalo, db)
}

#[tokio::test]
async fn default_web_ui_inbox_deduplicates_marks_read_and_deletes() {
    let (hub, _tg, _zalo, _db) = setup_hub().await;
    let mut sse_rx = hub.subscribe_web();

    let input = NotificationInput {
        dedup_key: Some("reminder_due:sched-1:1000".into()),
        content: NotificationContent {
            topic: NotificationTopic::ReminderDue,
            severity: NotificationSeverity::Warning,
            title: "Nhắc nhở: Họp sprint".into(),
            body: "Bắt đầu họp lúc 09:00.".into(),
            source_kind: Some(NotificationSourceKind::Schedule),
            source_id: Some("sched-1".into()),
        },
        metadata: json!({"resource_href": "/jobs"}),
    };

    let first = hub.publish(input.clone()).await.expect("first publish");
    let second = hub.publish(input).await.expect("dedup publish");
    assert_eq!(first.id, second.id);

    // Exactly one SSE NotificationCreated event should be emitted.
    let ev = sse_rx.try_recv().expect("first SSE event");
    match ev {
        NotificationSseEvent::NotificationCreated(created) => assert_eq!(created.id, first.id),
        other => panic!("unexpected SSE event: {other:?}"),
    }
    assert!(sse_rx.try_recv().is_err());

    let page = hub
        .list_inbox(&InboxListQuery::default())
        .await
        .expect("list inbox");
    assert_eq!(page.total, 1);
    assert_eq!(page.unread_count, 1);
    assert!(!page.items[0].is_read);

    // Mark read
    let read_res = hub
        .mark_read(std::slice::from_ref(&first.id), false)
        .await
        .expect("mark read");
    assert_eq!(read_res.updated, 1);
    assert_eq!(read_res.unread_count, 0);

    // Purge read
    let del_res = hub.delete_inbox(None, true).await.expect("purge read");
    assert_eq!(del_res.deleted, 1);
    assert_eq!(del_res.unread_count, 0);
}

#[tokio::test]
async fn topic_routing_only_enqueues_subscribed_enabled_channels() {
    let (hub, tg, zalo, _db) = setup_hub().await;

    // Enable Telegram only for ReminderDue; keep Zalo disabled.
    let updated = hub
        .update_settings(
            1,
            NotificationSettingsUpdate {
                web_ui: WebUiChannelSettingsUpdate {
                    toast_enabled: true,
                    retention_days: 30,
                },
                telegram: TelegramChannelSettingsUpdate {
                    enabled: true,
                    chat_id: "-100998877".into(),
                    thread_id: Some(7),
                    subscribed_topics: vec![NotificationTopic::ReminderDue],
                    bot_token: Patch::Clear(Some(SecretValue::new("123456:ABCDEF_TOKEN".into()))),
                },
                zalo: ZaloChannelSettingsUpdate {
                    enabled: false,
                    recipient_id: String::new(),
                    subscribed_topics: vec![NotificationTopic::ReminderDue],
                    bot_token: Patch::Absent,
                },
            },
            "req-routing-1",
        )
        .await
        .expect("update settings");
    assert_eq!(updated.revision, 2);
    assert!(updated.telegram.has_saved_token);

    // 1. Publish TaskCompleted -> should go to Web UI Inbox ONLY, no outbox rows.
    let completed = hub
        .publish(NotificationInput {
            dedup_key: Some("task_completed:job-1".into()),
            content: NotificationContent {
                topic: NotificationTopic::TaskCompleted,
                severity: NotificationSeverity::Success,
                title: "Hoàn tất tác vụ".into(),
                body: "Đã tổng hợp báo cáo.".into(),
                source_kind: Some(NotificationSourceKind::Job),
                source_id: Some("job-1".into()),
            },
            metadata: json!({}),
        })
        .await
        .expect("publish task_completed");
    assert!(completed.deliveries.is_empty());
    assert_eq!(hub.process_due_outbox().await.expect("process outbox"), 0);
    assert!(tg.recorded().is_empty());
    assert!(zalo.recorded().is_empty());

    // 2. Publish ReminderDue -> should go to Web UI Inbox AND Telegram outbox.
    let reminder = hub
        .publish(NotificationInput {
            dedup_key: Some("reminder_due:rem-1".into()),
            content: NotificationContent {
                topic: NotificationTopic::ReminderDue,
                severity: NotificationSeverity::Warning,
                title: "Nhắc nhở kiểm tra thiết bị".into(),
                body: "Kiểm tra phòng họp B.".into(),
                source_kind: Some(NotificationSourceKind::Schedule),
                source_id: Some("rem-1".into()),
            },
            metadata: json!({}),
        })
        .await
        .expect("publish reminder_due");
    assert_eq!(reminder.deliveries.len(), 1);
    assert_eq!(
        reminder.deliveries[0].channel,
        ExternalChannelKind::Telegram
    );
    assert_eq!(reminder.deliveries[0].status, OutboxDeliveryStatus::Pending);

    let processed = hub.process_due_outbox().await.expect("process outbox");
    assert_eq!(processed, 1);

    let tg_sent = tg.recorded();
    assert_eq!(tg_sent.len(), 1);
    assert_eq!(tg_sent[0].recipient_id, "-100998877");
    assert_eq!(tg_sent[0].thread_id, Some(7));
    assert!(tg_sent[0].text.contains("Nhắc nhở kiểm tra thiết bị"));
    assert!(zalo.recorded().is_empty());
}

#[tokio::test]
async fn fault_isolation_retries_transient_errors_and_stops_on_unauthorized() {
    let (hub, tg, zalo, db) = setup_hub().await;

    hub.update_settings(
        1,
        NotificationSettingsUpdate {
            web_ui: WebUiChannelSettingsUpdate {
                toast_enabled: true,
                retention_days: 30,
            },
            telegram: TelegramChannelSettingsUpdate {
                enabled: true,
                chat_id: "-100111".into(),
                thread_id: None,
                subscribed_topics: vec![NotificationTopic::TaskFailed],
                bot_token: Patch::Clear(Some(SecretValue::new("tg_secret_token_1234".into()))),
            },
            zalo: ZaloChannelSettingsUpdate {
                enabled: true,
                recipient_id: "84900112233".into(),
                subscribed_topics: vec![NotificationTopic::TaskFailed],
                bot_token: Patch::Clear(Some(SecretValue::new("zalo_secret_token_5678".into()))),
            },
        },
        "req-fault-1",
    )
    .await
    .expect("enable both channels");

    // Telegram: upstream timeout on attempt 1, then success on attempt 2.
    tg.push_outcome(Err(ChannelDeliveryError::UpstreamTimeout));
    tg.push_outcome(Ok(ExternalDeliveryReceipt {
        external_message_id: Some("tg-msg-99".into()),
    }));

    // Zalo: permanent Unauthorized error on attempt 1 -> must mark Failed immediately.
    zalo.push_outcome(Err(ChannelDeliveryError::Unauthorized));

    let item = hub
        .publish(NotificationInput {
            dedup_key: Some("task_failed:job-err-1".into()),
            content: NotificationContent {
                topic: NotificationTopic::TaskFailed,
                severity: NotificationSeverity::Error,
                title: "Tác vụ lỗi".into(),
                body: "Timeout kết nối.".into(),
                source_kind: Some(NotificationSourceKind::Job),
                source_id: Some("job-err-1".into()),
            },
            metadata: json!({}),
        })
        .await
        .expect("publish task_failed");

    // First outbox pass:
    hub.process_due_outbox().await.expect("first pass");

    let page_after_first = hub
        .list_inbox(&InboxListQuery::default())
        .await
        .expect("list after first pass");
    let first_view = page_after_first
        .items
        .iter()
        .find(|i| i.id == item.id)
        .expect("item in inbox");

    let tg_status_1 = first_view
        .deliveries
        .iter()
        .find(|d| d.channel == ExternalChannelKind::Telegram)
        .expect("tg delivery");
    assert_eq!(tg_status_1.status, OutboxDeliveryStatus::Pending);
    assert_eq!(tg_status_1.attempts, 1);
    assert_eq!(
        tg_status_1.last_error_code.as_deref(),
        Some("upstream_timeout")
    );

    let zalo_status_1 = first_view
        .deliveries
        .iter()
        .find(|d| d.channel == ExternalChannelKind::Zalo)
        .expect("zalo delivery");
    assert_eq!(zalo_status_1.status, OutboxDeliveryStatus::Failed);
    assert_eq!(zalo_status_1.attempts, 1);
    assert_eq!(
        zalo_status_1.last_error_code.as_deref(),
        Some("unauthorized")
    );

    // Fast-forward next_attempt_at_ms for Telegram in DB to simulate backoff elapsed.
    sqlx::query(
        "UPDATE notification_outbox SET next_attempt_at_ms = 0 WHERE channel_kind = 'telegram'",
    )
    .execute(db.pool())
    .await
    .expect("fast-forward retry timer");

    // Second outbox pass: only Telegram should be retried and succeed!
    let processed_2 = hub.process_due_outbox().await.expect("second pass");
    assert_eq!(processed_2, 1);

    let page_after_second = hub
        .list_inbox(&InboxListQuery::default())
        .await
        .expect("list after second pass");
    let second_view = page_after_second
        .items
        .iter()
        .find(|i| i.id == item.id)
        .expect("item in inbox");
    let tg_status_2 = second_view
        .deliveries
        .iter()
        .find(|d| d.channel == ExternalChannelKind::Telegram)
        .expect("tg delivery");
    assert_eq!(tg_status_2.status, OutboxDeliveryStatus::Sent);
    assert_eq!(tg_status_2.attempts, 2);
    assert!(tg_status_2.last_error_code.is_none());
}

#[tokio::test]
async fn settings_patch_secret_redaction_if_match_conflict_and_audit() {
    let (hub, _tg, _zalo, db) = setup_hub().await;

    // 1. Save secret token (revision 1 -> 2)
    let saved = hub
        .update_settings(
            1,
            NotificationSettingsUpdate {
                web_ui: WebUiChannelSettingsUpdate {
                    toast_enabled: false,
                    retention_days: 45,
                },
                telegram: TelegramChannelSettingsUpdate {
                    enabled: true,
                    chat_id: "-100555".into(),
                    thread_id: None,
                    subscribed_topics: vec![NotificationTopic::SystemChangeRequest],
                    bot_token: Patch::Clear(Some(SecretValue::new("SUPER_SECRET_BOT_9876".into()))),
                },
                zalo: ZaloChannelSettingsUpdate {
                    enabled: false,
                    recipient_id: String::new(),
                    subscribed_topics: vec![],
                    bot_token: Patch::Absent,
                },
            },
            "req-audit-save",
        )
        .await
        .expect("save settings");

    assert_eq!(saved.revision, 2);
    assert!(saved.telegram.has_saved_token);
    let serialized = serde_json::to_string(&saved).expect("serialize settings");
    assert!(!serialized.contains("SUPER_SECRET_BOT_9876"));
    assert!(!serialized.contains("sealed:"));
    assert_eq!(
        saved.telegram.credential_metadata["masked_key"].as_str(),
        Some("…9876")
    );

    // 2. Omit bot_token (Patch::Absent) -> retains saved token (revision 2 -> 3)
    let retained = hub
        .update_settings(
            2,
            NotificationSettingsUpdate {
                web_ui: WebUiChannelSettingsUpdate {
                    toast_enabled: true,
                    retention_days: 60,
                },
                telegram: TelegramChannelSettingsUpdate {
                    enabled: true,
                    chat_id: "-100555".into(),
                    thread_id: Some(12),
                    subscribed_topics: vec![NotificationTopic::SystemChangeRequest],
                    bot_token: Patch::Absent,
                },
                zalo: ZaloChannelSettingsUpdate {
                    enabled: false,
                    recipient_id: String::new(),
                    subscribed_topics: vec![],
                    bot_token: Patch::Absent,
                },
            },
            "req-audit-retain",
        )
        .await
        .expect("retain token");
    assert_eq!(retained.revision, 3);
    assert!(retained.telegram.has_saved_token);
    assert_eq!(
        retained.telegram.credential_metadata["masked_key"].as_str(),
        Some("…9876")
    );

    // 3. Clear bot_token (Patch::Clear(None)) while disabling channel (revision 3 -> 4)
    let cleared = hub
        .update_settings(
            3,
            NotificationSettingsUpdate {
                web_ui: WebUiChannelSettingsUpdate {
                    toast_enabled: true,
                    retention_days: 60,
                },
                telegram: TelegramChannelSettingsUpdate {
                    enabled: false,
                    chat_id: "-100555".into(),
                    thread_id: None,
                    subscribed_topics: vec![NotificationTopic::SystemChangeRequest],
                    bot_token: Patch::Clear(None),
                },
                zalo: ZaloChannelSettingsUpdate {
                    enabled: false,
                    recipient_id: String::new(),
                    subscribed_topics: vec![],
                    bot_token: Patch::Absent,
                },
            },
            "req-audit-clear",
        )
        .await
        .expect("clear token");
    assert_eq!(cleared.revision, 4);
    assert!(!cleared.telegram.has_saved_token);
    assert!(cleared.telegram.credential_metadata.is_null());

    // 4. Stale If-Match revision -> RevisionConflict + conflict audit event
    let conflict = hub
        .update_settings(
            1,
            NotificationSettingsUpdate {
                web_ui: WebUiChannelSettingsUpdate {
                    toast_enabled: true,
                    retention_days: 30,
                },
                telegram: TelegramChannelSettingsUpdate {
                    enabled: false,
                    chat_id: String::new(),
                    thread_id: None,
                    subscribed_topics: vec![],
                    bot_token: Patch::Absent,
                },
                zalo: ZaloChannelSettingsUpdate {
                    enabled: false,
                    recipient_id: String::new(),
                    subscribed_topics: vec![],
                    bot_token: Patch::Absent,
                },
            },
            "req-audit-conflict",
        )
        .await
        .expect_err("stale revision must fail");
    assert!(matches!(conflict, NotificationHubError::RevisionConflict));

    let audit_rows: Vec<(String, String)> = sqlx::query_as(
        "SELECT request_id, outcome FROM admin_audit_events \
         WHERE resource_type = 'notification_settings' ORDER BY id ASC",
    )
    .fetch_all(db.pool())
    .await
    .expect("query audit events");
    assert_eq!(
        audit_rows,
        vec![
            ("req-audit-save".into(), "success".into()),
            ("req-audit-retain".into(), "success".into()),
            ("req-audit-clear".into(), "success".into()),
            ("req-audit-conflict".into(), "conflict".into()),
        ]
    );
}

#[tokio::test]
async fn admin_http_endpoints_and_sse_stream_work_end_to_end() {
    let (hub, tg, zalo, db) = setup_hub().await;

    let config: crate::config::AppConfig = toml::from_str(
        r#"
[server]
bind = "127.0.0.1:0"
public_ws_url = "ws://127.0.0.1:0/voice/v1/"
[provider_defaults]
vad = "test"
asr = "test"
llm = "test"
tts = "test"
[api]
enabled = true
admin_token = "notif-admin-secret"
"#,
    )
    .expect("valid test config");

    let state = crate::app::AppState::from_provider_set_with_database_resolver_and_shutdown(
        config,
        Arc::new(crate::providers::ProviderSet::unavailable()),
        Some(db),
        TestSecretResolver::new(),
        crate::lifecycle::RuntimeLifecycle::new(std::time::Duration::from_millis(500)),
    )
    .with_notification_hub(Arc::clone(&hub));

    let router = crate::app::router_with_state(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind listener");
    let base = format!("http://{}/api/admin", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        let _ = axum::serve(listener, router).await;
    });

    let client = reqwest::Client::new();

    // 1. Unauthenticated request returns 401
    let unauth = client
        .get(format!("{base}/notifications"))
        .send()
        .await
        .unwrap();
    assert_eq!(unauth.status(), reqwest::StatusCode::UNAUTHORIZED);

    // 2. GET /notifications/settings returns initial revision 1
    let settings_resp: serde_json::Value = client
        .get(format!("{base}/notifications/settings"))
        .bearer_auth("notif-admin-secret")
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(settings_resp["revision"], 1);
    assert_eq!(settings_resp["web_ui"]["enabled"], true);

    // 3. PUT /notifications/settings with If-Match: "1" enables Telegram & Zalo
    let put_resp = client
        .put(format!("{base}/notifications/settings"))
        .bearer_auth("notif-admin-secret")
        .header("If-Match", "\"1\"")
        .json(&json!({
            "web_ui": {
                "toast_enabled": true,
                "retention_days": 30
            },
            "telegram": {
                "enabled": true,
                "chat_id": "-1004242",
                "thread_id": 9,
                "subscribed_topics": ["reminder_due", "system_change_request"],
                "bot_token": "tg_bot_token_secret_4242"
            },
            "zalo": {
                "enabled": true,
                "recipient_id": "84912345678",
                "subscribed_topics": ["system_change_request"],
                "bot_token": "zalo_bot_token_secret_8888"
            }
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(put_resp.status(), reqwest::StatusCode::OK);
    let updated_json: serde_json::Value = put_resp.json().await.unwrap();
    assert_eq!(updated_json["revision"], 2);
    assert_eq!(updated_json["telegram"]["has_saved_token"], true);
    assert_eq!(
        updated_json["telegram"]["credential_metadata"]["masked_key"],
        "…4242"
    );
    assert_eq!(updated_json["zalo"]["has_saved_token"], true);
    assert_eq!(
        updated_json["zalo"]["credential_metadata"]["masked_key"],
        "…8888"
    );

    // 4. POST /notifications/channels/telegram/test & zalo/test
    let test_tg: serde_json::Value = client
        .post(format!("{base}/notifications/channels/telegram/test"))
        .bearer_auth("notif-admin-secret")
        .json(&json!({"message": "Ping Telegram"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(test_tg["ok"], true);
    assert_eq!(tg.recorded().len(), 1);

    let test_zalo: serde_json::Value = client
        .post(format!("{base}/notifications/channels/zalo/test"))
        .bearer_auth("notif-admin-secret")
        .json(&json!({"message": "Ping Zalo"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(test_zalo["ok"], true);
    assert_eq!(zalo.recorded().len(), 1);

    // 5. Publish via bridge::from_system_change_request and verify Inbox + MarkRead + Delete
    let published = hub
        .publish(bridge::from_system_change_request(
            "req-mcp-1",
            "mcp_request_server_change",
            "Đề xuất bật MCP server crm.",
        ))
        .await
        .unwrap();

    let inbox_resp: serde_json::Value = client
        .get(format!("{base}/notifications?unread_only=true"))
        .bearer_auth("notif-admin-secret")
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(inbox_resp["unread_count"], 1);
    assert_eq!(inbox_resp["items"][0]["id"], published.id);

    let read_resp: serde_json::Value = client
        .post(format!("{base}/notifications/read"))
        .bearer_auth("notif-admin-secret")
        .json(&json!({"ids": [published.id], "all": false}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(read_resp["updated"], 1);
    assert_eq!(read_resp["unread_count"], 0);

    let del_resp: serde_json::Value = client
        .delete(format!("{base}/notifications/{}", published.id))
        .bearer_auth("notif-admin-secret")
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(del_resp["deleted"], 1);

    server.abort();
}
