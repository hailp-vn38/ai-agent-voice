use std::{
    fs,
    sync::{Arc, Mutex},
    time::Duration,
};

use reqwest::{Client, StatusCode};
use voice_agent_server::{
    app::{AppState, bootstrap_with_providers, router_with_state},
    config::AppConfig,
    database::Database,
    providers::{
        LlmError, LlmProvider, ProviderSet,
        llm::{ChatMessage, LlmRequest},
    },
};

struct DiagnosticLlm {
    requests: Arc<Mutex<Vec<LlmRequest>>>,
}

impl LlmProvider for DiagnosticLlm {
    fn adapter(&self) -> &'static str {
        "diagnostic-test"
    }

    fn complete(&self, request: &LlmRequest) -> Result<String, LlmError> {
        self.requests.lock().unwrap().push(request.clone());
        Ok("diagnostic answer".into())
    }
}

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

async fn server_with_loaded_llm() -> (
    String,
    Arc<Mutex<Vec<LlmRequest>>>,
    tokio::task::JoinHandle<()>,
) {
    let config_path = std::env::temp_dir().join(format!(
        "voice-agent-admin-diagnostic-{}.toml",
        uuid::Uuid::new_v4()
    ));
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
enabled = true
admin_token = "admin-test-token"
[mcp.external.network]
allowed_hosts = ["mcp.example.test"]
"#,
            database_url(),
        ),
    )
    .unwrap();
    let config = AppConfig::parse_and_resolve(&config_path).unwrap();
    fs::remove_file(config_path).unwrap();
    let database = Database::connect_if_enabled(&config.database)
        .await
        .unwrap();
    let requests = Arc::new(Mutex::new(Vec::new()));
    let state = AppState::from_provider_set_with_database(
        config,
        Arc::new(ProviderSet::unavailable()),
        database,
    )
    .with_database_llm_runtime_for_test(
        "llm_loaded",
        Arc::new(DiagnosticLlm {
            requests: Arc::clone(&requests),
        }),
        2,
        Duration::from_secs(1),
        1,
    );
    let router = router_with_state(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let task = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    (format!("http://{address}"), requests, task)
}

