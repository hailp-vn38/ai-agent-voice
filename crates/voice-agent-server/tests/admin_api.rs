use std::{
    fs,
    sync::{Arc, Mutex},
    time::Duration,
};

use reqwest::{Client, StatusCode};
use voice_agent_server::{
    app::{AppState, bootstrap_with_providers, router_with_state},
    audio::PcmF32Mono,
    config::AppConfig,
    database::Database,
    providers::{
        AsrError, AsrEvent, AsrProvider, AsrResult, AsrSession, LlmError, LlmProvider, ProviderSet,
        TtsDiagnosticRequest, TtsError, TtsProvider, VadError, VadInput, VadProbability,
        VadProvider, VadSession,
        llm::{ChatMessage, LlmRequest},
    },
    workers::WorkerRuntimeConfig,
};

struct DiagnosticLlm {
    requests: Arc<Mutex<Vec<LlmRequest>>>,
}

struct DiagnosticTts {
    requests: Arc<Mutex<Vec<TtsDiagnosticRequest>>>,
}

struct DiagnosticAsr {
    received_samples: Arc<Mutex<Vec<usize>>>,
}

struct DiagnosticAsrSession {
    received_samples: Arc<Mutex<Vec<usize>>>,
    samples: usize,
}

struct DiagnosticVad {
    inputs: Arc<Mutex<Vec<VadInput>>>,
}

struct DiagnosticVadSession {
    inputs: Arc<Mutex<Vec<VadInput>>>,
}

impl VadProvider for DiagnosticVad {
    fn open(&self) -> Result<Box<dyn VadSession>, VadError> {
        Ok(Box::new(DiagnosticVadSession {
            inputs: Arc::clone(&self.inputs),
        }))
    }

    fn adapter(&self) -> &'static str {
        "diagnostic-vad"
    }
}

impl VadSession for DiagnosticVadSession {
    fn push(&mut self, input: VadInput) -> Result<VadProbability, VadError> {
        self.inputs.lock().unwrap().push(input.clone());
        Ok(VadProbability {
            start_sample: input.start_sample,
            end_sample: input.start_sample + 512,
            probability: 0.25,
        })
    }

    fn reset(&mut self) -> Result<(), VadError> {
        Ok(())
    }
}

impl AsrProvider for DiagnosticAsr {
    fn open(&self) -> Result<Box<dyn AsrSession>, AsrError> {
        Ok(Box::new(DiagnosticAsrSession {
            received_samples: Arc::clone(&self.received_samples),
            samples: 0,
        }))
    }
}

impl AsrSession for DiagnosticAsrSession {
    fn push_pcm(&mut self, pcm: &PcmF32Mono) -> Result<Vec<AsrEvent>, AsrError> {
        self.samples += pcm.samples().len();
        Ok(Vec::new())
    }

    fn finish(&mut self) -> Result<AsrResult, AsrError> {
        self.received_samples.lock().unwrap().push(self.samples);
        Ok(AsrResult::new("xin chao"))
    }

    fn cancel(&mut self) {}
}

