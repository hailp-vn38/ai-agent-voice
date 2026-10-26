use super::*;
use crate::{
    app::AppState,
    config::{AppConfig, DatabaseConfig},
    database::{Database, agents::NewAgent, mcp_servers::McpInput},
    providers::ProviderSet,
};
use std::time::Duration;

pub(crate) async fn fixture(
    enabled: bool,
    max_result_chars: usize,
) -> (Arc<SystemMcp>, SystemToolContext, Arc<Database>) {
    let config: AppConfig = toml::from_str(
        r#"
[server]
bind="127.0.0.1:0"
public_ws_url="ws://127.0.0.1:0/voice/v1/"
[provider_defaults]
vad="test"
asr="test"
llm="test"
tts="test"
"#,
    )
    .unwrap();
    let database = Database::connect(&DatabaseConfig {
        url: "sqlite::memory:".into(),
        ..Default::default()
    })
    .await
    .unwrap();
    database
        .create_agent(
            NewAgent {
                key: "studio",
                name: "Studio",
                description: None,
            },
            "test",
        )
        .await
        .unwrap();
    let device = database
        .save_assistant_device("studio", None, true, None, "test")
        .await
        .unwrap();
    let agent_id = get_agent_by(&database, "studio").await.unwrap().id;
    let state = AppState::from_provider_set_with_database(
        config,
        Arc::new(ProviderSet::unavailable()),
        Some(database),
    );
    let db = state.database.clone().unwrap();
    let context = SystemToolContext {
        caller: SystemCaller::WebAssistant {
            device_db_id: device.id,
            agent_id,
        },
        grants: SystemToolGrants {
            system_read: true,
            mcp_read: true,
            ..Default::default()
        },
        revoked: db.tool_security.register(agent_id, Vec::new()),
        cancellation: CancellationToken::new(),
        deadline: Instant::now() + Duration::from_secs(5),
    };
    (
        Arc::new(SystemMcp::new(
            enabled,
            state.system_status(),
            state.system_metrics.clone(),
            max_result_chars,
        )),
        context,
        db,
    )
}

#[tokio::test]
async fn catalog_scopes_and_disabled_ceiling_deny_guessed_calls() {
    let (core, mut ctx, _) = fixture(true, 4096).await;
    assert_eq!(core.list_tools(&ctx).len(), 7);
    ctx.grants.mcp_read = false;
    assert_eq!(core.list_tools(&ctx).len(), 3);
    assert_eq!(
        core.call_tool(&ctx, "mcp_list_servers", json!({})).await,
        Err(SystemToolError::Denied)
    );
    ctx.grants.system_read = false;
    ctx.grants.mcp_read = true;
    assert_eq!(core.list_tools(&ctx).len(), 4);
    assert_eq!(
        core.call_tool(&ctx, "system_get_status", json!({})).await,
        Err(SystemToolError::Denied)
    );
    ctx.grants.mcp_read = false;
    assert!(core.list_tools(&ctx).is_empty());
    assert_eq!(
        core.call_tool(&ctx, "system_get_status", json!({})).await,
        Err(SystemToolError::Denied)
    );
    let (core, ctx, _) = fixture(false, 4096).await;
    assert!(core.list_tools(&ctx).is_empty());
    assert_eq!(
        core.call_tool(&ctx, "mcp_get_server", json!({"server_key":"private"}))
            .await,
        Err(SystemToolError::Denied)
    );
}

