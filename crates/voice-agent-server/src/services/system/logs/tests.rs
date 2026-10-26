use super::*;
use tracing_subscriber::layer::SubscriberExt;

#[test]
fn span_correlation_and_subsystem_inheritance_from_tracing_layer() {
    let hub = Arc::new(SystemLogHub::with_capacity(64, 16));
    let subscriber = tracing_subscriber::registry().with(LogHubLayer::new(Arc::clone(&hub)));

    tracing::subscriber::with_default(subscriber, || {
        let session_span = tracing::info_span!(
            "voice_session",
            session_id = "sess-alpha-01",
            agent_key = "assistant_may",
            device_id = 42_i64,
        );
        let _session_guard = session_span.enter();

        let turn_span = tracing::info_span!(
            "voice_turn",
            turn_id = 7_u64,
            generation = 2_u64,
        );
        let _turn_guard = turn_span.enter();

        tracing::info!(
            target: "voice_agent_server::workers::tts::pool",
            event = "tts_synthesis_completed",
            outcome = "success",
            elapsed_ms = 185_u64,
            samples = 4800_u64,
            "TTS synthesis ended"
        );
    });

    let page = hub.query(&LogFilter::default()).expect("valid query");
    assert_eq!(page.items.len(), 1);
    let entry = &page.items[0];
    assert_eq!(entry.seq, 1);
    assert_eq!(entry.level, LogLevel::Info);
    assert_eq!(entry.subsystem, "voice.tts");
    assert_eq!(entry.event.as_deref(), Some("tts_synthesis_completed"));
    assert_eq!(entry.message, "TTS synthesis ended");
    assert_eq!(entry.outcome.as_deref(), Some("success"));
    assert_eq!(entry.duration_ms, Some(185));
    assert_eq!(entry.correlation.session_id.as_deref(), Some("sess-alpha-01"));
    assert_eq!(entry.correlation.agent_key.as_deref(), Some("assistant_may"));
    assert_eq!(entry.correlation.device_id, Some(42));
    assert_eq!(entry.correlation.turn_id, Some(7));
    assert_eq!(entry.correlation.generation, Some(2));
    assert_eq!(entry.fields.get("samples"), Some(&serde_json::json!(4800)));
}

#[test]
fn redacts_sensitive_keys_bearer_tokens_and_truncates_oversized_strings() {
    let hub = Arc::new(SystemLogHub::with_capacity(32, 16));
    let subscriber = tracing_subscriber::registry().with(LogHubLayer::new(Arc::clone(&hub)));
    let long_value = "a".repeat(1200);

    tracing::subscriber::with_default(subscriber, || {
        tracing::warn!(
            subsystem = "mcp.external",
            event = "external_mcp_auth_failed",
            api_key = "sk-live-secret-9999",
            bot_token = "123456:ABCDEF",
            authorization = "Bearer top-secret-admin-jwt",
            raw_header = "Bearer leaked-in-other-field",
            sealed_ref = "sealed:v1:ciphertext",
            input_tokens = 512_u64,
            output_tokens = 128_u64,
            has_saved_token = true,
            detail = %long_value,
            "Authentication failed against remote endpoint"
        );
    });

    let page = hub.query(&LogFilter::default()).expect("valid query");
    assert_eq!(page.items.len(), 1);
    let entry = &page.items[0];
    assert_eq!(
        entry.fields.get("api_key"),
        Some(&serde_json::json!("[REDACTED]"))
    );
    assert_eq!(
        entry.fields.get("bot_token"),
        Some(&serde_json::json!("[REDACTED]"))
    );
    assert_eq!(
        entry.fields.get("authorization"),
        Some(&serde_json::json!("[REDACTED]"))
    );
    assert_eq!(
        entry.fields.get("raw_header"),
        Some(&serde_json::json!("[REDACTED]"))
    );
    assert_eq!(
        entry.fields.get("sealed_ref"),
        Some(&serde_json::json!("[REDACTED]"))
    );
    // Token counters and boolean indicators are preserved intact.
    assert_eq!(entry.fields.get("input_tokens"), Some(&serde_json::json!(512)));
    assert_eq!(entry.fields.get("output_tokens"), Some(&serde_json::json!(128)));
    assert_eq!(
        entry.fields.get("has_saved_token"),
        Some(&serde_json::json!(true))
    );

    let detail = entry
        .fields
        .get("detail")
        .and_then(|v| v.as_str())
        .expect("detail string");
    assert!(detail.ends_with("…[truncated]"));
    assert!(detail.len() < 540);

    let serialized = serde_json::to_string(&page).expect("serialize page");
    assert!(!serialized.contains("sk-live-secret-9999"));
    assert!(!serialized.contains("123456:ABCDEF"));
    assert!(!serialized.contains("top-secret-admin-jwt"));
    assert!(!serialized.contains("leaked-in-other-field"));
    assert!(!serialized.contains("sealed:v1:ciphertext"));
}

