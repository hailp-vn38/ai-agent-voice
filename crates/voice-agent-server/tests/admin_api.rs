use std::{fs, sync::Arc};

use reqwest::{Client, StatusCode};
use voice_agent_server::{
    app::bootstrap_with_providers, config::AppConfig, providers::ProviderSet,
};

fn database_url() -> String {
    format!(
        "sqlite://{}",
        std::env::temp_dir()
            .join(format!("voice-agent-admin-{}.db", uuid::Uuid::new_v4()))
            .display()
    )
}

async fn server(api_enabled: bool) -> (String, tokio::task::JoinHandle<()>) {
    let config_path =
        std::env::temp_dir().join(format!("voice-agent-admin-{}.toml", uuid::Uuid::new_v4()));
    fs::write(
        &config_path,
        format!(
            r#"
[server]
bind = "127.0.0.1:0"
public_ws_url = "ws://127.0.0.1:0/voice/v1/"
[provider_defaults]
vad = "test"
asr = "test"
llm = "test"
tts = "test"
[database]
enabled = true
url = "{}"
[api]
enabled = {}
admin_token = "admin-test-token"
[mcp.external.network]
allowed_hosts = ["mcp.example.test"]
"#,
            database_url(),
            api_enabled
        ),
    )
    .unwrap();
    let config = AppConfig::parse_and_resolve(&config_path).unwrap();
    fs::remove_file(config_path).unwrap();
    let router = bootstrap_with_providers(config, Arc::new(ProviderSet::unavailable()))
        .await
        .unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let task = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    (format!("http://{address}"), task)
}

#[tokio::test]
async fn admin_router_auth_crud_revision_and_transport_contract() {
    let (base, task) = server(true).await;
    let client = Client::new();
    let url = format!("{base}/api/admin/agents");

    let unauthorized = client.get(&url).send().await.unwrap();
    assert_eq!(unauthorized.status(), StatusCode::UNAUTHORIZED);
    assert!(unauthorized.headers().contains_key("x-request-id"));
    assert_eq!(
        client
            .get(&url)
            .header("authorization", "Bearer admin-test-token")
            .header("authorization", "Bearer admin-test-token")
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        client
            .post(&url)
            .bearer_auth("admin-test-token")
            .header("content-encoding", "gzip")
            .header("content-type", "application/json")
            .body("{}")
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::UNSUPPORTED_MEDIA_TYPE
    );
    assert_eq!(
        client
            .post(&url)
            .bearer_auth("admin-test-token")
            .header("content-type", "application/json; nonsense")
            .body("{}")
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        client
            .get(format!("{base}/api/admin/agents?page_size=201"))
            .bearer_auth("admin-test-token")
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::BAD_REQUEST
    );

    let created = client
        .post(&url)
        .bearer_auth("admin-test-token")
        .json(&serde_json::json!({"key":"kitchen","name":"Kitchen"}))
        .send()
        .await
        .unwrap();
    assert_eq!(created.status(), StatusCode::CREATED);
    let agent: serde_json::Value = created.json().await.unwrap();
    assert_eq!(agent["key"], "kitchen");
    assert_eq!(agent["revision"], 1);

    let device_url = format!("{base}/api/admin/devices");
    let device = client.post(&device_url).bearer_auth("admin-test-token").json(&serde_json::json!({"device_id":"device-1","agent_key":"kitchen","metadata_json":{"zone":"home"}})).send().await.unwrap();
    assert_eq!(device.status(), StatusCode::CREATED);
    let device: serde_json::Value = device.json().await.unwrap();
    assert_eq!(device["device_id"], "device-1");

    let patch_url = format!("{base}/api/admin/agents/kitchen");
    let missing_match = client
        .patch(&patch_url)
        .bearer_auth("admin-test-token")
        .json(&serde_json::json!({"description": "primary"}))
        .send()
        .await
        .unwrap();
    assert_eq!(missing_match.status(), StatusCode::BAD_REQUEST);
    let changed = client
        .patch(&patch_url)
        .bearer_auth("admin-test-token")
        .header("if-match", "\"1\"")
        .json(&serde_json::json!({"description": "primary"}))
        .send()
        .await
        .unwrap();
    assert_eq!(changed.status(), StatusCode::OK);
    assert_eq!(
        changed.json::<serde_json::Value>().await.unwrap()["revision"],
        2
    );
    assert_eq!(
        client
            .patch(&patch_url)
            .bearer_auth("admin-test-token")
            .header("if-match", "\"1\"")
            .json(&serde_json::json!({"enabled":false}))
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::CONFLICT
    );
    assert_eq!(
        client
            .patch(format!("{base}/api/admin/devices/device-1"))
            .bearer_auth("admin-test-token")
            .header("if-match", "\"1\"")
            .json(&serde_json::json!({"agent_key": null}))
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        client
            .get(format!("{base}/api/admin/admin_audit_events"))
            .bearer_auth("admin-test-token")
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::NOT_FOUND
    );
    task.abort();
}

