use std::sync::Arc;

use super::harness::*;

/// One admission produces a fresh, bounded, fail-soft snapshot, and a Voice Session crosses the
/// boundary on it.
#[tokio::test]
async fn external_mcp_admission_snapshot_is_fresh_bounded_and_immutable() {
    let server = start_mcp(McpBehaviour::with_tools(vec![
        tool("Light/Turn-On"),
        tool("Dim"),
    ]))
    .await;
    let voice = start(ConstantSecrets(Some("s3cr3t-bearer".into()))).await;
    let pool = voice.database().await;
    seed(&pool).await;
    bind_server(
        &pool,
        "home_assistant",
        &server.url(),
        r#"{"X-Tenant": "kitchen"}"#,
        ("bearer", None, Some("WEATHER_TOKEN")),
    )
    .await;
    // A second bound server is unreachable.  It must cost only itself.
    bind_server(
        &pool,
        "dead",
        "http://127.0.0.1:1/mcp",
        "{}",
        ("none", None, None),
    )
    .await;

    let profile = voice.admit().await;
    assert_eq!(
        names(profile.external_mcp()),
        Vec::<String>::new(),
        "discovery records the unapproved contract but never publishes it"
    );
    assert_eq!(voice.upgrade().await, WsStatusCode::SWITCHING_PROTOCOLS);

    // Both the validated static header and the typed credential travelled, and the typed auth is
    // the only thing that ever puts a credential on the request.
    let seen = server.seen();
    assert!(
        seen.iter().all(|(authorization, tenant)| {
            authorization == &Some("Bearer s3cr3t-bearer".to_owned())
                && tenant == &Some("kitchen".to_owned())
        }),
        "every request carries the validated static header and the typed credential: {seen:?}"
    );
    assert!(!seen.is_empty());

    // Fresh, not cached: the remote catalog changing changes the next admission's snapshot, so a
    // previous session's verification is never authority for this one.
    server.rewrite(|script| script.tools = vec![tool("Only")]);
    assert_eq!(
        names(voice.admit().await.external_mcp()),
        Vec::<String>::new()
    );
    // And the earlier session's own catalog did not change under it.
    assert_eq!(
        profile.external_mcp().tool_count(),
        0,
        "a later admission never mutates an open session's snapshot"
    );

    voice.task.abort();
    server.task.abort();
}

/// The modern stateful variant is an SSE lifecycle, not a stateless JSON fixture wearing an SSE
/// content type: RMCP must retain the identity minted at initialize for its notification,
/// discovery and later tool invocation.
/// A credential that does not resolve excludes its server and discloses nothing.
#[tokio::test]
async fn external_mcp_secret_failure_excludes_its_server_and_redacts_its_reason() {
    let server = start_mcp(McpBehaviour::with_tools(vec![tool("Light")])).await;
    // The resolver knows nothing, so the reference cannot be resolved.
    let voice = start(ConstantSecrets(None)).await;
    let pool = voice.database().await;
    seed(&pool).await;
    bind_server(
        &pool,
        "guarded",
        &server.url(),
        "{}",
        ("bearer", None, Some("WEATHER_TOKEN")),
    )
    .await;

    let profile = voice.admit().await;
    assert!(
        profile.external_mcp().is_empty(),
        "a server whose credential did not resolve publishes no tools"
    );
    assert_eq!(
        server.calls(),
        0,
        "a credential that never resolved must not reach the network"
    );
    assert_eq!(voice.upgrade().await, WsStatusCode::SWITCHING_PROTOCOLS);
    // Neither the reference nor any resolved value may appear in anything a test can render.
    let rendered = format!(
        "{:?} {:?}",
        profile.external_mcp(),
        ExternalMcpExclusionReason::SecretResolutionFailed("secret_resolver_unavailable")
    );
    assert!(
        !rendered.contains("WEATHER_TOKEN"),
        "a diagnostic must never carry a secret reference: {rendered}"
    );
    assert!(rendered.contains("secret_resolver_unavailable"));

    voice.task.abort();
    server.task.abort();
}

type Case = (
    &'static str,
    ExternalMcpExclusionReason,
    Vec<serde_json::Value>,
    usize,
    ExternalMcpLimitsConfig,
);

