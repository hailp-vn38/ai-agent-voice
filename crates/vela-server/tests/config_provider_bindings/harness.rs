pub use std::{
    fs,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

pub use reqwest::{Client, StatusCode};
pub use serde_json::{Value, json};
pub use vela_server::{
    app::{AppState, router_with_state},
    audio::PcmF32Mono,
    config::{AppConfig, DatabaseConfig, DatabaseDevicesConfig},
    database::{Database, DesiredProvider},
    lifecycle::AdmissionGate,
    providers::{
        AsrError, AsrEvent, AsrProvider, AsrResult, AsrSession, DiagnosticRuntimeKind, LlmError,
        LlmProvider, ProviderSet, ResolvedAgentRuntimes, RuntimeCatalog, TtsDiagnosticRequest,
        TtsError, TtsProvider, VadError, VadInput, VadProbability, VadProvider, VadSession,
        llm::{LlmEvent, LlmEventStream, LlmRequest},
    },
    services::provider_runtime::{
        PreparedRuntime, ProviderRuntimeManager, RuntimeError, RuntimeLimits, RuntimeMaterializer,
        RuntimeResource,
    },
    workers::{
        AsrWorkerRuntime, LlmRuntime, ProviderRuntimeAdmission, TtsWorkerRuntime, VadWorkerRuntime,
        WorkerRuntimeConfig, WorkerSupervisor,
    },
};

pub struct Resource(pub RuntimeCatalog);

impl RuntimeResource for Resource {
    fn runtimes(&self) -> Option<RuntimeCatalog> {
        Some(self.0.clone())
    }
    fn unload(&self) -> bool {
        true
    }
    fn capabilities(&self) -> Option<Value> {
        Some(json!({"voices":["fixture"]}))
    }
}

pub struct Builder(
    pub Arc<AtomicUsize>,
    pub std::time::Duration,
    pub bool,
    pub Arc<WorkerSupervisor>,
);

impl RuntimeMaterializer for Builder {
    fn estimated_peak_bytes(&self, _: &DesiredProvider) -> Result<u64, RuntimeError> {
        Ok(1)
    }
    fn logical_capacity(&self, _: &DesiredProvider) -> Result<usize, RuntimeError> {
        Ok(1)
    }
    fn build(
        &self,
        snapshot: &DesiredProvider,
        _: Option<PreparedRuntime>,
        _: ProviderRuntimeAdmission,
    ) -> Result<Arc<dyn RuntimeResource>, RuntimeError> {
        self.0.fetch_add(1, Ordering::SeqCst);
        std::thread::sleep(self.1);
        let worker = WorkerRuntimeConfig {
            max_workers: 1,
            voice_reserved_capacity: 1,
            command_capacity: 4,
            final_timeout: std::time::Duration::from_secs(1),
            cleanup_grace: std::time::Duration::from_secs(1),
        };
        let runtimes = ResolvedAgentRuntimes {
            vad: Arc::new(VadWorkerRuntime::new(Arc::new(FixtureVad), worker.clone())),
            asr: Arc::new(AsrWorkerRuntime::new(Arc::new(FixtureAsr), worker.clone())),
            llm: Arc::new(LlmRuntime::new(
                Arc::new(FixtureLlm(snapshot.key.clone(), self.2)),
                1,
                std::time::Duration::from_secs(1),
            )),
            tts: Arc::new(TtsWorkerRuntime::new(Arc::new(FixtureTts), worker)),
            speaker: None,
            vad_segmenter: Default::default(),
            vad_pre_roll_samples: 0,
        };
        if snapshot.kind == "vad" {
            self.3.observe_vad(Arc::clone(&runtimes.vad));
        }
        let kind = match snapshot.kind.as_str() {
            "vad" => DiagnosticRuntimeKind::Vad,
            "asr" => DiagnosticRuntimeKind::Asr,
            "llm" => DiagnosticRuntimeKind::Llm,
            "tts" => DiagnosticRuntimeKind::Tts,
            _ => return Err(RuntimeError::Configuration),
        };
        Ok(Arc::new(Resource(RuntimeCatalog::single_provider(
            kind,
            snapshot.key.clone(),
            &runtimes,
        ))))
    }
}

pub async fn server(
    managed: bool,
) -> (
    String,
    Database,
    Arc<AtomicUsize>,
    tokio::task::JoinHandle<()>,
) {
    let (base, db, builds, task, _) =
        server_configured(managed, std::time::Duration::ZERO, false, 5000).await;
    (base, db, builds, task)
}

pub async fn server_configured(
    managed: bool,
    load_delay: std::time::Duration,
    hang_llm: bool,
    timeout_ms: u64,
) -> (
    String,
    Database,
    Arc<AtomicUsize>,
    tokio::task::JoinHandle<()>,
    Option<Arc<ProviderRuntimeManager>>,
) {
    let path = std::env::temp_dir().join(format!("config-bindings-{}.toml", uuid::Uuid::new_v4()));
    fs::write(
        &path,
        r#"
[server]
bind="127.0.0.1:0"
public_ws_url="ws://127.0.0.1:0/voice/v1/"
[provider_defaults]
vad="vad"
asr="asr"
llm="primary"
tts="primary"
[api]
enabled=true
admin_token="admin-test-token"
jwt_secret="bindings-test-signing-secret-0123456789"
[providers.vad.instances.vad]
adapter="silero_onnx"
[providers.asr.instances.asr]
adapter="gipformer_sherpa_offline"
[providers.llm.instances.primary]
adapter="openai"
base_url="https://example.test/v1"
model="fixture"
api_key="SECRET_SENTINEL_SHOULD_NOT_APPEAR"
[providers.tts.instances.primary]
adapter="zerotts_onnx"
voice="maichi"
"#,
    )
    .unwrap();
    let mut config = AppConfig::parse_and_resolve(&path).unwrap();
    config.api.provider_tests.timeout_ms = timeout_ms;
    config.api.provider_tests.max_concurrency = 1;
    fs::remove_file(path).unwrap();
    let db = Database::connect(&DatabaseConfig {
        url: "sqlite::memory:".into(),
        max_connections: 1,
        ..Default::default()
    })
    .await
    .unwrap();
    sqlx::raw_sql(r#"
INSERT INTO providers (key,name,type,adapter,config_json,created_at,updated_at) VALUES ('primary','primary','llm','openai','{"base_url":"https://example.test/v1","model":"fixture"}',1,1),('db-tts','Z TTS','tts','openai','{"base_url":"https://example.test/v1","model":"tts"}',1,1);
INSERT INTO agent_templates (key,name,language,prompt,created_at,updated_at) VALUES ('template','Template','vi','hello',1,1);
INSERT INTO agents (key,name,created_at,updated_at) VALUES ('agent','Agent',1,1);
INSERT INTO devices (device_id,agent_id,created_at,updated_at) VALUES ('device',1,1,1);
INSERT INTO agent_template_assignments (agent_id,template_id,is_default,created_at) VALUES (1,1,1,1);
"#).execute(db.pool()).await.unwrap();
    crate::admin_web::seed_default_admin(db.pool()).await;
    let builds = Arc::new(AtomicUsize::new(0));
    let mut state = AppState::from_provider_set_with_database(
        config,
        Arc::new(ProviderSet::unavailable()),
        Some(db.clone()),
    );
    if managed {
        let manager = ProviderRuntimeManager::new(
            RuntimeLimits {
                max_parallel_loads: 1,
                max_pending_loads: 4,
                max_waiters: 16,
                max_resident_bytes: 40,
                max_resources: 8,
                max_version_entries: 16,
                admission_timeout_ms: 1000,
                failure_cooldown_ms: 100,
                idle_ttl_ms: 1000,
            },
            Arc::new(Builder(
                builds.clone(),
                load_delay,
                hang_llm,
                state.worker_supervisor.clone(),
            )),
            AdmissionGate::open(),
        )
        .unwrap();
        state = state.with_runtime_manager(manager);
    }
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let manager = state.provider_runtime_manager.clone();
    let router = router_with_state(state);
    let task = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    (
        format!("http://{address}/api/admin"),
        db,
        builds,
        task,
        manager,
    )
}

pub async fn client(base: &str) -> Client {
    let token = crate::admin_web::admin_web_token(
        &Client::new(),
        base.trim_end_matches("/api/admin"),
    )
    .await;
    let mut headers = reqwest::header::HeaderMap::new();
    headers.insert(
        reqwest::header::AUTHORIZATION,
        format!("Bearer {token}").parse().unwrap(),
    );
    Client::builder().default_headers(headers).build().unwrap()
}

pub async fn get(client: &Client, url: &str) -> Value {
    let response = client.get(url).send().await.unwrap();
    assert_eq!(response.status(), StatusCode::OK, "{url}");
    let text = response.text().await.unwrap();
    assert!(!text.contains("SECRET_SENTINEL_SHOULD_NOT_APPEAR"));
    serde_json::from_str(&text).unwrap()
}

pub struct FixtureVad;
impl VadProvider for FixtureVad {
    fn open(&self) -> Result<Box<dyn VadSession>, VadError> {
        Ok(Box::new(FixtureVad))
    }
    fn adapter(&self) -> &'static str {
        "fixture"
    }
}
impl VadSession for FixtureVad {
    fn push(&mut self, input: VadInput) -> Result<VadProbability, VadError> {
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

pub struct FixtureAsr;
impl AsrProvider for FixtureAsr {
    fn open(&self) -> Result<Box<dyn AsrSession>, AsrError> {
        Ok(Box::new(FixtureAsr))
    }
}
impl AsrSession for FixtureAsr {
    fn push_pcm(&mut self, _: &PcmF32Mono) -> Result<Vec<AsrEvent>, AsrError> {
        Ok(Vec::new())
    }
    fn finish(&mut self) -> Result<AsrResult, AsrError> {
        Ok(AsrResult::new("recognized"))
    }
    fn cancel(&mut self) {}
}

pub struct FixtureLlm(pub String, pub bool);
#[async_trait::async_trait]
impl LlmProvider for FixtureLlm {
    async fn stream(&self, request: LlmRequest) -> Result<LlmEventStream, LlmError> {
        if self.1 {
            return Ok(Box::pin(futures_util::stream::pending()));
        }
        Ok(Box::pin(futures_util::stream::iter([
            Ok(LlmEvent::TextDelta(self.complete(&request)?)),
            Ok(LlmEvent::Finished),
        ])))
    }

    fn adapter(&self) -> &'static str {
        "fixture"
    }
    fn complete(&self, _: &LlmRequest) -> Result<String, LlmError> {
        Ok(format!("answer:{}", self.0))
    }
}

pub struct FixtureTts;
impl TtsProvider for FixtureTts {
    fn adapter(&self) -> &'static str {
        "fixture"
    }
    fn validate_diagnostic(&self, _: &TtsDiagnosticRequest) -> Result<(), TtsError> {
        Ok(())
    }
    fn synthesize_diagnostic(
        &self,
        _: &TtsDiagnosticRequest,
        _: &std::sync::atomic::AtomicBool,
        on_pcm: &mut dyn FnMut(PcmF32Mono) -> Result<(), TtsError>,
    ) -> Result<(), TtsError> {
        on_pcm(PcmF32Mono::new(vec![0.0; 512], 16_000))
    }
}