#[test]
fn ring_buffer_evicts_oldest_and_supports_after_seq_and_filter_queries() {
    let hub = Arc::new(SystemLogHub::with_capacity(5, 16));
    let subscriber = tracing_subscriber::registry().with(LogHubLayer::new(Arc::clone(&hub)));

    tracing::subscriber::with_default(subscriber, || {
        for idx in 1..=8_u64 {
            if idx % 2 == 0 {
                tracing::warn!(
                    subsystem = "voice.llm",
                    event = "llm_round_warning",
                    turn_id = idx,
                    "Warning on turn {idx}"
                );
            } else {
                tracing::info!(
                    subsystem = "voice.vad",
                    event = "vad_cycle_started",
                    turn_id = idx,
                    "VAD cycle {idx}"
                );
            }
        }
    });

    let all = hub.query(&LogFilter::default()).expect("query all");
    assert_eq!(all.buffered_count, 5);
    assert_eq!(all.oldest_seq, Some(4));
    assert_eq!(all.latest_seq, 8);
    assert_eq!(all.dropped_evicted, 3);
    let seqs: Vec<u64> = all.items.iter().map(|i| i.seq).collect();
    assert_eq!(seqs, vec![4, 5, 6, 7, 8]);

    let after_6 = hub
        .query(&LogFilter {
            after_seq: Some(6),
            ..Default::default()
        })
        .expect("query after_seq");
    let after_seqs: Vec<u64> = after_6.items.iter().map(|i| i.seq).collect();
    assert_eq!(after_seqs, vec![7, 8]);

    let warn_only = hub
        .query(&LogFilter {
            min_level: Some("warn".into()),
            subsystem: Some("voice.llm".into()),
            ..Default::default()
        })
        .expect("query warn");
    let warn_seqs: Vec<u64> = warn_only.items.iter().map(|i| i.seq).collect();
    assert_eq!(warn_seqs, vec![4, 6, 8]);
}

#[test]
fn suppresses_self_observation_polling_and_log_stream_requests_unless_warn_or_error() {
    let hub = Arc::new(SystemLogHub::with_capacity(32, 16));
    let subscriber = tracing_subscriber::registry().with(LogHubLayer::new(Arc::clone(&hub)));

    tracing::subscriber::with_default(subscriber, || {
        let logs_span = tracing::info_span!(
            "http_request",
            method = "GET",
            path = "/api/admin/system/logs/stream"
        );
        logs_span.in_scope(|| {
            tracing::info!(event = "http_request_completed", status = 200_u64, "ok");
        });

        let resources_span = tracing::info_span!(
            "http_request",
            method = "GET",
            path = "/api/admin/system/resources"
        );
        resources_span.in_scope(|| {
            tracing::info!(event = "http_request_completed", status = 200_u64, "ok");
            tracing::warn!(
                event = "http_request_completed",
                status = 503_u64,
                "system metrics unavailable"
            );
        });
    });

    let page = hub.query(&LogFilter::default()).expect("query");
    assert_eq!(page.items.len(), 1);
    assert_eq!(page.items[0].level, LogLevel::Warn);
    assert_eq!(page.items[0].message, "system metrics unavailable");
}

#[tokio::test]
async fn filtered_subscription_delivers_only_matching_entries_and_reports_lag() {
    let hub = Arc::new(SystemLogHub::with_capacity(32, 4));
    let mut sub = hub
        .subscribe(&LogFilter {
            min_level: Some("warn".into()),
            subsystem: Some("voice.llm".into()),
            ..Default::default()
        })
        .expect("subscribe");

    let subscriber = tracing_subscriber::registry().with(LogHubLayer::new(Arc::clone(&hub)));
    tracing::subscriber::with_default(subscriber, || {
        tracing::info!(subsystem = "voice.llm", "ignored info");
        tracing::warn!(subsystem = "voice.tts", "ignored subsystem");
        tracing::error!(
            subsystem = "voice.llm",
            event = "llm_stream_failed",
            session_id = "sess-match",
            "matched error"
        );
    });

    let item = sub.recv().await.expect("stream item");
    match item {
        LogStreamItem::Entry(entry) => {
            assert_eq!(entry.level, LogLevel::Error);
            assert_eq!(entry.subsystem, "voice.llm");
            assert_eq!(entry.correlation.session_id.as_deref(), Some("sess-match"));
            assert_eq!(entry.message, "matched error");
        }
        other => panic!("unexpected stream item: {other:?}"),
    }
}