/// Every catalog cap and namespace rule refuses a server rather than shrinking one.
#[tokio::test]
async fn external_mcp_discovery_refuses_caps_namespaces_and_collisions() {
    let rejected = ExternalMcpExclusionReason::CatalogRejected;
    let cases: Vec<Case> = vec![
        (
            "a schema outside the supported subset",
            rejected,
            vec![recursive_tool()],
            0,
            ExternalMcpLimitsConfig::default(),
        ),
        (
            "a composition the conversion cannot honor",
            rejected,
            vec![serde_json::json!({
                "name": "Light",
                "description": "composed",
                "inputSchema": {"type": "object", "oneOf": [{"type": "object"}]}
            })],
            0,
            ExternalMcpLimitsConfig::default(),
        ),
        (
            "a required property the schema does not declare",
            rejected,
            vec![serde_json::json!({
                "name": "Light",
                "description": "unsatisfiable",
                "inputSchema": {"type": "object", "required": ["absent"]}
            })],
            0,
            ExternalMcpLimitsConfig::default(),
        ),
        (
            "a schema larger than the configured cap",
            rejected,
            vec![serde_json::json!({
                "name": "Light",
                "description": "oversized",
                "inputSchema": {
                    "type": "object",
                    "description": "x".repeat(128),
                    "properties": {}
                }
            })],
            0,
            ExternalMcpLimitsConfig {
                max_tool_schema_bytes: 16,
                ..ExternalMcpLimitsConfig::default()
            },
        ),
        (
            "a description larger than the configured cap",
            rejected,
            vec![serde_json::json!({
                "name": "Light",
                "description": "x".repeat(64),
                "inputSchema": {"type": "object", "properties": {}}
            })],
            0,
            ExternalMcpLimitsConfig {
                max_tool_description_bytes: 16,
                ..ExternalMcpLimitsConfig::default()
            },
        ),
        (
            "more tools than one server may publish",
            rejected,
            (0..5).map(|index| tool(&format!("Tool{index}"))).collect(),
            0,
            ExternalMcpLimitsConfig {
                max_tools_per_server: 2,
                ..ExternalMcpLimitsConfig::default()
            },
        ),
        (
            "more pages than one server may walk",
            rejected,
            (0..4).map(|index| tool(&format!("Tool{index}"))).collect(),
            1,
            ExternalMcpLimitsConfig {
                max_pages_per_server: 2,
                ..ExternalMcpLimitsConfig::default()
            },
        ),
        (
            "two tool names that normalize alike",
            ExternalMcpExclusionReason::ToolNameCollision,
            vec![tool("foo-bar"), tool("foo_bar")],
            0,
            ExternalMcpLimitsConfig::default(),
        ),
    ];

    for (label, expected, tools, page_size, limits) in cases {
        let server = start_mcp(McpBehaviour {
            page_size,
            ..McpBehaviour::with_tools(tools)
        })
        .await;
        let mut app_config = config(database_url());
        app_config.mcp.external.limits = limits;
        let voice = start_with_config(app_config, ConstantSecrets(None)).await;
        let pool = voice.database().await;
        seed(&pool).await;
        bind_server(&pool, "kitchen", &server.url(), "{}", ("none", None, None)).await;

        assert!(
            voice.admit().await.external_mcp().is_empty(),
            "{label} must leave the session without that server's tools"
        );
        let snapshot = voice
            .state
            .external_mcp
            .as_ref()
            .expect("the shared transport is available")
            .resolve_snapshot(
                &Database::connect(&voice.state.config.database)
                    .await
                    .expect("the control plane is reachable")
                    .agent_mcp_servers(1)
                    .await
                    .expect("the bindings are readable"),
                &ConstantSecrets(None),
            )
            .await;
        assert_eq!(
            snapshot
                .exclusions
                .iter()
                .map(|exclusion| (exclusion.server_key.as_str(), exclusion.reason))
                .collect::<Vec<_>>(),
            vec![("kitchen", expected)],
            "{label} must be reported as one bounded class"
        );
        assert_eq!(
            voice.upgrade().await,
            WsStatusCode::SWITCHING_PROTOCOLS,
            "{label} must never refuse a Voice Session"
        );
        voice.task.abort();
        server.task.abort();
    }
}

/// Two server keys that normalize alike make each other's tools indistinguishable, so neither is
/// published — and the answer does not depend on bind order.
#[tokio::test]
async fn external_mcp_refuses_server_keys_that_normalize_alike() {
    let server = start_mcp(McpBehaviour::with_tools(vec![tool("Light")])).await;
    let voice = start(ConstantSecrets(None)).await;
    let pool = voice.database().await;
    seed(&pool).await;
    bind_server(&pool, "foo-bar", &server.url(), "{}", ("none", None, None)).await;
    bind_server(&pool, "foo_bar", &server.url(), "{}", ("none", None, None)).await;
    assert!(voice.admit().await.external_mcp().is_empty());
    assert_eq!(voice.upgrade().await, WsStatusCode::SWITCHING_PROTOCOLS);
    voice.task.abort();
    server.task.abort();
}

