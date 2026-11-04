use super::harness::*;

/// A destination the operator did not allow is never contacted, and a redirect is never followed.
#[tokio::test]
async fn external_mcp_outbound_policy_refuses_unlisted_destinations_and_redirects() {
    // 1. A redirect from `tools/list` is not followed, so the catalog behind it never arrives.
    let redirecting = start_mcp(McpBehaviour {
        redirect_tools_list_to: Some("/elsewhere".into()),
        ..McpBehaviour::with_tools(vec![tool("Light")])
    })
    .await;
    let voice = start(ConstantSecrets(None)).await;
    let pool = voice.database().await;
    seed(&pool).await;
    bind_server(
        &pool,
        "kitchen",
        &redirecting.url(),
        "{}",
        ("none", None, None),
    )
    .await;
    assert!(
        voice.admit().await.external_mcp().is_empty(),
        "a redirect must not be a way to reach a catalog the operator never approved"
    );
    assert_eq!(voice.upgrade().await, WsStatusCode::SWITCHING_PROTOCOLS);
    voice.task.abort();
    redirecting.task.abort();

    // 2. An explicitly configured hostname allowlist rejects an unlisted destination.
    let server = start_mcp(McpBehaviour::with_tools(vec![tool("Light")])).await;
    let mut app_config = config(database_url());
    app_config.mcp.external.network = ExternalMcpNetworkConfig {
        allowed_hosts: vec!["mcp.example.test".into()],
    };
    let voice = start_with_config(app_config, ConstantSecrets(None)).await;
    let pool = voice.database().await;
    seed(&pool).await;
    bind_server(&pool, "kitchen", &server.url(), "{}", ("none", None, None)).await;
    assert!(voice.admit().await.external_mcp().is_empty());
    assert_eq!(
        server.calls(),
        0,
        "an unlisted destination is never contacted"
    );
    voice.task.abort();
    server.task.abort();
}

/// An `https` destination is never dialled without certificate and hostname validation, so a
/// plain-HTTP endpoint cannot be reached by asking for TLS: there is no insecure mode to ask for.
#[tokio::test]
async fn external_mcp_https_never_falls_back_to_an_unvalidated_connection() {
    let server = start_mcp(McpBehaviour::with_tools(vec![tool("Light")])).await;
    let voice = start(ConstantSecrets(None)).await;
    let pool = voice.database().await;
    seed(&pool).await;
    let https = format!("https://{}/mcp", server.address);
    bind_server(&pool, "kitchen", &https, "{}", ("none", None, None)).await;

    assert!(
        voice.admit().await.external_mcp().is_empty(),
        "a certificate the client cannot validate leaves the server with no tools"
    );
    assert_eq!(server.calls(), 0);
    assert_eq!(voice.upgrade().await, WsStatusCode::SWITCHING_PROTOCOLS);
    voice.task.abort();
    server.task.abort();
}

/// A disabled binding or a disabled server is not an intent this session acts on.
#[tokio::test]
async fn external_mcp_publishes_only_enabled_bindings_of_enabled_servers() {
    let server = start_mcp(McpBehaviour::with_tools(vec![tool("Light")])).await;
    let voice = start(ConstantSecrets(None)).await;
    let pool = voice.database().await;
    seed(&pool).await;
    bind_server(&pool, "enabled", &server.url(), "{}", ("none", None, None)).await;
    bind_server(&pool, "disabled", &server.url(), "{}", ("none", None, None)).await;
    bind_server(
        &pool,
        "soft-deleted",
        &server.url(),
        "{}",
        ("none", None, None),
    )
    .await;
    sqlx::query(
        "UPDATE agent_mcp_bindings SET enabled = 0 WHERE mcp_server_id = \
         (SELECT id FROM mcp_servers WHERE key = 'disabled')",
    )
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query("UPDATE mcp_servers SET enabled = 0 WHERE key = 'soft-deleted'")
        .execute(&pool)
        .await
        .unwrap();

    assert_eq!(
        names(voice.admit().await.external_mcp()),
        Vec::<String>::new()
    );
    assert_eq!(voice.upgrade().await, WsStatusCode::SWITCHING_PROTOCOLS);
    voice.task.abort();
    server.task.abort();
}

/// An Agent with no External MCP binding gets a session with no tools, and nothing else changes.
#[tokio::test]
async fn an_agent_without_mcp_bindings_admits_without_external_tools() {
    let voice = start(ConstantSecrets(None)).await;
    let pool = voice.database().await;
    seed(&pool).await;

    assert!(voice.admit().await.external_mcp().is_empty());
    assert_eq!(voice.upgrade().await, WsStatusCode::SWITCHING_PROTOCOLS);
    voice.task.abort();
}

/// Every exclusion a snapshot reports is a bounded class, and the shared transport builds once
/// with the redirect and TLS policy the guide requires.
#[tokio::test]
async fn external_mcp_diagnostics_stay_bounded_classes() {
    assert_eq!(
        ExternalMcpExclusionReason::ToolsListTimeout.to_string(),
        "mcp_tools_list_timeout"
    );
    assert_eq!(
        ExternalMcpExclusionReason::SessionToolCapExceeded.to_string(),
        "external_mcp_session_tool_cap_exceeded"
    );
    assert_eq!(
        ExternalMcpExclusionReason::SecretResolutionFailed("secret_resolver_unavailable")
            .to_string(),
        "external_mcp_secret_resolution_failed:secret_resolver_unavailable"
    );
    let manager =
        ExternalMcpManager::new(&ExternalMcpConfig::default()).expect("the transport builds");
    assert_eq!(manager.limiter().capacity(), 16);
}
