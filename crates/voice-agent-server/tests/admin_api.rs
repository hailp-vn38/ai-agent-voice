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

/// Creates a Provider through the public Admin API and returns the key the server generated.
async fn create_provider(client: &Client, providers: &str, body: serde_json::Value) -> String {
    let response = client
        .post(providers)
        .bearer_auth("admin-test-token")
        .json(&body)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CREATED, "{body}");
    response.json::<serde_json::Value>().await.unwrap()["key"]
        .as_str()
        .expect("a created provider carries its generated key")
        .to_owned()
}

/// Creates a Provider on a throwaway Admin server and returns the database it committed to plus
/// the generated key. A loaded-runtime fixture can only publish its runtime under a key chosen
/// before its router exists, so the provider is created first and the database is carried over
/// to the server that owns the runtime.
async fn seeded_provider(body: serde_json::Value) -> (String, String) {
    let database_uri = database_url();
    let (base, task) = server_with_database(true, &database_uri).await;
    let key = create_provider(&Client::new(), &format!("{base}/api/admin/providers"), body).await;
    // The server that owns the runtime opens this same SQLite path next, so the seeding process
    // has to be gone rather than merely told to stop.
    task.abort();
    let _ = task.await;
    (database_uri, key)
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

async fn server_with_loaded_llm(
    database_uri: &str,
    instance_id: &str,
) -> (
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
            database_uri,
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
        instance_id,
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

async fn server_with_loaded_tts(
    database_uri: &str,
    instance_id: &str,
) -> (
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
            database_uri
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
        instance_id,
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

async fn server_with_loaded_asr(
    database_uri: &str,
    instance_id: &str,
) -> (String, Arc<Mutex<Vec<usize>>>, tokio::task::JoinHandle<()>) {
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
            database_uri
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
        instance_id,
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

async fn server_with_loaded_vad(
    database_uri: &str,
    instance_id: &str,
) -> (
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
            database_uri
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
        instance_id,
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
    let (database_uri, key) = seeded_provider(serde_json::json!({
        "name":"Loaded VAD", "type":"vad", "adapter":"silero_onnx",
        "config_json":{}
    }))
    .await;
    let (base, inputs, task) = server_with_loaded_vad(&database_uri, &key).await;
    let client = Client::new();
    let providers = format!("{base}/api/admin/providers");
    let response = client
        .post(format!("{providers}/{key}/test/vad"))
        .bearer_auth("admin-test-token")
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let response = response.json::<serde_json::Value>().await.unwrap();
    assert_eq!(response["provider_key"], key);
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
        .post(format!("{providers}/{key}/test/vad"))
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
    let other = create_provider(
        &client,
        &providers,
        serde_json::json!({
            "name":"Other ASR", "type":"asr", "adapter":"gipformer_sherpa_offline",
            "config_json":{"decoding_method":"greedy_search","max_active_paths":4}
        }),
    )
    .await;
    let wrong_type = client
        .post(format!("{providers}/{other}/test/vad"))
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
    let (database_uri, key) = seeded_provider(serde_json::json!({
        "name": "Loaded LLM", "type": "llm", "adapter": "openai",
        "config_json": {"base_url":"https://example.test/v1","model":"test","max_tokens":8}
    }))
    .await;
    let (base, requests, task) = server_with_loaded_llm(&database_uri, &key).await;
    let client = Client::new();
    let providers = format!("{base}/api/admin/providers");

    let response = client
        .post(format!("{providers}/{key}/test/llm"))
        .bearer_auth("admin-test-token")
        .json(&serde_json::json!({"input":"xin chao"}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let response = response.json::<serde_json::Value>().await.unwrap();
    assert_eq!(response["provider_key"], key);
    assert_eq!(response["type"], "llm");
    assert_eq!(response["status"], "success");
    assert_eq!(response["result"]["text"], "diagnostic answer");
    assert_eq!(response["runtime"]["runtime_status"], "loaded");
    assert_eq!(response["runtime"]["tested_runtime"], "loaded");
    assert_eq!(response["runtime"]["runtime_matches_desired"], true);
    assert_eq!(response["runtime"]["requires_restart"], false);
    assert!(response["metrics"]["elapsed_ms"].is_u64());

    let changed = client
        .patch(format!("{providers}/{key}"))
        .bearer_auth("admin-test-token")
        .header("if-match", "\"1\"")
        .json(&serde_json::json!({"name":"Changed desired state"}))
        .send()
        .await
        .unwrap();
    assert_eq!(changed.status(), StatusCode::OK);
    let stale = client
        .post(format!("{providers}/{key}/test/llm"))
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
        .post(format!("{providers}/{key}/test/llm"))
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
    let mut kids_vad = String::new();
    for (name, kind, adapter, config_json) in [
        ("Kids VAD", "vad", "silero_onnx", serde_json::json!({})),
        (
            "Main LLM",
            "llm",
            "openai",
            serde_json::json!({"base_url":"https://example.test/v1","model":"test","max_tokens":8}),
        ),
    ] {
        let key = create_provider(
            &client,
            &providers,
            serde_json::json!({"name":name,"type":kind,"adapter":adapter,"config_json":config_json}),
        )
        .await;
        if kind == "vad" {
            kids_vad = key;
        }
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
    assert_eq!(provider_page["items"][0]["key"], kids_vad);

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
    let (database_uri, key) = seeded_provider(serde_json::json!({
        "name":"Loaded TTS", "type":"tts", "adapter":"zerotts_onnx",
        "config_json":{"voice":"maichi","language":"vi-VN"}
    }))
    .await;
    let (base, requests, task) = server_with_loaded_tts(&database_uri, &key).await;
    let client = Client::new();
    let providers = format!("{base}/api/admin/providers");
    let response = client
        .post(format!("{providers}/{key}/test/tts"))
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
    let (database_uri, key) = seeded_provider(serde_json::json!({
        "name":"Loaded ASR", "type":"asr", "adapter":"gipformer_sherpa_offline",
        "config_json":{"language":"vi-VN","decoding_method":"greedy_search","max_active_paths":4}
    }))
    .await;
    let (base, received_samples, task) = server_with_loaded_asr(&database_uri, &key).await;
    let client = Client::new();
    let providers = format!("{base}/api/admin/providers");
    let url = format!("{providers}/{key}/test/asr");

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
    assert_eq!(response["provider_key"], key);
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
            "name":"Bad", "type":"llm", "adapter":"openai",
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
            "name":"LLM", "type":"llm", "adapter":"openai",
            "config_json":{"base_url":"https://example.test/v1","model":"test","max_tokens":8},
            "secret_ref":"LLM_SECRET"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(llm.status(), StatusCode::CREATED);
    let llm = llm.json::<serde_json::Value>().await.unwrap();
    let llm_key = llm["key"].as_str().expect("a key is generated").to_owned();
    assert_eq!(llm["has_secret_ref"], true);
    assert!(llm.get("secret_ref").is_none());
    assert_eq!(llm["runtime_status"], "not_loaded");
    assert_eq!(llm["runtime_matches_desired"], false);
    assert_eq!(llm["requires_restart"], true);

    let mut generated = vec![("llm".to_owned(), llm_key)];
    for (name, kind, adapter, config_json) in [
        ("Main VAD", "vad", "silero_onnx", serde_json::json!({})),
        (
            "Main ASR",
            "asr",
            "zipformer_sherpa",
            serde_json::json!({"decoding_method":"greedy_search"}),
        ),
        (
            "Main TTS",
            "tts",
            "zerotts_onnx",
            serde_json::json!({"voice":"maichi"}),
        ),
    ] {
        let key = create_provider(
            &client,
            &provider_url,
            serde_json::json!({
                "name":name,"type":kind,"adapter":adapter,"config_json":config_json
            }),
        )
        .await;
        generated.push((kind.to_owned(), key));
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

    for (kind, provider_key) in &generated {
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

    let mut generated = Vec::new();
    for (name, kind, adapter, config_json) in [
        ("Main VAD", "vad", "silero_onnx", serde_json::json!({})),
        (
            "Main ASR",
            "asr",
            "zipformer_sherpa",
            serde_json::json!({"decoding_method":"greedy_search"}),
        ),
        (
            "Main LLM",
            "llm",
            "openai",
            serde_json::json!({"base_url":"https://example.test/v1","model":"test","max_tokens":8}),
        ),
        (
            "Main TTS",
            "tts",
            "zerotts_onnx",
            serde_json::json!({"voice":"maichi"}),
        ),
    ] {
        let key = create_provider(
            &client,
            &providers,
            serde_json::json!({
                "name":name,"type":kind,"adapter":adapter,"config_json":config_json
            }),
        )
        .await;
        assert!(key.starts_with(kind));
        generated.push((kind, key));
    }

    for (template_revision, (kind, provider_key)) in (1..).zip(&generated) {
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
    assert_eq!(agent_templates["items"][0]["is_default"], true);
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
    assert_eq!(template_agents["items"][0]["is_default"], true);
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
    let llm_key = &generated[2].1;
    assert_eq!(bindings["bindings"]["llm"]["provider_key"], *llm_key);

    let provider_templates = client
        .get(format!("{providers}/{llm_key}/templates"))
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

    // Agents may have no Templates. Unlinking their sole default clears the assignment and lets
    // subsequent sessions resolve through the server defaults.
    let unlinked_default = client
        .delete(format!("{agents}/kitchen/templates/quiet"))
        .bearer_auth(auth)
        .header("if-match", "\"2\"")
        .send()
        .await
        .unwrap();
    assert_eq!(unlinked_default.status(), StatusCode::OK);
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
    assert_eq!(
        client
            .put(format!("{agents}/kitchen/templates/quiet"))
            .bearer_auth(auth)
            .header("if-match", "\"3\"")
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::OK
    );
    assert_eq!(
        client
            .post(&templates)
            .bearer_auth(auth)
            .json(&serde_json::json!({
                "key":"loud","name":"Loud","language":"vi-VN","prompt":"Nói to"
            }))
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::CREATED
    );
    // A default Template may inherit missing provider slots from server defaults.
    let default_loud = client
        .put(format!("{agents}/kitchen/default-template/loud"))
        .bearer_auth(auth)
        .header("if-match", "\"4\"")
        .send()
        .await
        .unwrap();
    assert_eq!(
        default_loud.status(),
        StatusCode::OK,
        "promoting a partial template failed: {:?}",
        default_loud.text().await
    );
    for (revision, (kind, provider_key)) in (1..).zip(&generated) {
        assert_eq!(
            client
                .put(format!("{templates}/loud/providers/{kind}"))
                .bearer_auth(auth)
                .header("if-match", format!("\"{revision}\""))
                .json(&serde_json::json!({"provider_key":provider_key}))
                .send()
                .await
                .unwrap()
                .status(),
            StatusCode::OK
        );
    }
    let unlinked = client
        .delete(format!("{agents}/kitchen/templates/quiet"))
        .bearer_auth(auth)
        .header("if-match", "\"5\"")
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
        1
    );

    let reassigned = client
        .put(format!("{agents}/kitchen/templates/quiet"))
        .bearer_auth(auth)
        .header("if-match", "\"6\"")
        .send()
        .await
        .unwrap();
    assert_eq!(reassigned.status(), StatusCode::OK);
    let stale_unlink = client
        .delete(format!("{agents}/kitchen/templates/quiet"))
        .bearer_auth(auth)
        .header("if-match", "\"6\"")
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
        .header("if-match", "\"7\"")
        .send()
        .await
        .unwrap();
    assert_eq!(defaulted.status(), StatusCode::OK);
    let unlinked_default = client
        .delete(format!("{agents}/kitchen/templates/quiet"))
        .bearer_auth(auth)
        .header("if-match", "\"8\"")
        .send()
        .await
        .unwrap();
    assert_eq!(unlinked_default.status(), StatusCode::OK);

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
    assert_eq!(bindings["bindings"]["tts"]["provider_key"], generated[3].1);

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
async fn conditional_delete_requires_a_current_revision_and_preserves_linked_templates() {
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
    let llm_key = create_provider(
        &client,
        &providers,
        serde_json::json!({"name":"LLM","type":"llm","adapter":"openai","config_json":{"base_url":"https://example.test/v1","model":"test","max_tokens":8}}),
    )
    .await;

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
            .json(&serde_json::json!({"provider_key":llm_key}))
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::OK
    );
    let provider_in_use = client
        .delete(format!("{providers}/{llm_key}"))
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
            .delete(format!("{providers}/{llm_key}"))
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
    // A Template is globally reusable. Deleting an Agent only removes that Agent's assignment;
    // it must not delete or otherwise mutate the Template itself.
    assert_eq!(
        client
            .delete(format!("{agents}/kitchen"))
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
            .get(format!("{templates}/quiet"))
            .bearer_auth(auth)
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::OK
    );
    let template_agents = client
        .get(format!("{templates}/quiet/agents"))
        .bearer_auth(auth)
        .send()
        .await
        .unwrap();
    assert_eq!(template_agents.status(), StatusCode::OK);
    assert_eq!(
        template_agents.json::<serde_json::Value>().await.unwrap()["items"],
        serde_json::json!([])
    );
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
        .header("if-match", "\"1\"")
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
async fn kokoro_desired_configuration_accepts_catalog_selection_and_rejects_factory_incompatible_voice()
 {
    let (base, task) = server(true).await;
    let client = Client::new();
    for (voice, status) in [
        ("diem_trinh", StatusCode::CREATED),
        ("unknown", StatusCode::BAD_REQUEST),
    ] {
        let response = client.post(format!("{base}/api/admin/providers"))
            .bearer_auth("admin-test-token")
            .json(&serde_json::json!({"name":"Kokoro","type":"tts","adapter":"kokoro_vi_onnx","config_json":{"voice":voice,"language":"vi-VN","speed_percent":100}}))
            .send().await.unwrap();
        assert_eq!(response.status(), status);
    }
    task.abort();
}

#[tokio::test]
async fn provider_mutations_reject_server_owned_configuration() {
    let (base, task) = server(true).await;
    let client = Client::new();
    for (kind, adapter, valid, overrides) in [
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
    {
        let providers = format!("{base}/api/admin/providers");
        let key = create_provider(
            &client,
            &providers,
            serde_json::json!({"name":"Valid","type":kind,"adapter":adapter,"config_json":valid}),
        )
        .await;
        for (field, value) in overrides {
            let mut invalid = valid.clone();
            invalid[field] = value;
            for update in [false, true] {
                let request = if update {
                    client
                        .patch(format!("{providers}/{key}"))
                        .header("if-match", "\"1\"")
                        .json(&serde_json::json!({"config_json":invalid}))
                } else {
                    client.post(&providers)
                        .json(&serde_json::json!({"name":"Invalid","type":kind,"adapter":adapter,"config_json":invalid}))
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
            .get(format!("{providers}/{key}"))
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
async fn provider_key_is_generated_by_the_server_and_never_accepted_from_a_client() {
    let (base, task) = server(true).await;
    let client = Client::new();
    let providers = format!("{base}/api/admin/providers");
    let body = serde_json::json!({"name":"OpenAI LLM","type":"llm","adapter":"openai","config_json":{"base_url":"https://example.test/v1","model":"test","max_tokens":8}});
    let key = create_provider(&client, &providers, body.clone()).await;
    assert!(key.starts_with("llm_"), "{key}");
    assert!(key.len() <= 64, "{key}");
    assert!(
        key.bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_'),
        "{key}"
    );
    let identity = key.strip_prefix("llm_").expect("the type prefixes the key");
    assert_eq!(identity.len(), 32, "{key}");
    assert!(
        identity
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)),
        "{key}"
    );

    let twin = create_provider(&client, &providers, body).await;
    assert_ne!(twin, key, "identity never comes from the display name");

    let chosen = client
        .post(&providers)
        .bearer_auth("admin-test-token")
        .json(&serde_json::json!({"key":"user_controlled_key","name":"OpenAI LLM","type":"llm","adapter":"openai","config_json":{}}))
        .send()
        .await
        .unwrap();
    assert_eq!(chosen.status(), StatusCode::BAD_REQUEST);
    assert_eq!(
        chosen.json::<serde_json::Value>().await.unwrap()["error"]["code"],
        "invalid_json"
    );

    let fetched = client
        .get(format!("{providers}/{key}"))
        .bearer_auth("admin-test-token")
        .send()
        .await
        .unwrap();
    assert_eq!(fetched.status(), StatusCode::OK);
    assert_eq!(
        fetched.json::<serde_json::Value>().await.unwrap()["key"],
        key
    );

    let renamed = client
        .patch(format!("{providers}/{key}"))
        .bearer_auth("admin-test-token")
        .header("if-match", "\"1\"")
        .json(&serde_json::json!({"name":"Renamed"}))
        .send()
        .await
        .unwrap();
    assert_eq!(renamed.status(), StatusCode::OK);
    // A display name is the user's to change; identity is not.
    assert_eq!(
        renamed.json::<serde_json::Value>().await.unwrap()["key"],
        key
    );

    let immovable = client
        .patch(format!("{providers}/{key}"))
        .bearer_auth("admin-test-token")
        .header("if-match", "\"2\"")
        .json(&serde_json::json!({"key":"new_key"}))
        .send()
        .await
        .unwrap();
    assert_eq!(immovable.status(), StatusCode::BAD_REQUEST);
    assert_eq!(
        immovable.json::<serde_json::Value>().await.unwrap()["error"]["code"],
        "immutable_field"
    );
    task.abort();
}

#[tokio::test]
async fn a_generated_provider_key_binds_a_template_without_being_retyped() {
    use voice_agent_server::config::DatabaseConfig;
    let database_uri = database_url();
    let (base, task) = server_with_database(true, &database_uri).await;
    let client = Client::new();
    let templates = format!("{base}/api/admin/templates");
    let providers = format!("{base}/api/admin/providers");
    assert_eq!(
        client
            .post(&templates)
            .bearer_auth("admin-test-token")
            .json(&serde_json::json!({"key":"quiet","name":"Quiet","language":"vi-VN","prompt":"Be concise"}))
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::CREATED
    );
    let key = create_provider(
        &client,
        &providers,
        serde_json::json!({"name":"Main LLM","type":"llm","adapter":"openai","config_json":{"base_url":"https://example.test/v1","model":"test","max_tokens":8}}),
    )
    .await;
    assert_eq!(
        client
            .put(format!("{templates}/quiet/providers/llm"))
            .bearer_auth("admin-test-token")
            .header("if-match", "\"1\"")
            .json(&serde_json::json!({"provider_key":key}))
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::OK
    );
    let bindings: serde_json::Value = client
        .get(format!("{templates}/quiet/providers"))
        .bearer_auth("admin-test-token")
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(bindings["bindings"]["llm"]["provider_key"], key);
    let using: serde_json::Value = client
        .get(format!("{providers}/{key}/templates"))
        .bearer_auth("admin-test-token")
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(using["items"][0]["key"], "quiet");

    // An echoed key would still pass above if the row resolved to a different provider, so the
    // binding is checked against the row the generated key actually names.
    let database = Database::connect(&DatabaseConfig {
        url: database_uri,
        ..Default::default()
    })
    .await
    .unwrap();
    let provider_id: i64 = sqlx::query_scalar("SELECT id FROM providers WHERE key = ?")
        .bind(&key)
        .fetch_one(database.pool())
        .await
        .unwrap();
    let bound_id: i64 = sqlx::query_scalar(
        "SELECT b.provider_id FROM template_provider_bindings b \
         JOIN agent_templates t ON t.id = b.template_id WHERE t.key = ? AND b.provider_type = ?",
    )
    .bind("quiet")
    .bind("llm")
    .fetch_one(database.pool())
    .await
    .unwrap();
    assert_eq!(bound_id, provider_id);
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

/// An Agent inside the Template mechanism has no path back to server defaults: admission requires
/// exactly one enabled default assignment and refuses the connection otherwise. `assign_template`
/// used to insert `is_default = 0` unconditionally, so the very first assignment to an Agent left
/// it in that state and every one of its devices got a bare 503.
#[tokio::test]
async fn first_assignment_to_an_agent_becomes_its_enabled_default() {
    let (base, task) = server(true).await;
    let client = Client::new();
    let auth = "admin-test-token";
    let agents = format!("{base}/api/admin/agents");
    let templates = format!("{base}/api/admin/templates");
    let providers = format!("{base}/api/admin/providers");

    for (path, body) in [
        (
            &agents,
            serde_json::json!({"key":"kitchen","name":"Kitchen"}),
        ),
        (
            &templates,
            serde_json::json!({"key":"quiet","name":"Quiet","language":"vi-VN","prompt":"Nói ngắn gọn"}),
        ),
        (
            &templates,
            serde_json::json!({"key":"loud","name":"Loud","language":"vi-VN","prompt":"Nói to"}),
        ),
    ] {
        assert_eq!(
            client
                .post(path)
                .bearer_auth(auth)
                .json(&body)
                .send()
                .await
                .unwrap()
                .status(),
            StatusCode::CREATED
        );
    }

    // A first assignment takes the default slot unless an explicit core binding is broken;
    // absent core slots fall back to the server-default providers.
    let mut generated = Vec::new();
    for (name, kind, adapter, config_json) in [
        ("Main VAD", "vad", "silero_onnx", serde_json::json!({})),
        (
            "Main ASR",
            "asr",
            "zipformer_sherpa",
            serde_json::json!({"decoding_method":"greedy_search"}),
        ),
        (
            "Main LLM",
            "llm",
            "openai",
            serde_json::json!({"base_url":"https://example.test/v1","model":"test","max_tokens":8}),
        ),
        (
            "Main TTS",
            "tts",
            "zerotts_onnx",
            serde_json::json!({"voice":"maichi"}),
        ),
    ] {
        let key = create_provider(
            &client,
            &providers,
            serde_json::json!({
                "name":name,"type":kind,"adapter":adapter,"config_json":config_json
            }),
        )
        .await;
        generated.push((kind, key));
    }

    for template_key in ["quiet", "loud"] {
        for (revision, (kind, provider_key)) in (1..).zip(&generated) {
            assert_eq!(
                client
                    .put(format!("{templates}/{template_key}/providers/{kind}"))
                    .bearer_auth(auth)
                    .header("if-match", format!("\"{revision}\""))
                    .json(&serde_json::json!({"provider_key":provider_key}))
                    .send()
                    .await
                    .unwrap()
                    .status(),
                StatusCode::OK
            );
        }
    }

    let assign = |template_key: &'static str, if_match: &'static str| {
        let client = client.clone();
        let url = format!("{agents}/kitchen/templates/{template_key}");
        async move {
            client
                .put(url)
                .bearer_auth(auth)
                .header("if-match", if_match)
                .send()
                .await
                .unwrap()
                .status()
        }
    };

    // The first assignment has no default to preserve, so it must take the default slot.
    assert_eq!(assign("quiet", "\"1\"").await, StatusCode::OK);
    let after_first: serde_json::Value = client
        .get(format!("{agents}/kitchen/templates"))
        .bearer_auth(auth)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let default_count = |items: &serde_json::Value| {
        items["items"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|item| item["is_default"] == true)
            .count()
    };
    assert_eq!(
        default_count(&after_first),
        1,
        "the first assignment must become the default, got {after_first}"
    );
    let find = |items: &serde_json::Value, key: &str| {
        items["items"]
            .as_array()
            .unwrap()
            .iter()
            .find(|item| item["key"] == key)
            .unwrap_or_else(|| panic!("{key} missing from {items}"))
            .clone()
    };
    assert_eq!(find(&after_first, "quiet")["is_default"], true);

    // A second assignment is a switch candidate and must not steal the default slot.
    assert_eq!(assign("loud", "\"2\"").await, StatusCode::OK);
    let after_second: serde_json::Value = client
        .get(format!("{agents}/kitchen/templates"))
        .bearer_auth(auth)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(
        default_count(&after_second),
        1,
        "exactly one enabled default must remain, got {after_second}"
    );
    assert_eq!(find(&after_second, "quiet")["is_default"], true);
    assert_eq!(find(&after_second, "loud")["is_default"], false);
    task.abort();
}

/// An optional Speaker slot must not make the four-slot core completeness check fail: absent core
/// slots fall back to the deployment default, so a Template that binds only a partial core plus a
/// Speaker provider still takes the default slot on its first assignment. A Speaker provider bound
/// under a core slot is still a type mismatch.
#[tokio::test]
async fn first_assignment_with_a_partial_core_and_speaker_slot_becomes_the_default() {
    let (base, task) = server(true).await;
    let client = Client::new();
    let auth = "admin-test-token";
    let agents = format!("{base}/api/admin/agents");
    let templates = format!("{base}/api/admin/templates");
    let providers = format!("{base}/api/admin/providers");

    for (path, body) in [
        (
            &agents,
            serde_json::json!({"key":"kitchen","name":"Kitchen"}),
        ),
        (
            &templates,
            serde_json::json!({"key":"partial","name":"Partial","language":"vi-VN","prompt":"Nói ngắn gọn"}),
        ),
    ] {
        assert_eq!(
            client
                .post(path)
                .bearer_auth(auth)
                .json(&body)
                .send()
                .await
                .unwrap()
                .status(),
            StatusCode::CREATED
        );
    }

    let llm = create_provider(
        &client,
        &providers,
        serde_json::json!({
            "name":"Main LLM", "type":"llm", "adapter":"openai",
            "config_json":{"base_url":"https://example.test/v1","model":"test","max_tokens":8}
        }),
    )
    .await;
    let speaker = create_provider(
        &client,
        &providers,
        serde_json::json!({"type":"speaker","adapter":"campplus_sherpa","name":"Voice","config_json":{}}),
    )
    .await;

    // A Speaker provider may only bind the Speaker slot.
    let wrong_type = client
        .put(format!("{templates}/partial/providers/llm"))
        .bearer_auth(auth)
        .header("if-match", "\"1\"")
        .json(&serde_json::json!({"provider_key":speaker}))
        .send()
        .await
        .unwrap();
    assert_eq!(wrong_type.status(), StatusCode::BAD_REQUEST);
    assert_eq!(
        wrong_type.json::<serde_json::Value>().await.unwrap()["error"]["code"],
        "invalid_provider"
    );

    for (revision, (kind, provider_key)) in (1..).zip([("llm", &llm), ("speaker", &speaker)]) {
        assert_eq!(
            client
                .put(format!("{templates}/partial/providers/{kind}"))
                .bearer_auth(auth)
                .header("if-match", format!("\"{revision}\""))
                .json(&serde_json::json!({"provider_key":provider_key}))
                .send()
                .await
                .unwrap()
                .status(),
            StatusCode::OK
        );
    }

    assert_eq!(
        client
            .put(format!("{agents}/kitchen/templates/partial"))
            .bearer_auth(auth)
            .header("if-match", "\"1\"")
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::OK
    );
    let assignments: serde_json::Value = client
        .get(format!("{agents}/kitchen/templates"))
        .bearer_auth(auth)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let partial = assignments["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["key"] == "partial")
        .unwrap();
    assert_eq!(
        partial["is_default"], true,
        "a partial core plus a Speaker slot must still become the default: {assignments}"
    );

    // Unlinking the Speaker slot is a CAS on the Template revision, and once unbound the provider
    // can be conditionally deleted.
    let before_unlink = client
        .get(format!("{templates}/partial/providers"))
        .bearer_auth(auth)
        .send()
        .await
        .unwrap()
        .json::<serde_json::Value>()
        .await
        .unwrap();
    assert_eq!(
        before_unlink["bindings"]["speaker"]["provider_key"],
        speaker
    );
    let revision = before_unlink["revision"].as_i64().unwrap();
    assert_eq!(
        client
            .delete(format!("{templates}/partial/providers/speaker"))
            .bearer_auth(auth)
            .header("if-match", format!("\"{revision}\""))
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::OK
    );
    let after_unlink = client
        .get(format!("{templates}/partial/providers"))
        .bearer_auth(auth)
        .send()
        .await
        .unwrap()
        .json::<serde_json::Value>()
        .await
        .unwrap();
    assert!(after_unlink["bindings"]["speaker"].is_null());
    assert_eq!(
        client
            .delete(format!("{providers}/{speaker}"))
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
async fn speaker_provider_is_created_cold_without_exposing_execution_configuration() {
    let (base, task) = server(true).await;
    let client = Client::new();
    let key = create_provider(&client, &format!("{base}/api/admin/providers"), serde_json::json!({"type":"speaker","adapter":"campplus_sherpa","name":"Voice","config_json":{}})).await;
    let response = client
        .get(format!("{base}/api/admin/providers/{key}"))
        .bearer_auth("admin-test-token")
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body: serde_json::Value = response.json().await.unwrap();
    assert_eq!(body["type"], "speaker");
    assert!(body["key"].as_str().unwrap().starts_with("speaker_"));
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(body["config_json"].as_str().unwrap()).unwrap()["min_speech_ms"],
        2000
    );
    assert_eq!(body["runtime_status"], "not_loaded");
    let diagnostic = client
        .post(format!("{base}/api/admin/providers/{key}/test/speaker"))
        .bearer_auth("admin-test-token")
        .header("If-Match", "\"1\"")
        .header("Content-Type", "audio/wav")
        .body(wav_pcm16_mono(16000, 16000))
        .send()
        .await
        .unwrap();
    assert_eq!(diagnostic.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(
        diagnostic.json::<serde_json::Value>().await.unwrap()["error"]["code"],
        "speaker_runtime_manager_required"
    );
    for config in [
        serde_json::json!({"model":"arbitrary"}),
        serde_json::json!({"num_threads":99}),
        serde_json::json!({"max_window_ms":6001}),
        serde_json::json!({"min_speech_ms":5000,"target_speech_ms":1000}),
    ] {
        let invalid=client.post(format!("{base}/api/admin/providers")).bearer_auth("admin-test-token").json(&serde_json::json!({"name":"Invalid","type":"speaker","adapter":"campplus_sherpa","config_json":config})).send().await.unwrap();
        assert_eq!(invalid.status(), StatusCode::BAD_REQUEST);
    }
    task.abort();
}

struct QualificationSpeaker {
    slow: bool,
    calls: usize,
}
impl voice_agent_server::providers::speaker::SpeakerProvider for QualificationSpeaker {
    fn dimension(&self) -> usize {
        3
    }
    fn extract(
        &mut self,
        _: &PcmF32Mono,
    ) -> Result<Vec<f32>, voice_agent_server::providers::speaker::SpeakerError> {
        if self.slow && self.calls > 0 {
            std::thread::sleep(Duration::from_millis(200));
        }
        self.calls += 1;
        Ok(vec![1.0, 2.0, 3.0])
    }
}
struct QualificationSpeakerResource(Arc<voice_agent_server::providers::speaker::SpeakerRuntime>);
impl voice_agent_server::services::provider_runtime::RuntimeResource
    for QualificationSpeakerResource
{
    fn unload(&self) -> bool {
        self.0.shutdown_acknowledged()
    }
    fn runtimes_for(
        &self,
        snapshot: &voice_agent_server::database::DesiredProvider,
        quota: voice_agent_server::workers::ProviderRuntimeAdmission,
    ) -> Option<voice_agent_server::providers::RuntimeCatalog> {
        Some(
            voice_agent_server::providers::RuntimeCatalog::single_speaker(
                snapshot.key.clone(),
                Arc::new(self.0.logical_view(quota)),
            ),
        )
    }
}
struct QualificationSpeakerFactory {
    slow: bool,
}
impl voice_agent_server::services::provider_runtime::RuntimeMaterializer
    for QualificationSpeakerFactory
{
    fn estimated_peak_bytes(
        &self,
        _: &voice_agent_server::database::DesiredProvider,
    ) -> Result<u64, voice_agent_server::services::provider_runtime::RuntimeError> {
        Ok(1)
    }
    fn logical_capacity(
        &self,
        _: &voice_agent_server::database::DesiredProvider,
    ) -> Result<usize, voice_agent_server::services::provider_runtime::RuntimeError> {
        Ok(1)
    }
    fn build(
        &self,
        _snapshot: &voice_agent_server::database::DesiredProvider,
        _: Option<voice_agent_server::services::provider_runtime::PreparedRuntime>,
        quota: voice_agent_server::workers::ProviderRuntimeAdmission,
    ) -> Result<
        Arc<dyn voice_agent_server::services::provider_runtime::RuntimeResource>,
        voice_agent_server::services::provider_runtime::RuntimeError,
    > {
        Ok(Arc::new(QualificationSpeakerResource(Arc::new(
            voice_agent_server::providers::speaker::SpeakerRuntime::new(
                Box::new(QualificationSpeaker {
                    slow: self.slow,
                    calls: 0,
                }),
                quota,
            )
            .unwrap(),
        ))))
    }
}

#[tokio::test]
async fn speaker_diagnostic_is_managed_exact_bounded_and_never_returns_embeddings() {
    use voice_agent_server::services::{
        provider_diagnostic::{ProviderDiagnosticLimiter, ProviderDiagnosticService},
        provider_runtime::{ProviderRuntimeManager, RuntimeLimits},
    };
    let config: AppConfig = toml::from_str(&format!(
        r#"
[server]
bind="127.0.0.1:0"
public_ws_url="ws://127.0.0.1:0/voice/v1/"
[provider_defaults]
vad="test"
asr="test"
llm="test"
tts="test"
[database]
url="{}"
[api]
enabled=true
admin_token="admin-test-token"
"#,
        database_url()
    ))
    .unwrap();
    let database = Database::connect(&config.database).await.unwrap();
    let mut state = AppState::from_provider_set_with_database(
        config,
        Arc::new(ProviderSet::unavailable()),
        Some(database),
    );
    let manager = ProviderRuntimeManager::new(
        RuntimeLimits {
            max_parallel_loads: 1,
            max_pending_loads: 2,
            max_waiters: 8,
            max_resident_bytes: 4,
            max_resources: 4,
            max_version_entries: 8,
            admission_timeout_ms: 1000,
            failure_cooldown_ms: 100,
            idle_ttl_ms: 1000,
        },
        Arc::new(QualificationSpeakerFactory { slow: false }),
        voice_agent_server::lifecycle::AdmissionGate::open(),
    )
    .unwrap();
    state.provider_diagnostics = Arc::new(
        ProviderDiagnosticService::new(
            Arc::new(Default::default()),
            None,
            state.database.clone(),
            ProviderDiagnosticLimiter::new(1),
            Duration::from_secs(1),
        )
        .with_runtime_manager(manager.clone()),
    );
    state.provider_runtime_manager = Some(manager);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let task = tokio::spawn(async move {
        axum::serve(listener, router_with_state(state))
            .await
            .unwrap()
    });
    let client = Client::new();
    let key=create_provider(&client,&format!("{base}/api/admin/providers"),serde_json::json!({"type":"speaker","adapter":"campplus_sherpa","name":"Voice","config_json":{}})).await;
    let provider = format!("{base}/api/admin/providers/{key}");
    let cold: serde_json::Value = client
        .get(&provider)
        .bearer_auth("admin-test-token")
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(cold["runtime"]["desired_state"], "cold");
    let mut wav = wav_pcm16_mono(16_000, 16_000);
    for sample in wav[44..].chunks_exact_mut(2) {
        sample.copy_from_slice(&2000_i16.to_le_bytes())
    }
    let diagnostic = format!("{provider}/test/speaker");
    let result = client
        .post(&diagnostic)
        .header("If-Match", "\"1\"")
        .bearer_auth("admin-test-token")
        .header("Content-Type", "audio/wav")
        .body(wav.clone())
        .send()
        .await
        .unwrap();
    assert_eq!(result.status(), StatusCode::OK);
    let result: serde_json::Value = result.json().await.unwrap();
    assert_eq!(result["provenance"]["dimension"], 3);
    assert_eq!(result["runtime"]["tested_revision"], 1);
    assert!(
        result["provenance"]["embedding_space_id"]
            .as_str()
            .unwrap()
            .starts_with("speaker:")
    );
    assert!(!result.to_string().contains("embedding\":"));
    assert_eq!(
        client
            .post(&diagnostic)
            .header("If-Match", "\"1\"")
            .bearer_auth("admin-test-token")
            .header("Content-Type", "application/json")
            .body(wav.clone())
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        client
            .post(&diagnostic)
            .header("If-Match", "\"1\"")
            .bearer_auth("admin-test-token")
            .header("Content-Type", "audio/wav")
            .body(wav_pcm16_mono(8000, 8000))
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        client
            .post(&diagnostic)
            .header("If-Match", "\"1\"")
            .bearer_auth("admin-test-token")
            .header("Content-Type", "audio/wav")
            .body(vec![0u8; 524_289])
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::PAYLOAD_TOO_LARGE
    );
    assert_eq!(
        client
            .patch(&provider)
            .bearer_auth("admin-test-token")
            .header("If-Match", "\"1\"")
            .json(&serde_json::json!({"name":"Revised"}))
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::OK
    );
    let result: serde_json::Value = client
        .post(&diagnostic)
        .header("If-Match", "\"2\"")
        .bearer_auth("admin-test-token")
        .header("Content-Type", "audio/wav")
        .body(wav)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(result["runtime"]["tested_revision"], 2);
    task.abort();
}

#[tokio::test]
async fn speaker_waiter_timeout_retains_exact_lease_and_capacity_until_native_completion() {
    use voice_agent_server::{
        database::DesiredProvider,
        providers::speaker::SpeakerError,
        services::provider_runtime::{ProviderRuntimeManager, RuntimeLimits},
    };
    let manager = ProviderRuntimeManager::new(
        RuntimeLimits {
            max_parallel_loads: 1,
            max_pending_loads: 2,
            max_waiters: 8,
            max_resident_bytes: 4,
            max_resources: 4,
            max_version_entries: 8,
            admission_timeout_ms: 1000,
            failure_cooldown_ms: 100,
            idle_ttl_ms: 1000,
        },
        Arc::new(QualificationSpeakerFactory { slow: true }),
        voice_agent_server::lifecycle::AdmissionGate::open(),
    )
    .unwrap();
    let snapshot = DesiredProvider {
        id: 1,
        key: "speaker_test".into(),
        kind: "speaker".into(),
        adapter: "campplus_sherpa".into(),
        config_json: "{}".into(),
        secret_ref: None,
        revision: 1,
    };
    let lease = manager.acquire(snapshot.clone()).await.unwrap();
    let runtime = lease.runtimes().unwrap().speaker(&snapshot.key).unwrap();
    let pcm = PcmF32Mono::new(vec![0.1; 16000], 16000);
    assert!(
        tokio::time::timeout(
            Duration::from_millis(20),
            runtime.extract(pcm.clone(), lease)
        )
        .await
        .is_err()
    );
    assert_eq!(manager.accounting().active_leases, 1);
    let second = manager.acquire(snapshot).await.unwrap();
    assert!(matches!(
        runtime.extract(pcm, second).await,
        Err(SpeakerError::Busy)
    ));
    tokio::time::sleep(Duration::from_millis(250)).await;
    assert_eq!(manager.accounting().active_leases, 0);
    assert_eq!(manager.accounting().physical_inference_usage, 0);
}

#[tokio::test]
async fn external_tool_reviews_are_admin_authenticated_and_default_empty() {
    let (base, task) = server_with_database(true, &database_url()).await;
    let client = Client::new();
    let response = client
        .post(format!("{base}/api/admin/agents"))
        .bearer_auth("admin-test-token")
        .json(&serde_json::json!({"key":"review_agent","name":"Review Agent"}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
    let url = format!("{base}/api/admin/agents/review_agent/tool-allowlist");
    assert_eq!(
        client.get(&url).send().await.unwrap().status(),
        StatusCode::UNAUTHORIZED
    );
    let response = client
        .get(&url)
        .bearer_auth("admin-test-token")
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.json::<serde_json::Value>().await.unwrap()["items"],
        serde_json::json!([])
    );
    task.abort();
}

#[tokio::test]
async fn device_tool_reviews_are_admin_authenticated_and_guarded_by_revision() {
    use voice_agent_server::{
        config::DatabaseConfig, database::device_tool_allowlist, tools::device_mcp::DiscoveredTool,
    };
    let database_uri = database_url();
    let config = DatabaseConfig {
        url: database_uri.clone(),
        max_connections: 2,
        busy_timeout_ms: 5_000,
        migrate_on_start: true,
        devices: Default::default(),
        history: Default::default(),
    };
    let database = Database::connect(&config).await.unwrap();
    sqlx::query("INSERT INTO agents (key,name,enabled,created_at,updated_at) VALUES ('review_agent','Review Agent',1,1,1)")
        .execute(database.pool())
        .await
        .unwrap();
    let device = sqlx::query("INSERT INTO devices (device_id,agent_id,enabled,created_at,updated_at) VALUES ('review-device',1,1,1,1)")
        .execute(database.pool())
        .await
        .unwrap()
        .last_insert_rowid();
    sqlx::query("INSERT INTO agent_speaker_policies (agent_id,mode) VALUES (1,'observe')")
        .execute(database.pool())
        .await
        .unwrap();
    device_tool_allowlist::observe(
        &database,
        device,
        &[DiscoveredTool {
            original_name: "SetBrightness".into(),
            description: "set brightness".into(),
            input_schema: serde_json::json!({"type":"object"}),
        }],
    )
    .await
    .unwrap();
    drop(database);

    let (base, task) = server_with_database(true, &database_uri).await;
    let client = Client::new();
    let url = format!("{base}/api/admin/agents/review_agent/device-tool-allowlist");
    assert_eq!(
        client.get(&url).send().await.unwrap().status(),
        StatusCode::UNAUTHORIZED
    );
    let listed = client
        .get(&url)
        .bearer_auth("admin-test-token")
        .send()
        .await
        .unwrap();
    assert_eq!(listed.status(), StatusCode::OK);
    let listed = listed.json::<serde_json::Value>().await.unwrap();
    let item = &listed["items"][0];
    assert_eq!(item["device_id"], serde_json::json!("review-device"));
    assert_eq!(item["original_name"], serde_json::json!("SetBrightness"));
    assert_eq!(item["presence"], serde_json::json!("observed_only"));
    assert_eq!(item["allowed"], serde_json::json!(false));

    let fingerprint = item["fingerprint"].as_str().unwrap().to_owned();
    let observed_revision = item["observed_revision"].as_i64().unwrap();
    // Approving with a stale revision conflicts.
    let stale = client
        .put(&url)
        .bearer_auth("admin-test-token")
        .header("if-match", "\"7\"")
        .json(&serde_json::json!({
            "device_id": "review-device",
            "original_name": "SetBrightness",
            "observed_revision": observed_revision,
            "fingerprint": fingerprint,
            "allowed": true
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(stale.status(), StatusCode::CONFLICT);

    let approved = client
        .put(&url)
        .bearer_auth("admin-test-token")
        .header("if-match", "\"1\"")
        .json(&serde_json::json!({
            "device_id": "review-device",
            "original_name": "SetBrightness",
            "observed_revision": observed_revision,
            "fingerprint": fingerprint,
            "allowed": true
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(approved.status(), StatusCode::OK);

    // A changed contract no longer matches the observation the review was based on.
    let drifted = client
        .put(&url)
        .bearer_auth("admin-test-token")
        .header("if-match", "\"2\"")
        .json(&serde_json::json!({
            "device_id": "review-device",
            "original_name": "SetBrightness",
            "observed_revision": observed_revision,
            "fingerprint": "0".repeat(64),
            "allowed": true
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(drifted.status(), StatusCode::CONFLICT);
    task.abort();
}
