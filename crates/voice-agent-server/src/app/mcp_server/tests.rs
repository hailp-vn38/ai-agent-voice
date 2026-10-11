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
    assert_eq!(service.list_all_tools().await.unwrap().len(), 6);
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
    assert_eq!(tools["result"]["tools"].as_array().unwrap().len(), 6);
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