#[tokio::test]
async fn llm_provider_test_uses_only_the_loaded_runtime_and_a_tool_free_request() {
    let (base, requests, task) = server_with_loaded_llm().await;
    let client = Client::new();
    let providers = format!("{base}/api/admin/providers");
    let created = client
        .post(&providers)
        .bearer_auth("admin-test-token")
        .json(&serde_json::json!({
            "key": "llm_loaded",
            "name": "Loaded LLM",
            "type": "llm",
            "adapter": "openai",
            "config_json": {"base_url":"https://example.test/v1","model":"test","max_tokens":8}
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(created.status(), StatusCode::CREATED);

    let response = client
        .post(format!("{providers}/llm_loaded/test/llm"))
        .bearer_auth("admin-test-token")
        .json(&serde_json::json!({"input":"xin chao"}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let response = response.json::<serde_json::Value>().await.unwrap();
    assert_eq!(response["provider_key"], "llm_loaded");
    assert_eq!(response["type"], "llm");
    assert_eq!(response["status"], "success");
    assert_eq!(response["result"]["text"], "diagnostic answer");
    assert_eq!(response["runtime"]["runtime_status"], "loaded");
    assert_eq!(response["runtime"]["tested_runtime"], "loaded");
    assert_eq!(response["runtime"]["runtime_matches_desired"], true);
    assert_eq!(response["runtime"]["requires_restart"], false);
    assert!(response["metrics"]["elapsed_ms"].is_u64());

    let changed = client
        .patch(format!("{providers}/llm_loaded"))
        .bearer_auth("admin-test-token")
        .header("if-match", "\"1\"")
        .json(&serde_json::json!({"name":"Changed desired state"}))
        .send()
        .await
        .unwrap();
    assert_eq!(changed.status(), StatusCode::OK);
    let stale = client
        .post(format!("{providers}/llm_loaded/test/llm"))
        .bearer_auth("admin-test-token")
        .json(&serde_json::json!({"input":"still loaded"}))
        .send()
        .await
        .unwrap()
        .json::<serde_json::Value>()
        .await
        .unwrap();
    assert_eq!(stale["status"], "success");
    assert_eq!(stale["runtime"]["runtime_matches_desired"], false);
    assert_eq!(stale["runtime"]["requires_restart"], true);

    let requests = requests.lock().unwrap();
    assert_eq!(requests.len(), 2);
    assert!(requests[0].tools.is_empty());
    assert_eq!(
        requests[0].messages,
        vec![ChatMessage::User {
            content: "xin chao".into()
        }]
    );
    drop(requests);

    let invalid = client
        .post(format!("{providers}/llm_loaded/test/llm"))
        .bearer_auth("admin-test-token")
        .json(&serde_json::json!({"input":""}))
        .send()
        .await
        .unwrap();
    assert_eq!(invalid.status(), StatusCode::BAD_REQUEST);
    assert_eq!(
        invalid.json::<serde_json::Value>().await.unwrap()["error"]["code"],
        "invalid_test_input"
    );
    task.abort();
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
async fn provider_adapter_descriptors_and_bootstrap_discovery_are_public_read_only_contracts() {
    let (base, task) = server(true).await;
    let client = Client::new();
    let adapters = format!("{base}/api/admin/provider-adapters");

    assert_eq!(
        client.get(&adapters).send().await.unwrap().status(),
        StatusCode::UNAUTHORIZED
    );

    let listed = client
        .get(format!("{adapters}?type=tts"))
        .bearer_auth("admin-test-token")
        .send()
        .await
        .unwrap();
    assert_eq!(listed.status(), StatusCode::OK);
    assert_eq!(
        listed.json::<serde_json::Value>().await.unwrap()["items"],
        serde_json::json!([
            {
                "adapter": "zerotts_onnx",
                "type": "tts",
                "display_name": "ZeroTTS"
            },
            {
                "adapter": "chillaudio_ws",
                "type": "tts",
                "display_name": "ChillAudio WebSocket"
            }
        ])
    );
    let bad_filter = client
        .get(format!("{adapters}?type=vision"))
        .bearer_auth("admin-test-token")
        .send()
        .await
        .unwrap();
    assert_eq!(bad_filter.status(), StatusCode::BAD_REQUEST);
    assert_eq!(
        bad_filter.json::<serde_json::Value>().await.unwrap()["error"]["code"],
        "validation_failed"
    );

    let descriptor = client
        .get(format!("{adapters}/zerotts_onnx"))
        .bearer_auth("admin-test-token")
        .send()
        .await
        .unwrap();
    assert_eq!(descriptor.status(), StatusCode::OK);
    let descriptor = descriptor.json::<serde_json::Value>().await.unwrap();
    assert_eq!(
        descriptor["capabilities"]["provider_output_sample_rates"],
        serde_json::json!([48_000])
    );
    assert_eq!(
        descriptor["capabilities"]["voice_delivery_sample_rates"],
        serde_json::json!([24_000])
    );
    assert!(
        descriptor["config_schema"]["fields"]
            .as_array()
            .unwrap()
            .iter()
            .any(|field| field["key"] == "language")
    );
    assert_eq!(descriptor["discovery"]["voices"], "bootstrap_and_runtime");
    assert_eq!(
        descriptor["config_schema"]["fields"]
            .as_array()
            .unwrap()
            .iter()
            .find(|field| field["key"] == "delivery_mode")
            .unwrap()["enum_values"],
        serde_json::json!(["stream", "file"])
    );

    let discovered = client
        .post(format!("{adapters}/zerotts_onnx/capabilities/discover"))
        .bearer_auth("admin-test-token")
        .json(&serde_json::json!({"selection":{"model":"zerotts_default"}}))
        .send()
        .await
        .unwrap();
    assert_eq!(discovered.status(), StatusCode::OK);
    let discovered = discovered.json::<serde_json::Value>().await.unwrap();
    assert_eq!(discovered["voices"][0]["id"], "maichi");
    assert_eq!(discovered["languages"][0]["id"], "vi-VN");

    let invalid = client
        .post(format!("{adapters}/zerotts_onnx/capabilities/discover"))
        .bearer_auth("admin-test-token")
        .json(&serde_json::json!({"selection":{"api_key":"not-allowed"}}))
        .send()
        .await
        .unwrap();
    assert_eq!(invalid.status(), StatusCode::BAD_REQUEST);
    assert_eq!(
        invalid.json::<serde_json::Value>().await.unwrap()["error"]["code"],
        "capability_discovery_invalid"
    );

    task.abort();
}

#[tokio::test]
async fn descriptor_language_fields_match_provider_config_migration_and_validation() {
    let (base, task) = server(true).await;
    let client = Client::new();
    let providers = format!("{base}/api/admin/providers");

    let legacy = client
        .post(&providers)
        .bearer_auth("admin-test-token")
        .json(&serde_json::json!({
            "key": "zerotts_legacy",
            "name": "Legacy ZeroTTS",
            "type": "tts",
            "adapter": "zerotts_onnx",
            "config_json": {
                "model": "zerotts_default",
                "voice": "maichi",
                "num_threads": 1
            }
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(legacy.status(), StatusCode::CREATED);
    let legacy = legacy.json::<serde_json::Value>().await.unwrap();
    assert_eq!(
        legacy["config_json"],
        "{\"model\":\"zerotts_default\",\"num_threads\":1,\"voice\":\"maichi\",\"language\":\"vi-VN\",\"preload\":false,\"delivery_mode\":\"stream\"}"
    );
    let mut canonical: serde_json::Value =
        serde_json::from_str(legacy["config_json"].as_str().unwrap()).unwrap();
    canonical
        .as_object_mut()
        .unwrap()
        .insert("adapter".into(), serde_json::json!("zerotts_onnx"));
    assert!(
        serde_json::from_value::<voice_agent_server::config::TtsInstanceConfig>(canonical).is_ok()
    );

    let incompatible = client
        .post(&providers)
        .bearer_auth("admin-test-token")
        .json(&serde_json::json!({
            "key": "zerotts_wrong_language",
            "name": "Wrong language",
            "type": "tts",
            "adapter": "zerotts_onnx",
            "config_json": {
                "model": "zerotts_default",
                "voice": "maichi",
                "language": "en-US",
                "num_threads": 1
            }
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(incompatible.status(), StatusCode::BAD_REQUEST);
    assert_eq!(
        incompatible.json::<serde_json::Value>().await.unwrap()["error"]["code"],
        "provider_config_invalid"
    );

    let chillaudio = client
        .post(&providers)
        .bearer_auth("admin-test-token")
        .json(&serde_json::json!({
            "key": "chillaudio_main",
            "name": "ChillAudio",
            "type": "tts",
            "adapter": "chillaudio_ws",
            "config_json": {
                "ws_url": "wss://tts.example.test/socket",
                "voice": "BV421_vivn_streaming"
            }
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(chillaudio.status(), StatusCode::CREATED);

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
    assert_eq!(llm["runtime_status"], "not_loaded");
    assert_eq!(llm["runtime_matches_desired"], false);
    assert_eq!(llm["requires_restart"], true);

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
