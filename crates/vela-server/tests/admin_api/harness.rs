pub use std::{
    fs,
    sync::{Arc, Mutex},
    time::Duration,
};

pub use reqwest::{Client, StatusCode};
pub use serde_json::{Value, json};
pub use vela_server::{
    app::{AppState, bootstrap_with_providers_and_secret_resolver, router_with_state},
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

pub struct DiagnosticLlm {
    pub requests: Arc<Mutex<Vec<LlmRequest>>>,
}

pub struct DiagnosticTts {
    pub requests: Arc<Mutex<Vec<TtsDiagnosticRequest>>>,
}

pub struct DiagnosticAsr {
    pub received_samples: Arc<Mutex<Vec<usize>>>,
}

pub struct DiagnosticAsrSession {
    pub received_samples: Arc<Mutex<Vec<usize>>>,
    pub samples: usize,
}

pub struct DiagnosticVad {
    pub inputs: Arc<Mutex<Vec<VadInput>>>,
}

pub struct DiagnosticVadSession {
    pub inputs: Arc<Mutex<Vec<VadInput>>>,
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

pub struct QualificationSpeaker {
    pub slow: bool,
    pub calls: usize,
}

impl vela_server::providers::speaker::SpeakerProvider for QualificationSpeaker {
    fn dimension(&self) -> usize {
        3
    }
    fn extract(
        &mut self,
        _: &PcmF32Mono,
    ) -> Result<Vec<f32>, vela_server::providers::speaker::SpeakerError> {
        if self.slow && self.calls > 0 {
            std::thread::sleep(Duration::from_millis(200));
        }
        self.calls += 1;
        Ok(vec![1.0, 2.0, 3.0])
    }
}

pub struct QualificationSpeakerResource(pub Arc<vela_server::providers::speaker::SpeakerRuntime>);

impl vela_server::services::provider_runtime::RuntimeResource for QualificationSpeakerResource {
    fn unload(&self) -> bool {
        self.0.shutdown_acknowledged()
    }
    fn runtimes_for(
        &self,
        snapshot: &vela_server::database::DesiredProvider,
        quota: vela_server::workers::ProviderRuntimeAdmission,
    ) -> Option<vela_server::providers::RuntimeCatalog> {
        Some(vela_server::providers::RuntimeCatalog::single_speaker(
            snapshot.key.clone(),
            Arc::new(self.0.logical_view(quota)),
        ))
    }
}

pub struct QualificationSpeakerFactory {
    pub slow: bool,
}

impl vela_server::services::provider_runtime::RuntimeMaterializer for QualificationSpeakerFactory {
    fn estimated_peak_bytes(
        &self,
        _: &vela_server::database::DesiredProvider,
    ) -> Result<u64, vela_server::services::provider_runtime::RuntimeError> {
        Ok(1)
    }
    fn logical_capacity(
        &self,
        _: &vela_server::database::DesiredProvider,
    ) -> Result<usize, vela_server::services::provider_runtime::RuntimeError> {
        Ok(1)
    }
    fn build(
        &self,
        _snapshot: &vela_server::database::DesiredProvider,
        _: Option<vela_server::services::provider_runtime::PreparedRuntime>,
        quota: vela_server::workers::ProviderRuntimeAdmission,
    ) -> Result<
        Arc<dyn vela_server::services::provider_runtime::RuntimeResource>,
        vela_server::services::provider_runtime::RuntimeError,
    > {
        Ok(Arc::new(QualificationSpeakerResource(Arc::new(
            vela_server::providers::speaker::SpeakerRuntime::new(
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

pub fn database_url() -> String {
    format!(
        "sqlite://{}",
        std::env::temp_dir()
            .join(format!("vela-admin-{}", uuid::Uuid::new_v4()))
            .join("vela.db")
            .display()
    )
}

pub async fn server(api_enabled: bool) -> (String, tokio::task::JoinHandle<()>) {
    server_with_database(api_enabled, &database_url()).await
}

pub async fn server_with_database(
    api_enabled: bool,
    database_uri: &str,
) -> (String, tokio::task::JoinHandle<()>) {
    server_with_resolver(
        api_enabled,
        database_uri,
        Arc::new(vela_server::database::secrets::EnvSecretResolver),
    )
    .await
}

pub async fn server_with_resolver(
    api_enabled: bool,
    database_uri: &str,
    resolver: Arc<dyn vela_server::database::secrets::SecretResolver>,
) -> (String, tokio::task::JoinHandle<()>) {
    let config_path =
        std::env::temp_dir().join(format!("vela-admin-{}.toml", uuid::Uuid::new_v4()));
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
jwt_secret = "admin-harness-signing-secret-0123456789"
"#,
            database_uri, api_enabled
        ),
    )
    .unwrap();
    let config = AppConfig::parse_and_resolve(&config_path).unwrap();
    fs::remove_file(config_path).unwrap();
    let router = bootstrap_with_providers_and_secret_resolver(
        config,
        Arc::new(ProviderSet::unavailable()),
        resolver,
    )
    .await
    .unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let task = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    (format!("http://{address}"), task)
}

/// Creates a Provider through the public Admin API and returns the key the server generated.
pub async fn create_provider(client: &Client, providers: &str, body: serde_json::Value) -> String {
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
pub async fn seeded_provider(body: serde_json::Value) -> (String, String) {
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
pub async fn server_with_closed_database() -> (String, tokio::task::JoinHandle<()>) {
    resource_server(true, false).await
}

pub async fn resource_server(
    api_enabled: bool,
    collect: bool,
) -> (String, tokio::task::JoinHandle<()>) {
    let config_path =
        std::env::temp_dir().join(format!("vela-admin-closed-{}.toml", uuid::Uuid::new_v4()));
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
jwt_secret = "admin-harness-signing-secret-0123456789"
"#,
            database_url(),
            api_enabled
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
    if collect {
        let stop = state.lifecycle.listening().clone();
        let sampler = state.system_metrics.clone().start(stop.clone());
        tokio::time::timeout(Duration::from_secs(3), async {
            while state.system_metrics.latest().is_none() {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        stop.cancel();
        sampler.await.unwrap();
    }
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let task = tokio::spawn(async move {
        axum::serve(listener, router_with_state(state))
            .await
            .unwrap()
    });
    (format!("http://{address}"), task)
}

pub async fn server_with_loaded_llm(
    database_uri: &str,
    instance_id: &str,
) -> (
    String,
    Arc<Mutex<Vec<LlmRequest>>>,
    tokio::task::JoinHandle<()>,
) {
    let config_path = std::env::temp_dir().join(format!(
        "vela-admin-diagnostic-{}.toml",
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
jwt_secret = "admin-harness-signing-secret-0123456789"
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

pub async fn server_with_loaded_tts(
    database_uri: &str,
    instance_id: &str,
) -> (
    String,
    Arc<Mutex<Vec<TtsDiagnosticRequest>>>,
    tokio::task::JoinHandle<()>,
) {
    let config_path = std::env::temp_dir().join(format!(
        "vela-admin-tts-diagnostic-{}.toml",
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
jwt_secret = "admin-harness-signing-secret-0123456789"
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
            max_workers: 1,
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

pub async fn server_with_loaded_asr(
    database_uri: &str,
    instance_id: &str,
) -> (String, Arc<Mutex<Vec<usize>>>, tokio::task::JoinHandle<()>) {
    let config_path = std::env::temp_dir().join(format!(
        "vela-admin-asr-diagnostic-{}.toml",
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
jwt_secret = "admin-harness-signing-secret-0123456789"
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

pub async fn server_with_loaded_vad(
    database_uri: &str,
    instance_id: &str,
) -> (
    String,
    Arc<Mutex<Vec<VadInput>>>,
    tokio::task::JoinHandle<()>,
) {
    let config_path = std::env::temp_dir().join(format!(
        "vela-admin-vad-diagnostic-{}.toml",
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
jwt_secret = "admin-harness-signing-secret-0123456789"
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

pub fn wav_pcm16_mono(sample_rate: u32, samples: usize) -> Vec<u8> {
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

/// Helper for writer lock concurrency assertion.
pub async fn assert_waits_for_writer(
    database_uri: &str,
    request: reqwest::RequestBuilder,
    expected: StatusCode,
) {
    let writer = sqlx::SqlitePool::connect(database_uri).await.unwrap();
    let lock = writer.begin_with("BEGIN IMMEDIATE").await.unwrap();
    let response = tokio::spawn(request.send());
    tokio::time::sleep(Duration::from_millis(150)).await;
    lock.rollback().await.unwrap();
    let response = response.await.unwrap().unwrap();
    let status = response.status();
    let body = response.text().await.unwrap();
    assert_eq!(status, expected, "{body}");
    writer.close().await;
}

/// The password the harness sets on every fresh Admin database.
pub const HARNESS_ADMIN_PASSWORD: &str = "harness-admin-password";

/// Logs in with the default Admin Password, rotates it to `HARNESS_ADMIN_PASSWORD`, and returns a
/// usable Admin Web Token. Tests that call protected Admin routes use this instead of `admin_token`.
pub async fn admin_web_token(client: &Client, base: &str) -> String {
    let login: Value = client
        .post(format!("{base}/api/admin/auth/login"))
        .json(&json!({ "password": "admin" }))
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    let first_token = login["token"].as_str().unwrap();
    let changed: Value = client
        .post(format!("{base}/api/admin/auth/password"))
        .bearer_auth(first_token)
        .json(&json!({
            "current_password": "admin",
            "new_password": HARNESS_ADMIN_PASSWORD,
        }))
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    changed["token"].as_str().unwrap().to_owned()
}