#[tokio::test]
async fn read_tools_redact_sources_and_keep_approvals_independent() {
    let (core, ctx, db) = fixture(true, 8192).await;
    db.create_mcp_server(
        McpInput {
            key: "weather",
            name: "Weather",
            url: "http://internal.example/private?token=hidden",
            auth_type: "bearer",
            auth_header_name: None,
            credential: None,
            connect_timeout_ms: 1000,
            request_timeout_ms: 1000,
        },
        "test",
    )
    .await
    .unwrap();
    db.put_agent_mcp_binding("studio", "weather", true, 1, "test")
        .await
        .unwrap();
    let source = mcp_by(&db, "weather").await.unwrap();
    sqlx::query("INSERT INTO external_tool_observations(server_id,original_name,description,input_schema,fingerprint,server_revision,observed_at) VALUES(?,'forecast','hidden description','{}',?,1,unixepoch())")
        .bind(source.id).bind("a".repeat(64)).execute(db.pool()).await.unwrap();
    let listing = core
        .call_tool(&ctx, "mcp_list_servers", json!({}))
        .await
        .unwrap();
    assert_eq!(listing["items"][0]["key"], "weather");
    let detail = core
        .call_tool(&ctx, "mcp_get_server", json!({"server_key":"weather"}))
        .await
        .unwrap();
    assert_eq!(detail["transport"], "streamable_http");
    let tools = core
        .call_tool(&ctx, "mcp_list_tools", json!({"server_key":"weather"}))
        .await
        .unwrap();
    assert_eq!(tools["items"][0]["allowed"], false);
    assert_eq!(tools["items"][0]["current"], true);
    assert_eq!(tools["items"][0]["tool_name"], "forecast");
    let bindings = core
        .call_tool(
            &ctx,
            "mcp_get_agent_bindings",
            json!({"agent_key":"studio"}),
        )
        .await
        .unwrap();
    assert_eq!(bindings["items"][0]["enabled"], true);
    for value in [listing, detail, tools, bindings] {
        let text = value.to_string();
        for forbidden in [
            "internal.example",
            "hidden",
            "auth_header",
            "credential",
            "input_schema",
        ] {
            assert!(!text.contains(forbidden), "unsafe metadata: {forbidden}");
        }
    }
    assert_eq!(
        core.call_tool(&ctx, "mcp_get_server", json!({"server_key":"missing"}))
            .await,
        Err(SystemToolError::NotFound)
    );
    let summary = core
        .call_tool(&ctx, "system_get_status", json!({}))
        .await
        .unwrap();
    assert_eq!(summary["database"]["status"], "ready");
    assert!(summary.get("providers").is_some());
    assert_eq!(
        core.call_tool(&ctx, "system_get_resources", json!({}))
            .await,
        Err(SystemToolError::Unavailable)
    );
    let stop = CancellationToken::new();
    let sampler = core.metrics.clone().start(stop.clone());
    tokio::time::timeout(Duration::from_secs(5), async {
        while core.metrics.latest().is_none() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    let resources = core
        .call_tool(&ctx, "system_get_resources", json!({}))
        .await
        .unwrap();
    assert!(resources["system"].get("memory_total_bytes").is_some());
    assert!(resources["process"].get("rss_bytes").is_some());
    stop.cancel();
    sampler.await.unwrap();
}

#[tokio::test]
async fn schema_keys_and_page_bounds_are_validated_in_rust() {
    let (core, ctx, _) = fixture(true, 4096).await;
    for (name, args) in [
        ("system_get_status", json!({"approved":true})),
        ("system_get_status", json!([])),
        ("system_get_status", json!({"payload":"x".repeat(4097)})),
        ("mcp_list_servers", json!({"page":0})),
        ("mcp_list_servers", json!({"page":10001})),
        ("mcp_list_servers", json!({"page_size":101})),
        ("mcp_list_servers", json!({"page":1.5})),
        ("mcp_get_server", json!({})),
        ("mcp_get_server", json!({"server_key":"../private"})),
        ("mcp_get_server", json!({"server_key":"a".repeat(65)})),
        ("mcp_list_tools", json!({"agent_key":""})),
        ("mcp_list_tools", json!({"agent_key":null})),
        (
            "mcp_get_agent_bindings",
            json!({"agent_key":"studio","scope":"admin"}),
        ),
    ] {
        assert_eq!(
            core.call_tool(&ctx, name, args).await,
            Err(SystemToolError::InvalidArguments),
            "{name}"
        );
    }
    assert_eq!(
        core.call_tool(&ctx, "mcp_discover_tools", json!({})).await,
        Err(SystemToolError::Denied)
    );
}

#[tokio::test]
async fn cancellation_deadline_revocation_physical_identity_and_database_failure_fail_closed() {
    let (core, mut ctx, db) = fixture(true, 4096).await;
    ctx.cancellation.cancel();
    assert_eq!(
        core.call_tool(&ctx, "system_get_status", json!({})).await,
        Err(SystemToolError::Cancelled)
    );
    ctx.cancellation = CancellationToken::new();
    ctx.deadline = Instant::now() - Duration::from_secs(1);
    assert_eq!(
        core.call_tool(&ctx, "system_get_status", json!({})).await,
        Err(SystemToolError::Timeout)
    );
    ctx.deadline = Instant::now() + Duration::from_secs(5);
    sqlx::query("UPDATE devices SET kind='physical'")
        .execute(db.pool())
        .await
        .unwrap();
    assert_eq!(
        core.call_tool(&ctx, "system_get_status", json!({})).await,
        Err(SystemToolError::Denied)
    );
    sqlx::query("UPDATE devices SET kind='web_assistant',enabled=0")
        .execute(db.pool())
        .await
        .unwrap();
    assert_eq!(
        core.call_tool(&ctx, "system_get_status", json!({})).await,
        Err(SystemToolError::Denied)
    );
    sqlx::query("UPDATE devices SET enabled=1")
        .execute(db.pool())
        .await
        .unwrap();
    db.pool().close().await;
    assert_eq!(
        core.call_tool(&ctx, "system_get_status", json!({})).await,
        Err(SystemToolError::Unavailable)
    );
    ctx.revoked.cancel();
    assert!(core.list_tools(&ctx).is_empty());
    assert_eq!(
        core.call_tool(&ctx, "mcp_list_servers", json!({})).await,
        Err(SystemToolError::Denied)
    );
}

#[tokio::test]
async fn response_cap_and_shutdown_gate_apply_inside_core() {
    let (core, ctx, _) = fixture(true, 8).await;
    assert_eq!(
        core.call_tool(&ctx, "system_get_status", json!({})).await,
        Err(SystemToolError::ResultTooLarge)
    );
    core.status.lifecycle.gate().close();
    assert!(core.list_tools(&ctx).is_empty());
    assert_eq!(
        core.call_tool(&ctx, "system_get_status", json!({})).await,
        Err(SystemToolError::Denied)
    );
}