#[tokio::test]
async fn disabled_admin_route_is_not_mounted() {
    let (base, task) = server(false).await;
    assert_eq!(
        reqwest::get(format!("{base}/api/admin/agents"))
            .await
            .unwrap()
            .status(),
        StatusCode::NOT_FOUND
    );
    task.abort();
}

#[tokio::test]
async fn templates_and_provider_desired_configuration_are_bounded_and_restart_honest() {
    let (base, task) = server(true).await;
    let client = Client::new();
    let auth = "admin-test-token";

    let agent = client
        .post(format!("{base}/api/admin/agents"))
        .bearer_auth(auth)
        .json(&serde_json::json!({"key":"living_room","name":"Living room"}))
        .send()
        .await
        .unwrap()
        .json::<serde_json::Value>()
        .await
        .unwrap();
    assert_eq!(agent["revision"], 1);

    let provider_url = format!("{base}/api/admin/providers");
    let protected = client
        .post(&provider_url)
        .bearer_auth(auth)
        .json(&serde_json::json!({
            "key":"bad_openai", "name":"Bad", "type":"llm", "adapter":"openai",
            "config_json":{"model":"x","nested":{"api_key":"nope"}}
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(protected.status(), StatusCode::BAD_REQUEST);
    assert_eq!(
        protected.json::<serde_json::Value>().await.unwrap()["error"]["code"],
        "provider_config_invalid"
    );

    let llm = client
        .post(&provider_url)
        .bearer_auth(auth)
        .json(&serde_json::json!({
            "key":"llm_main", "name":"LLM", "type":"llm", "adapter":"openai",
            "config_json":{"base_url":"https://example.test/v1","model":"test","max_tokens":8},
            "secret_ref":"LLM_SECRET"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(llm.status(), StatusCode::CREATED);
    let llm = llm.json::<serde_json::Value>().await.unwrap();
    assert_eq!(llm["has_secret_ref"], true);
    assert!(llm.get("secret_ref").is_none());
    assert!(llm.get("runtime_status").is_none());
    assert!(llm.get("requires_restart").is_none());

    for (key, kind, adapter, config_json) in [
        (
            "vad_main",
            "vad",
            "silero_onnx",
            serde_json::json!({"model":"silero","num_threads":1}),
        ),
        (
            "asr_main",
            "asr",
            "zipformer_sherpa",
            serde_json::json!({"model":"zipformer","num_threads":1,"decoding_method":"greedy_search"}),
        ),
        (
            "tts_main",
            "tts",
            "zerotts_onnx",
            serde_json::json!({"model":"zerotts","num_threads":1,"voice":"vi"}),
        ),
    ] {
        assert_eq!(
            client
                .post(&provider_url)
                .bearer_auth(auth)
                .json(&serde_json::json!({
                    "key":key,"name":key,"type":kind,"adapter":adapter,"config_json":config_json
                }))
                .send()
                .await
                .unwrap()
                .status(),
            StatusCode::CREATED
        );
    }

    let templates_url = format!("{base}/api/admin/templates");
    let template = client
        .post(&templates_url)
        .bearer_auth(auth)
        .json(&serde_json::json!({
            "key":"quiet","name":"Quiet","language":"vi-VN","prompt":"Nói ngắn gọn"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(template.status(), StatusCode::CREATED);
    let mut template = template.json::<serde_json::Value>().await.unwrap();

    for (kind, provider_key) in [
        ("vad", "vad_main"),
        ("asr", "asr_main"),
        ("llm", "llm_main"),
        ("tts", "tts_main"),
    ] {
        let response = client
            .put(format!("{base}/api/admin/templates/quiet/providers/{kind}"))
            .bearer_auth(auth)
            .header("if-match", format!("\"{}\"", template["revision"]))
            .json(&serde_json::json!({"provider_key":provider_key}))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        template = response.json().await.unwrap();
    }
    let assigned = client
        .put(format!(
            "{base}/api/admin/agents/living_room/default-template/quiet"
        ))
        .bearer_auth(auth)
        .header("if-match", "\"1\"")
        .send()
        .await
        .unwrap();
    assert_eq!(assigned.status(), StatusCode::OK);
    task.abort();
}

#[tokio::test]
async fn external_mcp_configuration_is_redacted_validated_and_revisioned() {
    let (base, task) = server(true).await;
    let client = Client::new();
    let auth = "admin-test-token";
    assert_eq!(
        client
            .post(format!("{base}/api/admin/agents"))
            .bearer_auth(auth)
            .json(&serde_json::json!({"key":"kitchen","name":"Kitchen"}))
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::CREATED
    );
    let servers = format!("{base}/api/admin/mcp-servers");
    assert_eq!(
        client
            .post(&servers)
            .bearer_auth(auth)
            .json(&serde_json::json!({"key":"outside","name":"Outside","url":"https://outside.example.test/","auth":{"type":"none"}}))
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::BAD_REQUEST
    );
    assert_eq!(client.post(&servers).bearer_auth(auth).json(&serde_json::json!({"key":"bad","name":"Bad","url":"https://mcp.example.test/?token=secret","headers":{"authorization":"nope"},"auth":{"type":"none"}})).send().await.unwrap().status(), StatusCode::BAD_REQUEST);
    let created = client.post(&servers).bearer_auth(auth).json(&serde_json::json!({"key":"weather","name":"Weather","url":"https://mcp.example.test/tools","headers":{"x-client":"voice-agent"},"auth":{"type":"header","header_name":"x-api-key","secret_ref":"MCP_WEATHER_KEY"}})).send().await.unwrap();
    assert_eq!(created.status(), StatusCode::CREATED);
    let server: serde_json::Value = created.json().await.unwrap();
    assert_eq!(
        server["auth"],
        serde_json::json!({"type":"header","header_name":"x-api-key","has_secret_ref":true})
    );
    assert!(server.get("secret_ref").is_none());
    assert_eq!(
        client
            .post(&servers)
            .bearer_auth(auth)
            .json(&serde_json::json!({"key":"unsafe_auth","name":"Unsafe auth","url":"https://mcp.example.test/","auth":{"type":"header","header_name":"authorization","secret_ref":"MCP_WEATHER_KEY"}}))
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::BAD_REQUEST
    );
    for header in ["proxy-authenticate", "www-authenticate", "keep-alive"] {
        assert_eq!(
            client
                .post(&servers)
                .bearer_auth(auth)
                .json(&serde_json::json!({"key":format!("blocked_{}", header.replace('-', "_")),"name":"Blocked","url":"https://mcp.example.test/","headers":{header:"value"},"auth":{"type":"none"}}))
                .send()
                .await
                .unwrap()
                .status(),
            StatusCode::BAD_REQUEST,
            "{header} must be protected"
        );
    }
    let binding = client
        .put(format!(
            "{base}/api/admin/agents/kitchen/mcp-bindings/weather"
        ))
        .bearer_auth(auth)
        .header("if-match", "\"1\"")
        .json(&serde_json::json!({"enabled":true,"required":false}))
        .send()
        .await
        .unwrap();
    assert_eq!(binding.status(), StatusCode::OK);
    assert_eq!(
        binding.json::<serde_json::Value>().await.unwrap()["revision"],
        2
    );
    assert_eq!(
        client
            .put(format!(
                "{base}/api/admin/agents/kitchen/mcp-bindings/weather"
            ))
            .bearer_auth(auth)
            .header("if-match", "\"1\"")
            .json(&serde_json::json!({"enabled":false,"required":false}))
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::CONFLICT
    );
    assert_eq!(
        client
            .put(format!(
                "{base}/api/admin/agents/kitchen/mcp-bindings/weather"
            ))
            .bearer_auth(auth)
            .header("if-match", "\"2\"")
            .json(&serde_json::json!({"enabled":true,"required":true}))
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::BAD_REQUEST
    );
    task.abort();
}
