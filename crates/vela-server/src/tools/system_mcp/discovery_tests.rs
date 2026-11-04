use super::*;
use crate::{
    config::ExternalMcpConfig,
    database::{mcp_servers::McpInput, secrets::EnvSecretResolver},
    tools::external_mcp::ExternalMcpManager,
};
use rmcp::{
    ErrorData, RoleServer, ServerHandler,
    model::*,
    service::RequestContext,
    transport::streamable_http_server::{
        StreamableHttpServerConfig, StreamableHttpService, session::local::LocalSessionManager,
    },
};
use std::{
    sync::atomic::{AtomicBool, Ordering},
    time::Duration,
};
#[derive(Clone)]
struct DiscoveryServer {
    drift: Arc<AtomicBool>,
    slow: Arc<AtomicBool>,
}
impl ServerHandler for DiscoveryServer {
    fn get_info(&self) -> ServerConfig {
        let mut info = ServerConfig::default();
        info.capabilities = ServerCapabilities::builder().enable_tools().build();
        info
    }
    async fn list_tools(
        &self,
        _: Option<PaginatedRequestParams>,
        _: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, ErrorData> {
        if self.slow.load(Ordering::Relaxed) {
            tokio::time::sleep(Duration::from_secs(10)).await;
        }
        let description = if self.drift.load(Ordering::Relaxed) {
            "changed"
        } else {
            "original"
        };
        Ok(ListToolsResult {
            tools: vec![Tool::new(
                "forecast",
                description,
                json!({"type":"object","properties":{}})
                    .as_object()
                    .unwrap()
                    .clone(),
            )],
            ..Default::default()
        })
    }
}
#[tokio::test]
async fn bounded_discovery_persists_observations_never_approves_and_revokes_drift() {
    let (base, mut ctx, db) = tests::fixture(true, 8192).await;
    ctx.grants.mcp_discover = true;
    db.save_system_mcp_grants(true, ctx.grants, 1, "grant")
        .await
        .unwrap();
    let drift = Arc::new(AtomicBool::new(false));
    let slow = Arc::new(AtomicBool::new(false));
    let service = StreamableHttpService::new(
        {
            let drift = drift.clone();
            let slow = slow.clone();
            move || {
                Ok(DiscoveryServer {
                    drift: drift.clone(),
                    slow: slow.clone(),
                })
            }
        },
        Arc::new(LocalSessionManager::default()),
        StreamableHttpServerConfig::default()
            .with_legacy_session_mode(false)
            .with_json_response(true),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let task = tokio::spawn(async move {
        axum::serve(listener, axum::Router::new().route_service("/mcp", service))
            .await
            .unwrap();
    });
    db.create_mcp_server(
        McpInput {
            key: "weather",
            name: "Weather",
            url: &format!("http://{address}/mcp"),
            auth_type: "none",
            auth_header_name: None,
            credential: None,
            connect_timeout_ms: 1000,
            request_timeout_ms: 1000,
        },
        "server",
    )
    .await
    .unwrap();
    db.put_agent_mcp_binding("studio", "weather", true, 1, "binding")
        .await
        .unwrap();
    let manager = Arc::new(
        ExternalMcpManager::new(&ExternalMcpConfig::default())
            .unwrap()
            .with_probe_limits(1, Duration::from_millis(200)),
    );
    let core = Arc::try_unwrap(base)
        .ok()
        .unwrap()
        .with_discovery(Some(manager), Arc::new(EnvSecretResolver));
    let result = core
        .call_tool(&ctx, "mcp_discover_tools", json!({"server_key":"weather"}))
        .await
        .unwrap();
    assert_eq!(result["observed_tools"], 1);
    let agent = get_agent_by(&db, "studio").await.unwrap();
    let observed = db
        .system_mcp_tools(agent.id, Some("weather"), 1)
        .await
        .unwrap();
    assert_eq!(observed[0]["allowed"], false);
    let source = mcp_by(&db, "weather").await.unwrap();
    sqlx::query("INSERT INTO agent_external_tool_allowlist(agent_id,server_id,original_name,fingerprint,allowed,sensitive,revision) VALUES(?,?,'forecast',?,1,0,1)").bind(agent.id).bind(source.id).bind(observed[0]["fingerprint"].as_str().unwrap()).execute(db.pool()).await.unwrap();
    drift.store(true, Ordering::Relaxed);
    core.call_tool(&ctx, "mcp_discover_tools", json!({"server_key":"weather"}))
        .await
        .unwrap();
    let changed = db
        .system_mcp_tools(agent.id, Some("weather"), 1)
        .await
        .unwrap();
    assert_ne!(changed[0]["fingerprint"], observed[0]["fingerprint"]);
    assert_eq!(changed[0]["allowed"], false);
    slow.store(true, Ordering::Relaxed);
    assert!(
        core.call_tool(&ctx, "mcp_discover_tools", json!({"server_key":"weather"}))
            .await
            .is_err()
    );
    assert_eq!(
        db.system_mcp_tools(agent.id, Some("weather"), 1)
            .await
            .unwrap()[0]["fingerprint"],
        changed[0]["fingerprint"]
    );
    task.abort();
}
