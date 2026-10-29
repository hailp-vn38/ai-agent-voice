use super::*;
use crate::{config::AppConfig, providers::ProviderSet};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use ring::hmac;
use rmcp::{
    ServiceExt,
    transport::{
        StreamableHttpClientTransport, streamable_http_client::StreamableHttpClientTransportConfig,
    },
};
use serde_json::{Value, json};
const KEY: &str = "test-only dedicated key with at least 32 bytes";
fn jwt(claims: Value) -> String {
    let header = URL_SAFE_NO_PAD.encode(r#"{"alg":"HS256","typ":"JWT"}"#);
    let body = URL_SAFE_NO_PAD.encode(claims.to_string());
    let input = format!("{header}.{body}");
    format!(
        "{input}.{}",
        URL_SAFE_NO_PAD.encode(hmac::sign(
            &hmac::Key::new(hmac::HMAC_SHA256, KEY.as_bytes()),
            input.as_bytes()
        ))
    )
}
fn claims(scope: &str) -> Value {
    let now = crate::database::unix_seconds().unwrap();
    json!({"iss":"https://issuer.example","aud":"https://agent.example/mcp/system","sub":"reference","jti":"reference-one","iat":now,"exp":now+300,"scope":scope})
}
async fn serve(router: Router) -> (String, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let task = tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    (format!("http://{address}"), task)
}
async fn fixture() -> (AppState, Arc<SystemMcp>, SystemToolContext) {
    let (core, ctx, db) = crate::tools::system_mcp::tests::fixture(true, 8192).await;
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
[system_mcp.http]
enabled=true
issuer="https://issuer.example"
audience="https://agent.example/mcp/system"
allowed_origins=["https://admin.example"]
[api]
enabled=true
admin_token="admin-test-token"
"#,
    )
    .unwrap();
    config.database.url = "sqlite::memory:".into();
    let mut state = AppState::from_provider_set_with_database(
        config,
        Arc::new(ProviderSet::unavailable()),
        Some((*db).clone()),
    );
    state.system_mcp = Some(core.clone());
    (state, core, ctx)
}
fn client() -> reqwest::Client {
    let mut headers = http::HeaderMap::new();
    headers.insert(
        header::HOST,
        http::HeaderValue::from_static("agent.example"),
    );
    reqwest::Client::builder()
        .default_headers(headers)
        .build()
        .unwrap()
}
#[tokio::test]
async fn disabled_route_is_absent_and_invalid_enabled_auth_fails_configuration() {
    let (mut state, _, _) = fixture().await;
    assert!(auth::Verifier::from_key(&state.config, "short").is_err());
    assert!(auth::Verifier::from_key(&state.config, &state.config.api.admin_token).is_err());
    Arc::make_mut(&mut state.config).system_mcp.http.enabled = false;
    let (base, task) = serve(crate::app::router_with_state(state)).await;
    assert_eq!(
        client()
            .post(format!("{base}/mcp/system"))
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::NOT_FOUND
    );
    task.abort();
}
#[tokio::test]
async fn rmcp_and_independent_http_client_negotiate_and_share_voice_core_output() {
    let (state, core, voice) = fixture().await;
    let verifier = auth::Verifier::from_key(&state.config, KEY).unwrap();
    let (base, task) =
        serve(authenticated_router(&state, verifier).with_state(state.clone())).await;
    let url = format!("{base}/mcp/system");
    let token = jwt(claims("system:read mcp:read"));
    let mut config = StreamableHttpClientTransportConfig::with_uri(url.clone());
    config.auth_header = Some(token.clone());
    let service = ()
        .serve(StreamableHttpClientTransport::with_client(
            crate::tools::external_mcp::rmcp_adapter::PolicyHttpClient::new(
                url::Url::parse(&url).unwrap(),
                client(),
                65536,
            ),
            config,
        ))
        .await
        .unwrap();
    assert_eq!(service.list_all_tools().await.unwrap().len(), 7);
    let mut params = CallToolRequestParams::new("mcp_list_servers");
    params.arguments = Some(serde_json::Map::new());
    let result = service.call_tool(params).await.unwrap();
    let expected = core
        .call_tool(&voice, "mcp_list_servers", json!({}))
        .await
        .unwrap();
    assert_eq!(result.structured_content, Some(expected.clone()));
    service.cancel().await.unwrap();
    // Independently built HTTP reference client, no shared SDK request/response types.
    let http = client();
    let post = |body: Value| {
        http.post(&url)
            .bearer_auth(&token)
            .header(header::ACCEPT, "application/json, text/event-stream")
            .header("MCP-Protocol-Version", "2025-11-25")
            .json(&body)
    };
    let init:Value=post(json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"independent-http","version":"1"}}})).send().await.unwrap().json().await.unwrap();
    assert_eq!(init["result"]["protocolVersion"], "2025-11-25");
    let tools: Value = post(json!({"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(tools["result"]["tools"].as_array().unwrap().len(), 7);
    let call:Value=post(json!({"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"mcp_list_servers","arguments":{}}})).send().await.unwrap().json().await.unwrap();
    assert_eq!(call["result"]["structuredContent"], expected);
    let denied:Value=post(json!({"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"mcp_request_server_change","arguments":{"server_key":"weather","enabled":false,"expected_revision":1}}})).send().await.unwrap().json().await.unwrap();
    assert_eq!(denied["result"]["isError"], true);
    task.abort();
}
#[tokio::test]
async fn http_rejects_bad_tokens_origin_host_and_live_revocation() {
    let (state, _, _) = fixture().await;
    let db = state.database.clone().unwrap();
    let (base, task) = serve(
        authenticated_router(
            &state,
            auth::Verifier::from_key(&state.config, KEY).unwrap(),
        )
        .with_state(state.clone()),
    )
    .await;
    let url = format!("{base}/mcp/system");
    let http = client();
    for (field, value) in [
        ("aud", json!("admin")),
        ("iss", json!("wrong")),
        ("scope", json!("admin")),
        ("exp", json!(0)),
    ] {
        let mut body = claims("system:read");
        body[field] = value;
        assert_eq!(
            http.post(&url)
                .bearer_auth(jwt(body))
                .send()
                .await
                .unwrap()
                .status(),
            StatusCode::UNAUTHORIZED
        );
    }
    let token = jwt(claims("system:read"));
    assert_eq!(
        http.post(&url)
            .bearer_auth("admin-test-token")
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::UNAUTHORIZED
    );
    for (header, value) in [("Origin", "https://evil.example"), ("Host", "evil.example")] {
        assert_eq!(
            http.post(&url)
                .bearer_auth(&token)
                .header(header, value)
                .header("Accept", "application/json, text/event-stream")
                .json(&json!({"jsonrpc":"2.0","id":1,"method":"tools/list"}))
                .send()
                .await
                .unwrap()
                .status(),
            StatusCode::FORBIDDEN
        );
    }
    db.revoke_system_mcp_token(
        "reference-one",
        crate::database::unix_seconds().unwrap() + 300,
        "revoke",
    )
    .await
    .unwrap();
    assert_eq!(
        http.post(&url)
            .bearer_auth(&token)
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::UNAUTHORIZED
    );
    task.abort();
}

#[tokio::test]
async fn admin_auth_and_origin_precede_claim_and_reject() {
    let (state, core, mut ctx) = fixture().await;
    let db = state.database.clone().unwrap();
    ctx.grants.mcp_manage_request = true;
    db.save_system_mcp_grants(true, ctx.grants, 1, "grant")
        .await
        .unwrap();
    let proposal = core
        .call_tool(
            &ctx,
            "mcp_request_server_change",
            json!({"server_key":"weather","enabled":false,"expected_revision":1}),
        )
        .await
        .unwrap();
    let id = proposal["id"].as_str().unwrap();
    let router = axum::Router::new()
        .nest("/api/admin", crate::app::admin::router(state.clone()))
        .with_state(state.clone());
    let (base, task) = serve(router).await;
    let http = reqwest::Client::new();
    for action in ["confirm", "reject"] {
        let url = format!("{base}/api/admin/system-mcp/change-requests/{id}/{action}");
        assert_eq!(
            http.post(&url)
                .header("Origin", "http://localhost")
                .send()
                .await
                .unwrap()
                .status(),
            StatusCode::UNAUTHORIZED
        );
        assert_eq!(
            http.post(&url)
                .bearer_auth("admin-test-token")
                .header("Origin", "https://evil.example")
                .send()
                .await
                .unwrap()
                .status(),
            StatusCode::FORBIDDEN
        );
        assert_eq!(core.change_requests()[0].status, "pending");
    }
    let url = format!("{base}/api/admin/system-mcp/change-requests/{id}/reject");
    let result: Value = http
        .post(url)
        .bearer_auth("admin-test-token")
        .header("Origin", "http://localhost")
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(result["status"], "rejected");
    task.abort();
}

#[tokio::test]
async fn knowledge_mcp_http_auth_read_write_and_external_client_compatibility() {
    let vault_path = std::env::temp_dir().join(format!(
        "voice-agent-mcp-server-knowledge-test-{}",
        uuid::Uuid::new_v4()
    ));
    std::fs::create_dir_all(&vault_path).expect("create temp vault dir");
    let (mut state, _, _) = fixture().await;
    {
        let cfg = Arc::make_mut(&mut state.config);
        cfg.system_mcp.http.enabled = false;
        cfg.wiki.enabled = true;
        cfg.wiki.vault_dir = vault_path.clone();
        cfg.orchestrator.store_dir = vault_path.join("store");
        cfg.wiki.mcp.enabled = true;
        cfg.wiki.mcp.path = "/mcp/knowledge".to_string();
        cfg.wiki.mcp.allow_write = true;
        cfg.wiki.mcp.read_token = "knowledge-read-secret-123456".to_string();
        cfg.wiki.mcp.write_token = "knowledge-write-secret-654321".to_string();
    }

    let (base, task) = serve(crate::app::router_with_state(state)).await;
    let url = format!("{base}/mcp/knowledge");
    let http = reqwest::Client::new();

    // 1. Unauthenticated or bad token -> 401 UNAUTHORIZED
    assert_eq!(
        http.post(&url)
            .json(&json!({"jsonrpc":"2.0","id":1,"method":"tools/list"}))
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        http.post(&url)
            .bearer_auth("wrong-token")
            .json(&json!({"jsonrpc":"2.0","id":1,"method":"tools/list"}))
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::UNAUTHORIZED
    );

    // 2. Write token via RMCP Streamable HTTP client -> 14 tools, can write & stage
    let mut rw_cfg = StreamableHttpClientTransportConfig::with_uri(url.clone());
    rw_cfg.auth_header = Some("knowledge-write-secret-654321".to_string());
    let rw_service = ()
        .serve(StreamableHttpClientTransport::with_client(
            crate::tools::external_mcp::rmcp_adapter::PolicyHttpClient::new(
                url::Url::parse(&url).unwrap(),
                http.clone(),
                262_144,
            ),
            rw_cfg,
        ))
        .await
        .expect("connect write rmcp client");
    let rw_tools = rw_service.list_all_tools().await.expect("list write tools");
    let expected_rw_len = llm_wiki::tools::definitions::build_llm_tool_definitions().len() + 1;
    let expected_ro_len = llm_wiki::tools::definitions::build_llm_tool_definitions()
        .iter()
        .filter(|s| llm_wiki::is_read_only_wiki_tool(&s.name))
        .count();
    assert_eq!(rw_tools.len(), expected_rw_len);
    assert!(rw_tools.iter().any(|t| t.name == "add_to_sources"));
    for tool in &rw_tools {
        let schema_val = Value::Object(tool.input_schema.as_ref().clone());
        crate::tools::external_mcp::registry::validate_external_tool_schema(&schema_val)
            .unwrap_or_else(|e| panic!("wire schema for {} rejected: {e}", tool.name));
    }

    let mut write_call = CallToolRequestParams::new("wiki_write_file");
    write_call.arguments = Some(
        json!({
            "scope": "wiki",
            "rel_path": "entities/knowledge-mcp.md",
            "content": "---\ntitle: Knowledge MCP\ntags:\n  - mcp\n---\n\n# Knowledge MCP\n\nExposes [[llm-wiki]] over Streamable HTTP.\n"
        })
        .as_object()
        .cloned()
        .unwrap(),
    );
    let write_out = rw_service
        .call_tool(write_call)
        .await
        .expect("call wiki_write_file");
    assert_ne!(write_out.is_error, Some(true));
    assert!(write_out.structured_content.is_some());
    rw_service.cancel().await.unwrap();

    // 3. Read-only token via X-API-Key header -> 7 tools, can read, denied write
    let post_ro = |body: Value| {
        http.post(&url)
            .header("X-API-Key", "knowledge-read-secret-123456")
            .header(header::ACCEPT, "application/json, text/event-stream")
            .header("MCP-Protocol-Version", "2025-11-25")
            .json(&body)
    };
    let ro_list: Value =
        post_ro(json!({"jsonrpc":"2.0","id":10,"method":"tools/list","params":{}}))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
    assert_eq!(
        ro_list["result"]["tools"].as_array().unwrap().len(),
        expected_ro_len
    );

    let ro_get: Value = post_ro(json!({
        "jsonrpc": "2.0",
        "id": 11,
        "method": "tools/call",
        "params": {
            "name": "wiki_get",
            "arguments": { "file": "wiki/entities/knowledge-mcp.md" }
        }
    }))
    .send()
    .await
    .unwrap()
    .json()
    .await
    .unwrap();
    assert!(
        ro_get["result"]["structuredContent"]["content"]
            .as_str()
            .unwrap_or_default()
            .contains("Streamable HTTP")
    );

    let ro_denied: Value = post_ro(json!({
        "jsonrpc": "2.0",
        "id": 12,
        "method": "tools/call",
        "params": {
            "name": "wiki_write_file",
            "arguments": {
                "scope": "wiki",
                "rel_path": "entities/blocked.md",
                "content": "# Blocked\n"
            }
        }
    }))
    .send()
    .await
    .unwrap()
    .json()
    .await
    .unwrap();
    assert_eq!(ro_denied["result"]["isError"], true);

    task.abort();
    let _ = std::fs::remove_dir_all(&vault_path);
}

#[tokio::test]
async fn admin_web_can_configure_knowledge_mcp_and_create_revoke_tokens_live() {
    let vault_path = std::env::temp_dir().join(format!(
        "voice-agent-mcp-admin-web-test-{}",
        uuid::Uuid::new_v4()
    ));
    std::fs::create_dir_all(&vault_path).expect("create temp vault dir");
    let (mut state, _, _) = fixture().await;
    {
        let cfg = Arc::make_mut(&mut state.config);
        cfg.system_mcp.http.enabled = false;
        cfg.wiki.enabled = false;
        cfg.wiki.vault_dir = vault_path.clone();
        cfg.orchestrator.store_dir = vault_path.join("store");
        cfg.wiki.mcp.enabled = false;
    }

    let (base, task) = serve(crate::app::router_with_state(state)).await;
    let http = reqwest::Client::new();
    let mcp_url = format!("{base}/mcp/knowledge");
    let admin_url = format!("{base}/api/admin/system-mcp/knowledge");

    // Initially disabled -> 404 NOT_FOUND on /mcp/knowledge
    assert_eq!(
        http.post(&mcp_url)
            .json(&json!({"jsonrpc":"2.0","id":1,"method":"tools/list"}))
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::NOT_FOUND
    );

    // Mutating routes reject untrusted Origin with 403 FORBIDDEN
    assert_eq!(
        http.put(&admin_url)
            .bearer_auth("admin-test-token")
            .header("Origin", "https://evil.example")
            .json(&json!({"enabled": true, "allow_write": true}))
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::FORBIDDEN
    );

    // Enable via Admin Web API with trusted Origin and If-Match revision
    let updated: Value = http
        .put(&admin_url)
        .bearer_auth("admin-test-token")
        .header("Origin", "http://localhost")
        .header("If-Match", "\"1\"")
        .json(&json!({
            "enabled": true,
            "allow_write": true,
            "max_concurrency": 12,
            "call_timeout_ms": 20000,
            "max_result_chars": 10000
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(updated["enabled"], true);
    assert_eq!(updated["base_url"], mcp_url);
    assert_eq!(updated["revision"], 2);

    // Stale If-Match revision returns 409 CONFLICT
    assert_eq!(
        http.put(&admin_url)
            .bearer_auth("admin-test-token")
            .header("Origin", "http://localhost")
            .header("If-Match", "\"1\"")
            .json(&json!({"enabled": false, "allow_write": false}))
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::CONFLICT
    );

    // Create token via Admin Web API
    let created_res: Value = http
        .post(format!("{admin_url}/tokens"))
        .bearer_auth("admin-test-token")
        .header("Origin", "http://localhost")
        .json(&json!({
            "name": "Cursor Agent",
            "access": "read_only"
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let token_id = created_res["created_token"]["id"]
        .as_str()
        .unwrap()
        .to_string();
    let token_secret = created_res["created_token"]["token"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(token_secret.starts_with("kmcp_"));
    // Snapshot tokens list must only contain masked_token, never raw token
    assert!(created_res["tokens"][0].get("token").is_none());
    assert!(
        created_res["tokens"][0]["masked_token"]
            .as_str()
            .unwrap_or_default()
            .contains("••••••••")
    );

    // GET /api/admin/system-mcp/knowledge must also only expose masked_token
    let get_snap: Value = http
        .get(&admin_url)
        .bearer_auth("admin-test-token")
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let get_snap_raw = serde_json::to_string(&get_snap).unwrap();
    assert!(!get_snap_raw.contains(&token_secret));

    // Use newly generated token on /mcp/knowledge without restarting server
    let list_res: Value = http
        .post(&mcp_url)
        .bearer_auth(&token_secret)
        .header(header::ACCEPT, "application/json, text/event-stream")
        .header("MCP-Protocol-Version", "2025-11-25")
        .json(&json!({"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let expected_ro_len = llm_wiki::tools::definitions::build_llm_tool_definitions()
        .iter()
        .filter(|s| llm_wiki::is_read_only_wiki_tool(&s.name))
        .count();
    assert_eq!(
        list_res["result"]["tools"].as_array().unwrap().len(),
        expected_ro_len
    );

    // Delete token via Admin Web API -> immediately 401 UNAUTHORIZED
    let del_status = http
        .delete(format!("{admin_url}/tokens/{token_id}"))
        .bearer_auth("admin-test-token")
        .header("Origin", "http://localhost")
        .send()
        .await
        .unwrap()
        .status();
    assert_eq!(del_status, StatusCode::OK);

    assert_eq!(
        http.post(&mcp_url)
            .bearer_auth(&token_secret)
            .json(&json!({"jsonrpc":"2.0","id":3,"method":"tools/list"}))
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::UNAUTHORIZED
    );

    task.abort();
    let _ = std::fs::remove_dir_all(&vault_path);
}