#[tokio::test]
async fn system_mcp_get_logs_and_audit_logs_query_boundaries() {
    use crate::{
        app::AppState,
        config::{AppConfig, DatabaseConfig},
        database::{AuditLogFilter, Database, agents::NewAgent},
        providers::ProviderSet,
        tools::system_mcp::{SystemCaller, SystemToolContext, SystemToolGrants},
    };
    use std::time::Duration;
    use tokio::time::Instant;
    use tokio_util::sync::CancellationToken;

    let mut config: AppConfig = toml::from_str(
        r#"
[server]
bind="127.0.0.1:0"
public_ws_url="ws://127.0.0.1:0/voice/v1/"
[provider_defaults]
vad="test"
asr="test"
llm="test"
tts="test"
[system_mcp]
enabled=true
[api]
enabled=true
admin_token="admin-secret-token"
"#,
    )
    .expect("parse config");
    config.database = DatabaseConfig {
        url: "sqlite::memory:".into(),
        ..Default::default()
    };

    let database = Database::connect(&config.database)
        .await
        .expect("connect sqlite");
    database
        .create_agent(
            NewAgent {
                key: "studio",
                name: "Studio",
                description: None,
            },
            "req-audit-01",
        )
        .await
        .expect("create agent");
    let device = database
        .save_assistant_device("studio", None, true, None, "req-audit-02")
        .await
        .expect("save assistant device");
    let agent_id = crate::database::agents::get_agent_by(&database, "studio")
        .await
        .expect("get agent")
        .id;

    let audit_page = database
        .list_audit_events(&AuditLogFilter {
            page: 1,
            page_size: 20,
            resource_type: Some("agent".into()),
            outcome: Some("success".into()),
        })
        .await
        .expect("list audit events");
    assert_eq!(audit_page.total, 1);
    assert_eq!(audit_page.items[0].request_id, "req-audit-01");
    assert_eq!(audit_page.items[0].action, "create");

    let hub = Arc::new(SystemLogHub::with_capacity(64, 16));
    hub.record(RawLogRecord {
        timestamp_ms: Some(1_793_000_001_000),
        level: Some(LogLevel::Info),
        target: "voice_agent_server::session::actor::listening".into(),
        subsystem: Some("voice.asr".into()),
        event: Some("asr_final_recognized".into()),
        message: "ASR final recognized".into(),
        outcome: Some("success".into()),
        duration_ms: Some(140),
        error_code: None,
        correlation: CorrelationContext {
            session_id: Some("sess-http-01".into()),
            turn_id: Some(1),
            agent_key: Some("studio".into()),
            ..Default::default()
        },
        fields: [("text_chars".to_owned(), serde_json::json!(28))]
            .into_iter()
            .collect(),
    });
    hub.record(RawLogRecord {
        timestamp_ms: Some(1_793_000_002_000),
        level: Some(LogLevel::Error),
        target: "voice_agent_server::workers::llm".into(),
        subsystem: Some("voice.llm".into()),
        event: Some("llm_stream_failed".into()),
        message: "LLM stream timed out".into(),
        outcome: Some("failure".into()),
        duration_ms: Some(15_000),
        error_code: Some("llm_timeout".into()),
        correlation: CorrelationContext {
            session_id: Some("sess-http-01".into()),
            turn_id: Some(1),
            agent_key: Some("studio".into()),
            ..Default::default()
        },
        fields: Default::default(),
    });

    let state = AppState::from_provider_set_with_database(
        config,
        Arc::new(ProviderSet::unavailable()),
        Some(database),
    )
    .with_system_logs(Arc::clone(&hub));

    let db = state.database.as_ref().unwrap();
    let mcp = crate::tools::system_mcp::SystemMcp::new(
        true,
        state.system_status(),
        state.system_metrics.clone(),
        16_384,
    )
    .with_system_logs(Arc::clone(&hub));
    let ctx = SystemToolContext {
        caller: SystemCaller::WebAssistant {
            device_db_id: device.id,
            agent_id,
        },
        grants: SystemToolGrants {
            system_read: true,
            ..Default::default()
        },
        revoked: db.tool_security.register(agent_id, Vec::new()),
        cancellation: CancellationToken::new(),
        deadline: Instant::now() + Duration::from_secs(5),
    };
    let tool_out = mcp
        .call_tool(
            &ctx,
            "system_get_logs",
            serde_json::json!({"min_level": "error", "limit": 10}),
        )
        .await
        .expect("call system_get_logs");
    assert_eq!(tool_out["items"].as_array().unwrap().len(), 1);
    assert_eq!(tool_out["items"][0]["error_code"], "llm_timeout");
}