impl TtsProvider for DiagnosticTts {
    fn adapter(&self) -> &'static str {
        "diagnostic-tts"
    }

    fn validate_diagnostic(&self, _: &TtsDiagnosticRequest) -> Result<(), TtsError> {
        Ok(())
    }

    fn synthesize_diagnostic(
        &self,
        request: &TtsDiagnosticRequest,
        _: &std::sync::atomic::AtomicBool,
        on_pcm: &mut dyn FnMut(PcmF32Mono) -> Result<(), TtsError>,
    ) -> Result<(), TtsError> {
        self.requests.lock().unwrap().push(request.clone());
        on_pcm(PcmF32Mono::new(vec![0.0, 0.5, -0.5], 48_000))
    }
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
    server_with_database(api_enabled, &database_url()).await
}
async fn server_with_database(
    api_enabled: bool,
    database_uri: &str,
) -> (String, tokio::task::JoinHandle<()>) {
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
url = "{}"
[api]
enabled = {}
admin_token = "admin-test-token"
[mcp.external.network]
allowed_hosts = ["mcp.example.test"]
"#,
            database_uri, api_enabled
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

/// Keeps the Admin router mounted while making the already-configured database unavailable.
/// This exercises the public status route's degradation response rather than a startup failure.
async fn server_with_closed_database() -> (String, tokio::task::JoinHandle<()>) {
    let config_path = std::env::temp_dir().join(format!(
        "voice-agent-admin-closed-{}.toml",
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
url = "{}"
[api]
enabled = true
admin_token = "admin-test-token"
"#,
            database_url()
        ),
    )
    .unwrap();
    let config = AppConfig::parse_and_resolve(&config_path).unwrap();
    fs::remove_file(config_path).unwrap();
    let state = AppState::from_provider_set_with_database(
        config.clone(),
        Arc::new(ProviderSet::unavailable()),
        Some(Database::connect(&config.database).await.unwrap()),
    );
    state.database.as_ref().unwrap().pool().close().await;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let task = tokio::spawn(async move {
        axum::serve(listener, router_with_state(state))
            .await
            .unwrap()
    });
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
    let database = Some(Database::connect(&config.database).await.unwrap());
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

async fn server_with_loaded_tts() -> (
    String,
    Arc<Mutex<Vec<TtsDiagnosticRequest>>>,
    tokio::task::JoinHandle<()>,
) {
    let config_path = std::env::temp_dir().join(format!(
        "voice-agent-admin-tts-diagnostic-{}.toml",
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
url = "{}"
[api]
enabled = true
admin_token = "admin-test-token"
[mcp.external.network]
allowed_hosts = ["mcp.example.test"]
"#,
            database_url()
        ),
    )
    .unwrap();
    let config = AppConfig::parse_and_resolve(&config_path).unwrap();
    fs::remove_file(config_path).unwrap();
    let database = Some(Database::connect(&config.database).await.unwrap());
    let requests = Arc::new(Mutex::new(Vec::new()));
    let state = AppState::from_provider_set_with_database(
        config,
        Arc::new(ProviderSet::unavailable()),
        database,
    )
    .with_database_tts_runtime_for_test(
        "tts_loaded",
        Arc::new(DiagnosticTts {
            requests: Arc::clone(&requests),
        }),
        WorkerRuntimeConfig {
            max_workers: 2,
            voice_reserved_capacity: 1,
            command_capacity: 4,
            final_timeout: Duration::from_secs(1),
            cleanup_grace: Duration::from_millis(100),
        },
        1,
    );
    let router = router_with_state(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let task = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    (format!("http://{address}"), requests, task)
}

async fn server_with_loaded_asr() -> (String, Arc<Mutex<Vec<usize>>>, tokio::task::JoinHandle<()>) {
    let config_path = std::env::temp_dir().join(format!(
        "voice-agent-admin-asr-diagnostic-{}.toml",
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
url = "{}"
[api]
enabled = true
admin_token = "admin-test-token"
[mcp.external.network]
allowed_hosts = ["mcp.example.test"]
"#,
            database_url()
        ),
    )
    .unwrap();
    let config = AppConfig::parse_and_resolve(&config_path).unwrap();
    fs::remove_file(config_path).unwrap();
    let database = Some(Database::connect(&config.database).await.unwrap());
    let received_samples = Arc::new(Mutex::new(Vec::new()));
    let state = AppState::from_provider_set_with_database(
        config,
        Arc::new(ProviderSet::unavailable()),
        database,
    )
    .with_database_asr_runtime_for_test(
        "asr_loaded",
        Arc::new(DiagnosticAsr {
            received_samples: Arc::clone(&received_samples),
        }),
        WorkerRuntimeConfig {
            max_workers: 2,
            voice_reserved_capacity: 1,
            command_capacity: 4,
            final_timeout: Duration::from_secs(1),
            cleanup_grace: Duration::from_millis(100),
        },
        1,
    );
    let router = router_with_state(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let task = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    (format!("http://{address}"), received_samples, task)
}

async fn server_with_loaded_vad() -> (
    String,
    Arc<Mutex<Vec<VadInput>>>,
    tokio::task::JoinHandle<()>,
) {
    let config_path = std::env::temp_dir().join(format!(
        "voice-agent-admin-vad-diagnostic-{}.toml",
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
url = "{}"
[api]
enabled = true
admin_token = "admin-test-token"
"#,
            database_url()
        ),
    )
    .unwrap();
    let config = AppConfig::parse_and_resolve(&config_path).unwrap();
    fs::remove_file(config_path).unwrap();
    let database = Some(Database::connect(&config.database).await.unwrap());
    let inputs = Arc::new(Mutex::new(Vec::new()));
    let state = AppState::from_provider_set_with_database(
        config,
        Arc::new(ProviderSet::unavailable()),
        database,
    )
    .with_database_vad_runtime_for_test(
        "vad_loaded",
        Arc::new(DiagnosticVad {
            inputs: Arc::clone(&inputs),
        }),
        WorkerRuntimeConfig {
            max_workers: 2,
            voice_reserved_capacity: 1,
            command_capacity: 4,
            final_timeout: Duration::from_secs(1),
            cleanup_grace: Duration::from_millis(100),
        },
        1,
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let task = tokio::spawn(async move {
        axum::serve(listener, router_with_state(state))
            .await
            .unwrap()
    });
    (format!("http://{address}"), inputs, task)
}

fn wav_pcm16_mono(sample_rate: u32, samples: usize) -> Vec<u8> {
    let data_bytes = u32::try_from(samples.checked_mul(2).unwrap()).unwrap();
    let mut wav = Vec::with_capacity(44 + data_bytes as usize);
    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&(36 + data_bytes).to_le_bytes());
    wav.extend_from_slice(b"WAVEfmt ");
    wav.extend_from_slice(&16_u32.to_le_bytes());
    wav.extend_from_slice(&1_u16.to_le_bytes());
    wav.extend_from_slice(&1_u16.to_le_bytes());
    wav.extend_from_slice(&sample_rate.to_le_bytes());
    wav.extend_from_slice(&sample_rate.checked_mul(2).unwrap().to_le_bytes());
    wav.extend_from_slice(&2_u16.to_le_bytes());
    wav.extend_from_slice(&16_u16.to_le_bytes());
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&data_bytes.to_le_bytes());
    wav.resize(44 + data_bytes as usize, 0);
    wav
}

#[tokio::test]
async fn vad_provider_test_uses_one_canonical_silent_frame_from_the_loaded_runtime() {
    let (base, inputs, task) = server_with_loaded_vad().await;
    let client = Client::new();
    let providers = format!("{base}/api/admin/providers");
    assert_eq!(
        client
            .post(&providers)
            .bearer_auth("admin-test-token")
            .json(&serde_json::json!({
                "key":"vad_loaded", "name":"Loaded VAD", "type":"vad", "adapter":"silero_onnx",
                "config_json":{}
            }))
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::CREATED
    );
    let response = client
        .post(format!("{providers}/vad_loaded/test/vad"))
        .bearer_auth("admin-test-token")
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let response = response.json::<serde_json::Value>().await.unwrap();
    assert_eq!(response["provider_key"], "vad_loaded");
    assert_eq!(response["type"], "vad");
    assert_eq!(response["result"]["probability"], 0.25);
    assert_eq!(response["result"]["start_sample"], 0);
    assert_eq!(response["result"]["end_sample"], 512);
    assert_eq!(
        inputs.lock().unwrap().as_slice(),
        &[VadInput {
            pcm: vec![0.0; 512],
            start_sample: 0
        }]
    );
    let nonempty = client
        .post(format!("{providers}/vad_loaded/test/vad"))
        .bearer_auth("admin-test-token")
        .body("caller pcm is not accepted")
        .send()
        .await
        .unwrap();
    assert_eq!(nonempty.status(), StatusCode::BAD_REQUEST);
    assert_eq!(
        nonempty.json::<serde_json::Value>().await.unwrap()["error"]["code"],
        "invalid_test_input"
    );
    assert_eq!(
        client
            .post(&providers)
            .bearer_auth("admin-test-token")
            .json(&serde_json::json!({
                "key":"asr_other", "name":"Other ASR", "type":"asr", "adapter":"gipformer_sherpa_offline",
                "config_json":{"decoding_method":"greedy_search","max_active_paths":4}
            }))
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::CREATED
    );
    let wrong_type = client
        .post(format!("{providers}/asr_other/test/vad"))
        .bearer_auth("admin-test-token")
        .send()
        .await
        .unwrap();
    assert_eq!(wrong_type.status(), StatusCode::BAD_REQUEST);
    assert_eq!(
        wrong_type.json::<serde_json::Value>().await.unwrap()["error"]["code"],
        "provider_type_mismatch"
    );
    assert_eq!(
        client
            .post(format!("{providers}/missing/test/vad"))
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

    {
        let requests = requests.lock().unwrap();
        assert_eq!(requests.len(), 2);
        assert!(requests[0].tools.is_empty());
        assert_eq!(
            requests[0].messages,
            vec![ChatMessage::User {
                content: "xin chao".into()
            }]
        );
    }

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
async fn admin_p1_read_models_and_device_template_override_are_public_contracts() {
    let (base, task) = server(true).await;
    let client = Client::new();
    let auth = "admin-test-token";
    let agents = format!("{base}/api/admin/agents");
    let templates = format!("{base}/api/admin/templates");
    assert_eq!(
        client
            .post(&agents)
            .bearer_auth(auth)
            .json(&serde_json::json!({"key":"kitchen","name":"Kitchen"}))
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::CREATED
    );
    for (key, name, language) in [
        ("default", "Default assistant", "vi-VN"),
        ("kids", "Kids assistant", "en-US"),
    ] {
        assert_eq!(client.post(&templates).bearer_auth(auth).json(&serde_json::json!({"key":key,"name":name,"language":language,"prompt":"A bounded prompt"})).send().await.unwrap().status(), StatusCode::CREATED);
    }
    let assigned = client
        .put(format!("{base}/api/admin/agents/kitchen/templates/default"))
        .bearer_auth(auth)
        .header("if-match", "\"1\"")
        .send()
        .await
        .unwrap();
    assert_eq!(assigned.status(), StatusCode::OK);
    assert_eq!(
        client
            .put(format!("{base}/api/admin/agents/kitchen/templates/kids"))
            .bearer_auth(auth)
            .header("if-match", "\"2\"")
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::OK
    );

    let filtered: serde_json::Value = client
        .get(format!(
            "{templates}?q=Kids&language=en-US&page_size=1&sort=name"
        ))
        .bearer_auth(auth)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(filtered["total"], 1);
    assert_eq!(filtered["total_pages"], 1);
    assert_eq!(filtered["items"][0]["key"], "kids");
    assert_eq!(
        client
            .get(format!("{templates}?sort=unsafe"))
            .bearer_auth(auth)
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::BAD_REQUEST
    );

    let providers = format!("{base}/api/admin/providers");
    for (key, name, kind, adapter, config_json) in [
        (
            "vad_kids",
            "Kids VAD",
            "vad",
            "silero_onnx",
            serde_json::json!({}),
        ),
        (
            "llm_main",
            "Main LLM",
            "llm",
            "openai",
            serde_json::json!({"base_url":"https://example.test/v1","model":"test","max_tokens":8}),
        ),
    ] {
        assert_eq!(
            client
                .post(&providers)
                .bearer_auth(auth)
                .json(&serde_json::json!({"key":key,"name":name,"type":kind,"adapter":adapter,"config_json":config_json}))
                .send()
                .await
                .unwrap()
                .status(),
            StatusCode::CREATED
        );
    }
    let provider_page: serde_json::Value = client
        .get(format!("{providers}?type=vad&q=Kids&page_size=1&sort=name"))
        .bearer_auth(auth)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(provider_page["total"], 1);
    assert_eq!(provider_page["facets"]["vad"], 1);
    assert_eq!(provider_page["facets"]["llm"], 0);
    assert_eq!(provider_page["items"][0]["key"], "vad_kids");

    let device_url = format!("{base}/api/admin/devices");
    let device: serde_json::Value = client.post(&device_url).bearer_auth(auth).json(&serde_json::json!({"device_id":"kitchen-speaker","agent_key":"kitchen","template_key":"kids"})).send().await.unwrap().json().await.unwrap();
    assert_eq!(device["template_key"], "kids");
    let cleared: serde_json::Value = client
        .patch(format!("{device_url}/kitchen-speaker"))
        .bearer_auth(auth)
        .header("if-match", "\"1\"")
        .json(&serde_json::json!({"template_key":null}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(cleared["template_key"].is_null());
    assert_eq!(
        client
            .patch(format!("{device_url}/kitchen-speaker"))
            .bearer_auth(auth)
            .header("if-match", "\"2\"")
            .json(&serde_json::json!({"template_key":"default"}))
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::OK
    );
    assert_eq!(
        client
            .patch(format!("{device_url}/kitchen-speaker"))
            .bearer_auth(auth)
            .header("if-match", "\"3\"")
            .json(&serde_json::json!({"template_key":"missing"}))
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::BAD_REQUEST
    );

    let system: serde_json::Value = client
        .get(format!("{base}/api/admin/system"))
        .bearer_auth(auth)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(system["database"]["enabled"], true);
    assert!(system["uptime_seconds"].is_u64());
    task.abort();
}

#[tokio::test]
async fn admin_system_reports_database_unavailable_without_leaking_runtime_details() {
    let (base, task) = server_with_closed_database().await;
    let response = Client::new()
        .get(format!("{base}/api/admin/system"))
        .bearer_auth("admin-test-token")
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body: serde_json::Value = response.json().await.unwrap();
    assert_eq!(body["database"]["status"], "unavailable");
    assert!(body["providers"]["configured"].is_null());
    assert!(body["providers"]["loaded"].is_null());
    assert!(!body.to_string().contains("sqlite"));
    task.abort();
}

#[tokio::test]
async fn tts_provider_test_passes_typed_override_to_loaded_adapter_and_returns_provider_wav() {
    let (base, requests, task) = server_with_loaded_tts().await;
    let client = Client::new();
    let providers = format!("{base}/api/admin/providers");
    assert_eq!(
        client
            .post(&providers)
            .bearer_auth("admin-test-token")
            .json(&serde_json::json!({
                "key":"tts_loaded", "name":"Loaded TTS", "type":"tts", "adapter":"zerotts_onnx",
                "config_json":{"voice":"maichi","language":"vi-VN"}
            }))
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::CREATED
    );
    let response = client
        .post(format!("{providers}/tts_loaded/test/tts"))
        .bearer_auth("admin-test-token")
        .json(&serde_json::json!({"text":"xin chao","voice":"maichi","language":"vi-VN"}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()["content-type"], "audio/wav");
    assert_eq!(
        response.headers()["x-provider-runtime-matches-desired"],
        "true"
    );
    let wav = response.bytes().await.unwrap();
    assert_eq!(&wav[..4], b"RIFF");
    assert_eq!(&wav[8..12], b"WAVE");
    assert_eq!(u32::from_le_bytes(wav[24..28].try_into().unwrap()), 48_000);
    assert_eq!(
        requests.lock().unwrap().as_slice(),
        &[TtsDiagnosticRequest {
            text: "xin chao".into(),
            voice: Some("maichi".into()),
            language: Some("vi-VN".into()),
        }]
    );
    task.abort();
}

#[tokio::test]
async fn asr_provider_test_accepts_bounded_pcm_wav_and_rejects_other_media() {
    let (base, received_samples, task) = server_with_loaded_asr().await;
    let client = Client::new();
    let providers = format!("{base}/api/admin/providers");
    assert_eq!(client.post(&providers).bearer_auth("admin-test-token").json(&serde_json::json!({
        "key":"asr_loaded", "name":"Loaded ASR", "type":"asr", "adapter":"gipformer_sherpa_offline",
        "config_json":{"language":"vi-VN","decoding_method":"greedy_search","max_active_paths":4}
    })).send().await.unwrap().status(), StatusCode::CREATED);
    let url = format!("{providers}/asr_loaded/test/asr");

    let response = client
        .post(&url)
        .bearer_auth("admin-test-token")
        .header("content-type", "audio/wav")
        .body(wav_pcm16_mono(16_000, 16_000))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let response = response.json::<serde_json::Value>().await.unwrap();
    assert_eq!(response["provider_key"], "asr_loaded");
    assert_eq!(response["type"], "asr");
    assert_eq!(response["result"]["text"], "xin chao");
    assert_eq!(response["result"]["language"], "vi-VN");
    assert_eq!(response["metrics"]["audio_duration_ms"], 1_000);
    assert!(response["metrics"]["rtf"].is_number());
    assert_eq!(*received_samples.lock().unwrap(), vec![16_000]);

    for (content_type, body) in [
        ("audio/mpeg", wav_pcm16_mono(16_000, 1)),
        ("audio/wav", wav_pcm16_mono(8_000, 8_000)),
        ("audio/wav", wav_pcm16_mono(16_000, 16_000 * 31)),
    ] {
        let response = client
            .post(&url)
            .bearer_auth("admin-test-token")
            .header("content-type", content_type)
            .body(body)
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        assert_eq!(
            response.json::<serde_json::Value>().await.unwrap()["error"]["code"],
            "invalid_test_input"
        );
    }
    let response = client
        .post(&url)
        .bearer_auth("admin-test-token")
        .header("content-type", "audio/wav")
        .body(vec![0; 5 * 1024 * 1024 + 1])
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
    assert_eq!(
        response.json::<serde_json::Value>().await.unwrap()["error"]["code"],
        "request_too_large"
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
            },
            {
                "adapter": "kokoro_vi_onnx",
                "type": "tts",
                "display_name": "Kokoro Vietnamese ONNX"
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
    assert_eq!(descriptor["discovery"]["voices"], "static");
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
    assert_eq!(discovered["voices"].as_array().unwrap().len(), 8);
    assert!(
        discovered["voices"]
            .as_array()
            .unwrap()
            .iter()
            .any(|voice| voice["id"] == "maichi")
    );
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
                "voice": "maichi"
            }
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(legacy.status(), StatusCode::CREATED);
    let legacy = legacy.json::<serde_json::Value>().await.unwrap();
    assert_eq!(
        legacy["config_json"],
        "{\"voice\":\"maichi\",\"language\":\"vi-VN\",\"preload\":false,\"delivery_mode\":\"stream\"}"
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
                "voice": "maichi",
                "language": "en-US"
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
        ("vad_main", "vad", "silero_onnx", serde_json::json!({})),
        (
            "asr_main",
            "asr",
            "zipformer_sherpa",
            serde_json::json!({"decoding_method":"greedy_search"}),
        ),
        (
            "tts_main",
            "tts",
            "zerotts_onnx",
            serde_json::json!({"voice":"maichi"}),
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
async fn admin_relationship_reads_and_unlinks_are_revisioned_public_contracts() {
    let (base, task) = server(true).await;
    let client = Client::new();
    let auth = "admin-test-token";
    let agents = format!("{base}/api/admin/agents");
    let templates = format!("{base}/api/admin/templates");
    let providers = format!("{base}/api/admin/providers");

    assert_eq!(
        client
            .post(&agents)
            .bearer_auth(auth)
            .json(&serde_json::json!({"key":"kitchen","name":"Kitchen"}))
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::CREATED
    );
    assert_eq!(
        client
            .post(&templates)
            .bearer_auth(auth)
            .json(&serde_json::json!({
                "key":"quiet","name":"Quiet","language":"vi-VN","prompt":"Nói ngắn gọn"
            }))
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::CREATED
    );

    for (key, kind, adapter, config_json) in [
        ("vad_main", "vad", "silero_onnx", serde_json::json!({})),
        (
            "asr_main",
            "asr",
            "zipformer_sherpa",
            serde_json::json!({"decoding_method":"greedy_search"}),
        ),
        (
            "llm_main",
            "llm",
            "openai",
            serde_json::json!({"base_url":"https://example.test/v1","model":"test","max_tokens":8}),
        ),
        (
            "tts_main",
            "tts",
            "zerotts_onnx",
            serde_json::json!({"voice":"maichi"}),
        ),
    ] {
        assert_eq!(
            client
                .post(&providers)
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

    for (template_revision, (kind, provider_key)) in (1..).zip([
        ("vad", "vad_main"),
        ("asr", "asr_main"),
        ("llm", "llm_main"),
        ("tts", "tts_main"),
    ]) {
        let response = client
            .put(format!("{templates}/quiet/providers/{kind}"))
            .bearer_auth(auth)
            .header("if-match", format!("\"{template_revision}\""))
            .json(&serde_json::json!({"provider_key":provider_key}))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }

    let assigned = client
        .put(format!("{agents}/kitchen/templates/quiet"))
        .bearer_auth(auth)
        .header("if-match", "\"1\"")
        .send()
        .await
        .unwrap();
    assert_eq!(assigned.status(), StatusCode::OK);

    let agent_templates = client
        .get(format!("{agents}/kitchen/templates"))
        .bearer_auth(auth)
        .send()
        .await
        .unwrap()
        .json::<serde_json::Value>()
        .await
        .unwrap();
    assert_eq!(agent_templates["items"][0]["key"], "quiet");
    assert_eq!(agent_templates["items"][0]["is_default"], false);
    assert_eq!(agent_templates["revision"], 2);

    let template_agents = client
        .get(format!("{templates}/quiet/agents"))
        .bearer_auth(auth)
        .send()
        .await
        .unwrap()
        .json::<serde_json::Value>()
        .await
        .unwrap();
    assert_eq!(template_agents["items"][0]["key"], "kitchen");
    assert_eq!(template_agents["items"][0]["is_default"], false);
    assert_eq!(template_agents["revision"], 5);
    assert_eq!(template_agents["total"], 1);

    let bindings = client
        .get(format!("{templates}/quiet/providers"))
        .bearer_auth(auth)
        .send()
        .await
        .unwrap()
        .json::<serde_json::Value>()
        .await
        .unwrap();
    assert_eq!(bindings["template_key"], "quiet");
    assert_eq!(bindings["revision"], 5);
    assert_eq!(bindings["bindings"]["llm"]["provider_key"], "llm_main");

    let provider_templates = client
        .get(format!("{providers}/llm_main/templates"))
        .bearer_auth(auth)
        .send()
        .await
        .unwrap()
        .json::<serde_json::Value>()
        .await
        .unwrap();
    assert_eq!(provider_templates["items"][0]["key"], "quiet");
    assert_eq!(provider_templates["revision"], 1);
    assert_eq!(provider_templates["total"], 1);

    let missing_if_match = client
        .delete(format!("{agents}/kitchen/templates/quiet"))
        .bearer_auth(auth)
        .send()
        .await
        .unwrap();
    assert_eq!(missing_if_match.status(), StatusCode::BAD_REQUEST);

    let unlinked = client
        .delete(format!("{agents}/kitchen/templates/quiet"))
        .bearer_auth(auth)
        .header("if-match", "\"2\"")
        .send()
        .await
        .unwrap();
    assert_eq!(unlinked.status(), StatusCode::OK);
    assert_eq!(
        client
            .get(format!("{agents}/kitchen/templates"))
            .bearer_auth(auth)
            .send()
            .await
            .unwrap()
            .json::<serde_json::Value>()
            .await
            .unwrap()["items"]
            .as_array()
            .unwrap()
            .len(),
        0
    );

    let reassigned = client
        .put(format!("{agents}/kitchen/templates/quiet"))
        .bearer_auth(auth)
        .header("if-match", "\"3\"")
        .send()
        .await
        .unwrap();
    assert_eq!(reassigned.status(), StatusCode::OK);
    let stale_unlink = client
        .delete(format!("{agents}/kitchen/templates/quiet"))
        .bearer_auth(auth)
        .header("if-match", "\"3\"")
        .send()
        .await
        .unwrap();
    assert_eq!(stale_unlink.status(), StatusCode::CONFLICT);
    assert_eq!(
        stale_unlink.json::<serde_json::Value>().await.unwrap()["error"]["code"],
        "revision_conflict"
    );
    let defaulted = client
        .put(format!("{agents}/kitchen/default-template/quiet"))
        .bearer_auth(auth)
        .header("if-match", "\"4\"")
        .send()
        .await
        .unwrap();
    assert_eq!(defaulted.status(), StatusCode::OK);
    let default_conflict = client
        .delete(format!("{agents}/kitchen/templates/quiet"))
        .bearer_auth(auth)
        .header("if-match", "\"5\"")
        .send()
        .await
        .unwrap();
    assert_eq!(default_conflict.status(), StatusCode::CONFLICT);
    assert_eq!(
        default_conflict.json::<serde_json::Value>().await.unwrap()["error"]["code"],
        "default_template_conflict"
    );

    let missing_binding_if_match = client
        .delete(format!("{templates}/quiet/providers/llm"))
        .bearer_auth(auth)
        .send()
        .await
        .unwrap();
    assert_eq!(missing_binding_if_match.status(), StatusCode::BAD_REQUEST);

    let removed_binding = client
        .delete(format!("{templates}/quiet/providers/llm"))
        .bearer_auth(auth)
        .header("if-match", "\"5\"")
        .send()
        .await
        .unwrap();
    assert_eq!(removed_binding.status(), StatusCode::OK);
    let bindings = client
        .get(format!("{templates}/quiet/providers"))
        .bearer_auth(auth)
        .send()
        .await
        .unwrap()
        .json::<serde_json::Value>()
        .await
        .unwrap();
    assert!(bindings["bindings"].get("llm").is_none());
    let stale_binding_unlink = client
        .delete(format!("{templates}/quiet/providers/tts"))
        .bearer_auth(auth)
        .header("if-match", "\"5\"")
        .send()
        .await
        .unwrap();
    assert_eq!(stale_binding_unlink.status(), StatusCode::CONFLICT);
    assert_eq!(
        stale_binding_unlink
            .json::<serde_json::Value>()
            .await
            .unwrap()["error"]["code"],
        "revision_conflict"
    );
    let bindings = client
        .get(format!("{templates}/quiet/providers"))
        .bearer_auth(auth)
        .send()
        .await
        .unwrap()
        .json::<serde_json::Value>()
        .await
        .unwrap();
    assert_eq!(bindings["bindings"]["tts"]["provider_key"], "tts_main");

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

    let mcp_in_use = client
        .delete(format!("{base}/api/admin/mcp-servers/weather"))
        .bearer_auth(auth)
        .header("if-match", "\"1\"")
        .send()
        .await
        .unwrap();
    assert_eq!(mcp_in_use.status(), StatusCode::CONFLICT);
    assert_eq!(
        mcp_in_use.json::<serde_json::Value>().await.unwrap()["error"]["code"],
        "mcp_server_in_use"
    );
    let missing_unlink_match = client
        .delete(format!(
            "{base}/api/admin/agents/kitchen/mcp-bindings/weather"
        ))
        .bearer_auth(auth)
        .send()
        .await
        .unwrap();
    assert_eq!(missing_unlink_match.status(), StatusCode::BAD_REQUEST);
    assert_eq!(
        client
            .delete(format!(
                "{base}/api/admin/agents/kitchen/mcp-bindings/weather"
            ))
            .bearer_auth(auth)
            .header("if-match", "\"2\"")
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        client
            .delete(format!("{base}/api/admin/mcp-servers/weather"))
            .bearer_auth(auth)
            .header("if-match", "\"1\"")
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::NO_CONTENT
    );
    task.abort();
}

#[tokio::test]
async fn conditional_delete_requires_a_current_revision_and_explicit_unlink() {
    let (base, task) = server(true).await;
    let client = Client::new();
    let auth = "admin-test-token";
    let agents = format!("{base}/api/admin/agents");
    let templates = format!("{base}/api/admin/templates");
    let providers = format!("{base}/api/admin/providers");

    for (url, body) in [
        (
            agents.as_str(),
            serde_json::json!({"key":"kitchen","name":"Kitchen"}),
        ),
        (
            templates.as_str(),
            serde_json::json!({"key":"quiet","name":"Quiet","language":"vi-VN","prompt":"Be concise"}),
        ),
        (
            providers.as_str(),
            serde_json::json!({"key":"llm_main","name":"LLM","type":"llm","adapter":"openai","config_json":{"base_url":"https://example.test/v1","model":"test","max_tokens":8}}),
        ),
    ] {
        assert_eq!(
            client
                .post(url)
                .bearer_auth(auth)
                .json(&body)
                .send()
                .await
                .unwrap()
                .status(),
            StatusCode::CREATED
        );
    }

    let missing_match = client
        .delete(format!("{agents}/kitchen"))
        .bearer_auth(auth)
        .send()
        .await
        .unwrap();
    assert_eq!(missing_match.status(), StatusCode::BAD_REQUEST);
    let stale = client
        .delete(format!("{agents}/kitchen"))
        .bearer_auth(auth)
        .header("if-match", "\"2\"")
        .send()
        .await
        .unwrap();
    assert_eq!(stale.status(), StatusCode::CONFLICT);
    assert_eq!(
        stale.json::<serde_json::Value>().await.unwrap()["error"]["code"],
        "revision_conflict"
    );

    assert_eq!(
        client
            .put(format!("{templates}/quiet/providers/llm"))
            .bearer_auth(auth)
            .header("if-match", "\"1\"")
            .json(&serde_json::json!({"provider_key":"llm_main"}))
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::OK
    );
    let provider_in_use = client
        .delete(format!("{providers}/llm_main"))
        .bearer_auth(auth)
        .header("if-match", "\"1\"")
        .send()
        .await
        .unwrap();
    assert_eq!(provider_in_use.status(), StatusCode::CONFLICT);
    assert_eq!(
        provider_in_use.json::<serde_json::Value>().await.unwrap()["error"]["code"],
        "provider_in_use"
    );
    assert_eq!(
        client
            .delete(format!("{templates}/quiet/providers/llm"))
            .bearer_auth(auth)
            .header("if-match", "\"2\"")
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::OK
    );
    assert_eq!(
        client
            .delete(format!("{providers}/llm_main"))
            .bearer_auth(auth)
            .header("if-match", "\"1\"")
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::NO_CONTENT
    );

    assert_eq!(
        client
            .put(format!("{agents}/kitchen/templates/quiet"))
            .bearer_auth(auth)
            .header("if-match", "\"1\"")
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::OK
    );
    let template_in_use = client
        .delete(format!("{templates}/quiet"))
        .bearer_auth(auth)
        .header("if-match", "\"3\"")
        .send()
        .await
        .unwrap();
    assert_eq!(template_in_use.status(), StatusCode::CONFLICT);
    assert_eq!(
        template_in_use.json::<serde_json::Value>().await.unwrap()["error"]["code"],
        "template_in_use"
    );
    assert_eq!(
        client
            .delete(format!("{agents}/kitchen/templates/quiet"))
            .bearer_auth(auth)
            .header("if-match", "\"2\"")
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::OK
    );
    // P0 unlink retains an inert assignment row, but it is no longer an active relationship.
    assert_eq!(
        client
            .delete(format!("{templates}/quiet"))
            .bearer_auth(auth)
            .header("if-match", "\"3\"")
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::NO_CONTENT
    );
    let devices = format!("{base}/api/admin/devices");
    assert_eq!(
        client
            .post(&devices)
            .bearer_auth(auth)
            .json(&serde_json::json!({"device_id":"kitchen-speaker","agent_key":"kitchen"}))
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::CREATED
    );
    let agent_in_use = client
        .delete(format!("{agents}/kitchen"))
        .bearer_auth(auth)
        .header("if-match", "\"3\"")
        .send()
        .await
        .unwrap();
    assert_eq!(agent_in_use.status(), StatusCode::CONFLICT);
    assert_eq!(
        agent_in_use.json::<serde_json::Value>().await.unwrap()["error"]["code"],
        "agent_in_use"
    );
    assert_eq!(
        client
            .delete(format!("{devices}/kitchen-speaker"))
            .bearer_auth(auth)
            .header("if-match", "\"1\"")
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        client
            .delete(format!("{agents}/kitchen"))
            .bearer_auth(auth)
            .header("if-match", "\"3\"")
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::NO_CONTENT
    );
    task.abort();
}

#[tokio::test]
async fn kokoro_desired_configuration_accepts_catalog_selection_and_rejects_factory_incompatible_voice()
 {
    let (base, task) = server(true).await;
    let client = Client::new();
    for (key, voice, status) in [
        ("kokoro_valid", "diem_trinh", StatusCode::CREATED),
        ("kokoro_invalid", "unknown", StatusCode::BAD_REQUEST),
    ] {
        let response = client.post(format!("{base}/api/admin/providers"))
            .bearer_auth("admin-test-token")
            .json(&serde_json::json!({"key":key,"name":key,"type":"tts","adapter":"kokoro_vi_onnx","config_json":{"voice":voice,"language":"vi-VN","speed_percent":100}}))
            .send().await.unwrap();
        assert_eq!(response.status(), status);
    }
    task.abort();
}

#[tokio::test]
async fn provider_mutations_reject_server_owned_configuration() {
    let (base, task) = server(true).await;
    let client = Client::new();
    for (index, (kind, adapter, valid, overrides)) in [
        (
            "vad",
            "silero_onnx",
            serde_json::json!({}),
            vec![
                ("num_threads", serde_json::json!(128)),
                ("model", serde_json::json!("silero_vad_v5")),
            ],
        ),
        (
            "asr",
            "zipformer_sherpa",
            serde_json::json!({"decoding_method":"greedy_search"}),
            vec![
                ("num_threads", serde_json::json!(128)),
                ("model", serde_json::json!("zipformer_vi_streaming")),
                ("decoding_method", serde_json::json!("arbitrary")),
            ],
        ),
        (
            "asr",
            "gipformer_sherpa_offline",
            serde_json::json!({}),
            vec![
                ("num_threads", serde_json::json!(128)),
                ("model", serde_json::json!("gipformer15_vi_int8")),
                ("decoding_method", serde_json::json!("arbitrary")),
                ("max_active_paths", serde_json::json!(10001)),
            ],
        ),
        (
            "tts",
            "zerotts_onnx",
            serde_json::json!({"voice":"maichi"}),
            vec![
                ("num_threads", serde_json::json!(128)),
                ("model", serde_json::json!("zerotts_default")),
                ("voice", serde_json::json!("unknown")),
            ],
        ),
        (
            "tts",
            "kokoro_vi_onnx",
            serde_json::json!({"voice":"diem_trinh"}),
            vec![
                ("num_threads", serde_json::json!(128)),
                ("model", serde_json::json!("kokoro_vi_contextbox")),
            ],
        ),
        (
            "tts",
            "chillaudio_ws",
            serde_json::json!({"voice":"BV421_vivn_streaming"}),
            vec![
                ("ws_url", serde_json::json!("wss://attacker.example/socket")),
                ("timeout_ms", serde_json::json!(1)),
                ("voice", serde_json::json!("unknown")),
            ],
        ),
    ]
    .into_iter()
    .enumerate()
    {
        let key = format!("protected_{index}");
        let response = client.post(format!("{base}/api/admin/providers"))
            .bearer_auth("admin-test-token")
            .json(&serde_json::json!({"key":key,"name":"Valid","type":kind,"adapter":adapter,"config_json":valid}))
            .send().await.unwrap();
        assert_eq!(response.status(), StatusCode::CREATED, "{adapter}");
        for (field, value) in overrides {
            let mut invalid = valid.clone();
            invalid[field] = value;
            for update in [false, true] {
                let request = if update {
                    client
                        .patch(format!("{base}/api/admin/providers/{key}"))
                        .header("if-match", "\"1\"")
                        .json(&serde_json::json!({"config_json":invalid}))
                } else {
                    client.post(format!("{base}/api/admin/providers"))
                        .json(&serde_json::json!({"key":"invalid_override","name":"Invalid","type":kind,"adapter":adapter,"config_json":invalid}))
                };
                let response = request
                    .bearer_auth("admin-test-token")
                    .send()
                    .await
                    .unwrap();
                assert_eq!(
                    response.status(),
                    StatusCode::BAD_REQUEST,
                    "{adapter}.{field}, update={update}"
                );
                assert_eq!(
                    response.json::<serde_json::Value>().await.unwrap()["error"]["code"],
                    "provider_config_invalid"
                );
            }
        }
        let row = client
            .get(format!("{base}/api/admin/providers/{key}"))
            .bearer_auth("admin-test-token")
            .send()
            .await
            .unwrap()
            .json::<serde_json::Value>()
            .await
            .unwrap();
        assert_eq!(row["revision"], 1);
    }
    task.abort();
}

#[tokio::test]
async fn legacy_provider_runtime_fields_are_removed_by_migration_before_admin_reads() {
    use voice_agent_server::config::DatabaseConfig;
    let database_uri = database_url();
    let config = DatabaseConfig {
        url: database_uri.clone(),
        ..Default::default()
    };
    let database = Database::connect(&config).await.unwrap();
    let cases = [
        (
            "legacy_vad",
            "vad",
            "silero_onnx",
            serde_json::json!({"model":"silero_vad_v5","num_threads":128}),
            serde_json::json!({}),
        ),
        (
            "legacy_asr",
            "asr",
            "zipformer_sherpa",
            serde_json::json!({"model":"zipformer_vi_streaming","num_threads":128,"decoding_method":"greedy_search"}),
            serde_json::json!({"decoding_method":"greedy_search"}),
        ),
        (
            "legacy_gip",
            "asr",
            "gipformer_sherpa_offline",
            serde_json::json!({"model":"gipformer15_vi_int8","num_threads":128}),
            serde_json::json!({}),
        ),
        (
            "legacy_zero",
            "tts",
            "zerotts_onnx",
            serde_json::json!({"model":"zerotts_default","num_threads":128,"voice":"maichi"}),
            serde_json::json!({"voice":"maichi"}),
        ),
        (
            "legacy_kokoro",
            "tts",
            "kokoro_vi_onnx",
            serde_json::json!({"model":"kokoro_vi_contextbox","num_threads":128,"voice":"duc_an"}),
            serde_json::json!({"voice":"duc_an"}),
        ),
        (
            "legacy_chill",
            "tts",
            "chillaudio_ws",
            serde_json::json!({"ws_url":"wss://attacker.example/ws","timeout_ms":1,"voice":"BV421_vivn_streaming"}),
            serde_json::json!({"voice":"BV421_vivn_streaming"}),
        ),
    ];
    for (key, kind, adapter, legacy, _) in &cases {
        sqlx::query("INSERT INTO providers (key,name,type,adapter,config_json,revision,created_at,updated_at) VALUES (?,?,?,?,?,1,1,1)")
            .bind(*key).bind(*key).bind(*kind).bind(*adapter).bind(legacy.to_string())
            .execute(database.pool()).await.unwrap();
    }
    // Fixture represents the previous version: migration 0006 changes data only.
    sqlx::query("DELETE FROM _sqlx_migrations WHERE version = 6")
        .execute(database.pool())
        .await
        .unwrap();
    database.pool().close().await;
    let (base, task) = server_with_database(true, &database_uri).await;
    let client = Client::new();
    for (key, _, _, _, expected) in &cases {
        let response = client
            .get(format!("{base}/api/admin/providers/{key}"))
            .bearer_auth("admin-test-token")
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let row = response.json::<serde_json::Value>().await.unwrap();
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(row["config_json"].as_str().unwrap())
                .unwrap(),
            *expected
        );
        assert_eq!(row["revision"], 2);
    }
    task.abort();
}