/// The aggregate cap gets its own counter and no per-server duration, and every server that did
/// resolve still counts as resolved.  Counting it a second time as a server failure would report
/// an outcome and a duration for work that never happened.
#[tokio::test]
async fn external_mcp_aggregate_tool_cap_is_counted_once_and_not_as_a_server_failure() {
    let first = start_mcp(McpBehaviour::with_tools(vec![tool("A0"), tool("A1")])).await;
    let second = start_mcp(McpBehaviour::with_tools(vec![tool("B0"), tool("B1")])).await;
    let recorder = Arc::new(RecordingTelemetry::default());
    let mut app_config = config(database_url());
    app_config.mcp.external.limits = ExternalMcpLimitsConfig {
        max_tools_per_session: 3,
        ..ExternalMcpLimitsConfig::default()
    };
    let voice = start_with_config(app_config, ConstantSecrets(None)).await;
    let manager =
        ExternalMcpManager::new_with_telemetry(&voice.state.config.mcp.external, recorder.clone())
            .expect("the transport builds");
    let pool = voice.database().await;
    seed(&pool).await;
    bind_server(&pool, "alpha", &first.url(), "{}", ("none", None, None)).await;
    bind_server(&pool, "beta", &second.url(), "{}", ("none", None, None)).await;

    let snapshot = manager
        .resolve_snapshot(
            &Database::connect(&voice.state.config.database)
                .await
                .expect("the control plane is reachable")
                .agent_mcp_servers(1)
                .await
                .expect("the bindings are readable"),
            &ConstantSecrets(None),
        )
        .await;
    assert!(snapshot.servers.is_empty());
    assert_eq!(snapshot.exclusions.len(), 2);

    let recorded = recorder.recorded();
    let counted: Vec<(&str, Vec<(&str, String)>)> = recorded
        .iter()
        .map(|event| (event.metric, event.labels.clone()))
        .collect();

    // Counted once, with no label: it is a property of the snapshot, not of a server.
    let cap: Vec<_> = recorded
        .iter()
        .filter(|event| event.metric == EXTERNAL_MCP_SESSION_TOOL_CAP_EXCEEDED_TOTAL)
        .collect();
    assert_eq!(
        cap.len(),
        1,
        "the aggregate cap is counted once: {counted:?}"
    );
    assert_eq!(cap[0].labels, Vec::<(&str, String)>::new());

    // Both servers still counted as resolved, and neither as a failure.
    let successes: Vec<&Vec<(&str, String)>> = recorded
        .iter()
        .filter(|event| event.metric == MCP_RESOLVE_SUCCESS_TOTAL)
        .map(|event| &event.labels)
        .collect();
    assert_eq!(successes.len(), 2, "each resolved server still counts once");
    assert!(
        recorded
            .iter()
            .all(|event| event.metric != MCP_RESOLVE_FAILURE_TOTAL)
    );

    voice.task.abort();
    first.task.abort();
    second.task.abort();
}

/// The aggregate cap drops the whole External MCP snapshot rather than any subset of it.
#[tokio::test]
async fn external_mcp_aggregate_tool_cap_drops_the_whole_snapshot() {
    let first = start_mcp(McpBehaviour::with_tools(vec![tool("A0"), tool("A1")])).await;
    let second = start_mcp(McpBehaviour::with_tools(vec![tool("B0"), tool("B1")])).await;
    let mut app_config = config(database_url());
    app_config.mcp.external.limits = ExternalMcpLimitsConfig {
        max_tools_per_session: 3,
        ..ExternalMcpLimitsConfig::default()
    };
    let voice = start_with_config(app_config, ConstantSecrets(None)).await;
    let pool = voice.database().await;
    seed(&pool).await;
    bind_server(&pool, "alpha", &first.url(), "{}", ("none", None, None)).await;
    bind_server(&pool, "beta", &second.url(), "{}", ("none", None, None)).await;

    assert!(
        voice.admit().await.external_mcp().is_empty(),
        "an aggregate overflow must not leave a partial catalog behind"
    );
    assert_eq!(voice.upgrade().await, WsStatusCode::SWITCHING_PROTOCOLS);
    voice.task.abort();
    first.task.abort();
    second.task.abort();
}
