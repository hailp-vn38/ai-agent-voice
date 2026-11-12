use std::{
    collections::VecDeque,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

pub use crate::support::admin_auth::admin_web_token;
pub use futures_util::{SinkExt, StreamExt, stream};
use opus2::{Application, Channels, Encoder};
use sqlx::{FromRow, SqlitePool};
use tokio::sync::Notify;
use tokio::{net::TcpListener, task::JoinHandle, time::timeout};
use tokio_tungstenite::{
    connect_async,
    tungstenite::{Message, client::IntoClientRequest, http::StatusCode},
};
use url::Url;
pub use vela_server::{
    app::{AppState, router_with_state},
    config::{
        AdminApiConfig, AppConfig, AudioConfig, SecretString, AuthConfig, BargeInConfig, DatabaseConfig,
        DatabaseHistoryConfig, DeploymentConfig, LimitsConfig, LlmConfig, McpConfig,
        ProviderDefaultsConfig, ProvidersConfig, RuntimeConfig, ServerConfig, SileroOnnxConfig,
        SpeechOutputConfig, TtsConfig, VadInstanceConfig, VisionConfig, WebsocketConfig,
        WorkersConfig,
    },
    database::{
        Database,
        history::{HistoryWriterMetrics, RetentionCleaner, unix_millis_now},
    },
    providers::{
        AsrError, AsrEvent, AsrProvider, AsrResult, AsrSession, LlmError, LlmEvent, LlmProvider,
        ProviderSet, TtsError, TtsProvider, VadError, VadInput, VadProbability, VadProvider,
        VadSession,
        llm::{ChatMessage, LlmEventStream, LlmRequest, ToolCall},
    },
    tools::builtin::EXIT_TOOL_NAME,
};

pub const PROMPT_MARKER: &str = "SERVER-DEFAULT-PROMPT";
pub const TEMPLATE_PROMPT_V1: &str = "TEMPLATE-PROMPT-V1";
pub const TEMPLATE_PROMPT_V2: &str = "TEMPLATE-PROMPT-V2";
pub const GOODBYE_ARGUMENT: &str = "SECRET-GOODBYE-FROM-A-TOOL-ARGUMENT";

// ---------------------------------------------------------------- scripted providers

/// One scripted first round of a Conversational Turn.  A tool round is answered by a continuation
/// of the same turn, so the script is only consumed by a round that has no ToolResult in it.
pub enum Step {
    Answer(&'static str),
    Call {
        name: &'static str,
        arguments: serde_json::Value,
    },
}

/// Answers from a script and records every request it was handed, so a test can assert what the
/// prompt actually contained rather than that something was sent.
pub struct ScriptLlm {
    pub label: &'static str,
    pub steps: Mutex<VecDeque<Step>>,
    pub prompts: Mutex<Vec<usize>>,
    /// How long the very first request stalls before answering.
    ///
    /// An interrupt has to be applied by the actor, and a WebSocket send only flushes to the socket,
    /// so a test cannot use "I sent the abort" as proof the actor has seen it.  A stall the interrupt
    /// comfortably lands inside is what makes that ordering a fact; `asked` is what makes the stall
    /// start at a point the test chose.
    pub stall: Duration,
    pub stalled_once: AtomicBool,
    pub asked: Arc<Notify>,
}

impl ScriptLlm {
    pub fn new(label: &'static str, steps: Vec<Step>) -> Self {
        Self {
            label,
            steps: Mutex::new(steps.into()),
            prompts: Mutex::new(Vec::new()),
            stall: Duration::ZERO,
            stalled_once: AtomicBool::new(false),
            asked: Arc::new(Notify::new()),
        }
    }

    pub fn stalling(label: &'static str, steps: Vec<Step>, stall: Duration) -> (Self, Arc<Notify>) {
        let asked = Arc::new(Notify::new());
        (
            Self {
                label,
                steps: Mutex::new(steps.into()),
                prompts: Mutex::new(Vec::new()),
                stall,
                stalled_once: AtomicBool::new(false),
                asked: Arc::clone(&asked),
            },
            asked,
        )
    }

    pub fn prompt_sizes(&self) -> Vec<usize> {
        self.prompts.lock().unwrap().clone()
    }
}

#[async_trait::async_trait]
impl LlmProvider for ScriptLlm {
    fn adapter(&self) -> &'static str {
        "script-llm"
    }

    async fn stream(&self, request: LlmRequest) -> Result<LlmEventStream, LlmError> {
        self.prompts.lock().unwrap().push(request.messages.len());
        self.asked.notify_one();
        // Exactly one request stalls, and it is the first: a stall on every request would need its
        // own clock, and a test could then no longer say which turn it interrupted.
        if !self.stall.is_zero() && !self.stalled_once.swap(true, Ordering::SeqCst) {
            tokio::time::sleep(self.stall).await;
        }
        let answered_a_tool_round = request
            .messages
            .iter()
            .any(|message| matches!(message, ChatMessage::ToolResult { .. }));
        if answered_a_tool_round {
            return Ok(Box::pin(stream::iter(vec![
                Ok(LlmEvent::TextDelta(format!("{}[continued]", self.label))),
                Ok(LlmEvent::Finished),
            ])));
        }
        match self.steps.lock().unwrap().pop_front() {
            Some(Step::Answer(text)) => Ok(Box::pin(stream::iter(vec![
                Ok(LlmEvent::TextDelta(text.to_owned())),
                Ok(LlmEvent::Finished),
            ]))),
            Some(Step::Call { name, arguments }) => Ok(Box::pin(stream::iter(vec![
                Ok(LlmEvent::ToolCall(ToolCall {
                    id: "call-1".into(),
                    name: name.to_owned(),
                    arguments,
                })),
                Ok(LlmEvent::Finished),
            ]))),
            None => Ok(Box::pin(stream::iter(vec![
                Ok(LlmEvent::TextDelta(format!("{}[idle]", self.label))),
                Ok(LlmEvent::Finished),
            ]))),
        }
    }
}

pub struct FinalAsr;

impl AsrProvider for FinalAsr {
    fn open(&self) -> Result<Box<dyn AsrSession>, AsrError> {
        Ok(Box::new(FinalAsrSession))
    }
}

struct FinalAsrSession;

impl AsrSession for FinalAsrSession {
    fn push_pcm(&mut self, _: &vela_server::audio::PcmF32Mono) -> Result<Vec<AsrEvent>, AsrError> {
        Ok(Vec::new())
    }

    fn finish(&mut self) -> Result<AsrResult, AsrError> {
        Ok(AsrResult::new("utterance"))
    }

    fn cancel(&mut self) {}
}

/// Manual turns never consult VAD; this provider exists so the runtime slot is a real handle.
pub struct SilentVad;

impl VadProvider for SilentVad {
    fn open(&self) -> Result<Box<dyn VadSession>, VadError> {
        Ok(Box::new(SilentVadSession))
    }

    fn adapter(&self) -> &'static str {
        "silent-vad"
    }
}

struct SilentVadSession;

impl VadSession for SilentVadSession {
    fn push(&mut self, input: VadInput) -> Result<VadProbability, VadError> {
        Ok(VadProbability {
            start_sample: input.start_sample,
            end_sample: input.start_sample + 512,
            probability: 0.0,
        })
    }

    fn reset(&mut self) -> Result<(), VadError> {
        Ok(())
    }
}

pub struct ShortTts;

impl TtsProvider for ShortTts {
    fn adapter(&self) -> &'static str {
        "short-tts"
    }

    fn synthesize(&self, _: &str) -> Result<vela_server::audio::PcmF32Mono, TtsError> {
        Ok(vela_server::audio::PcmF32Mono::new(
            vec![0.1; 4_800],
            48_000,
        ))
    }
}

// ---------------------------------------------------------------- harness

pub fn database_url() -> String {
    format!(
        "sqlite://{}",
        std::env::temp_dir()
            .join(format!("vela-transcript-{}.db", uuid::Uuid::new_v4()))
            .display()
    )
}

pub fn config(url: String, history: DatabaseHistoryConfig) -> AppConfig {
    let mut providers = ProvidersConfig::default();
    providers.vad.instances.insert(
        "test".into(),
        VadInstanceConfig::SileroOnnx(SileroOnnxConfig {
            min_speech_ms: 32,
            end_silence_ms: 32,
            ..Default::default()
        }),
    );
    let mut config = AppConfig {
        orchestrator: Default::default(),
        wiki: Default::default(),
        memory: Default::default(),
        speaker_recognition: Default::default(),
        server: ServerConfig {
            bind: "127.0.0.1:0".parse().unwrap(),
            public_ws_url: Url::parse("ws://127.0.0.1:0/voice/v1/").unwrap(),
            hello_timeout_ms: 500,
            trusted_origins: Vec::new(),
        },
        auth: AuthConfig::default(),
        audio: AudioConfig::default(),
        websocket: WebsocketConfig::default(),
        limits: LimitsConfig::default(),
        provider_defaults: ProviderDefaultsConfig {
            vad: "test".into(),
            asr: "test".into(),
            llm: "test".into(),
            tts: "test".into(),
            vision: None,
        },
        providers,
        workers: WorkersConfig::default(),
        deployment: DeploymentConfig::default(),
        runtime: RuntimeConfig::default(),
        provider_runtime: None,
        llm: LlmConfig::default(),
        tts: TtsConfig::default(),
        speech_output: SpeechOutputConfig::default(),
        barge_in: BargeInConfig::default(),
        mcp: McpConfig::default(),
        system_mcp: Default::default(),
        vision: VisionConfig::default(),
        database: DatabaseConfig {
            url,
            busy_timeout_ms: 30_000,
            devices: Default::default(),
            history,
            ..Default::default()
        },
        api: AdminApiConfig {
            enabled: true,
            jwt_secret: SecretString::new("transcript-harness-signing-secret-0123456789".to_owned()),
            ..Default::default()
        },
        shutdown: Default::default(),
        agent: None,
        effective_agent: Default::default(),
    };
    // A marker the archive must never contain: the deployed prompt, not the answer.
    config.effective_agent.prompt_template = format!("{{{{persona}}}} {PROMPT_MARKER}");
    config
}

pub struct Harness {
    pub base: String,
    pub token: String,
    pub url: String,
    pub metrics: Option<Arc<HistoryWriterMetrics>>,
    pub llm: Arc<ScriptLlm>,
    pub task: JoinHandle<()>,
}

pub async fn start(llm: Arc<ScriptLlm>, history: DatabaseHistoryConfig) -> Harness {
    start_with_database(llm, history, 30_000).await
}

/// A server built through the production bootstrap, so the default Admin Password is seeded and
/// `token` is a usable Admin Web Token. Tests that call the Admin API use this; it exposes no
/// history metrics, which the archive-writer tests read through `start`.
pub async fn start_prepared(llm: Arc<ScriptLlm>, history: DatabaseHistoryConfig) -> Harness {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let url = database_url();
    let app_config = config(url.clone(), history);
    let shared_llm: Arc<dyn LlmProvider> = llm.clone();
    let router = vela_server::app::bootstrap_with_providers(
        app_config,
        Arc::new(ProviderSet::with_all(
            Arc::new(SilentVad),
            Arc::new(FinalAsr),
            shared_llm,
            Arc::new(ShortTts),
        )),
    )
    .await
    .unwrap();
    let task = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    let base = format!("http://{address}");
    let token = admin_web_token(&reqwest::Client::new(), &base).await;
    Harness {
        base,
        token,
        url,
        metrics: None,
        llm,
        task,
    }
}

pub async fn start_with_database(
    llm: Arc<ScriptLlm>,
    history: DatabaseHistoryConfig,
    busy_timeout_ms: u64,
) -> Harness {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let url = database_url();
    let mut app_config = config(url.clone(), history);
    app_config.database.busy_timeout_ms = busy_timeout_ms;
    let database = Database::connect(&app_config.database).await.unwrap();
    let shared_llm: Arc<dyn LlmProvider> = llm.clone();
    let state = AppState::from_provider_set_with_database(
        app_config,
        Arc::new(ProviderSet::with_all(
            Arc::new(SilentVad),
            Arc::new(FinalAsr),
            shared_llm,
            Arc::new(ShortTts),
        )),
        Some(database),
    );
    let metrics = state.history_metrics();
    let task = tokio::spawn(async move {
        axum::serve(listener, router_with_state(state))
            .await
            .unwrap()
    });
    Harness {
        base: format!("http://{address}"),
        token: String::new(),
        url,
        metrics,
        llm,
        task,
    }
}

pub async fn seed(url: &str) -> SqlitePool {
    let pool = SqlitePool::connect(url).await.unwrap();
    sqlx::query("INSERT INTO agents (key,name,enabled,created_at,updated_at) VALUES ('agent','Agent',1,1,1)")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO devices (device_id,agent_id,enabled,created_at,updated_at) VALUES ('device',1,1,1,1)")
        .execute(&pool)
        .await
        .unwrap();
    pool
}

pub async fn insert_template(pool: &SqlitePool, key: &str, prompt: &str) -> i64 {
    sqlx::query(
        "INSERT INTO agent_templates (key,name,language,prompt,enabled,created_at,updated_at) \
         VALUES (?, ?, 'vi-VN', ?, 1, 1, 1)",
    )
    .bind(key)
    .bind(key)
    .bind(prompt)
    .execute(pool)
    .await
    .unwrap();
    sqlx::query_scalar("SELECT id FROM agent_templates WHERE key = ?")
        .bind(key)
        .fetch_one(pool)
        .await
        .unwrap()
}

/// One provider instance per kind, reused by both Templates so binding the same runtime twice is
/// not what this file is testing.
pub async fn bind_providers(pool: &SqlitePool, template_id: i64) {
    for kind in ["vad", "asr", "llm", "tts"] {
        let provider_id: i64 = match sqlx::query_scalar(
            "SELECT id FROM providers WHERE key = 'test'",
        )
        .fetch_optional(pool)
        .await
        .unwrap()
        {
            Some(id) => id,
            None => {
                sqlx::query(
                    "INSERT INTO providers (key,name,type,adapter,config_json,enabled,created_at,updated_at) \
                     VALUES ('test', 'test', ?, 'openai', '{}', 1, 1, 1)",
                )
                .bind(kind)
                .execute(pool)
                .await
                .unwrap();
                sqlx::query_scalar("SELECT id FROM providers WHERE key = 'test'")
                    .fetch_one(pool)
                    .await
                    .unwrap()
            }
        };
        sqlx::query(
            "INSERT INTO template_provider_bindings (template_id,provider_type,provider_id,created_at,updated_at) \
             VALUES (?, ?, ?, 1, 1)",
        )
        .bind(template_id)
        .bind(kind)
        .bind(provider_id)
        .execute(pool)
        .await
        .unwrap();
    }
}

/// Two enabled Templates this process can actually run, so a switch is honorable end to end.
pub async fn seed_switchable_pair(pool: &SqlitePool) -> (i64, i64) {
    let primary = insert_template(pool, "primary", TEMPLATE_PROMPT_V1).await;
    bind_providers(pool, primary).await;
    sqlx::query(
        "INSERT INTO agent_template_assignments (agent_id,template_id,is_default,enabled,created_at) \
         VALUES (1, ?, 1, 1, 1)",
    )
    .bind(primary)
    .execute(pool)
    .await
    .unwrap();
    let secondary = insert_template(pool, "sales", TEMPLATE_PROMPT_V2).await;
    bind_providers(pool, secondary).await;
    sqlx::query(
        "INSERT INTO agent_template_assignments (agent_id,template_id,is_default,enabled,created_at) \
         VALUES (1, ?, 0, 1, 1)",
    )
    .bind(secondary)
    .execute(pool)
    .await
    .unwrap();
    (primary, secondary)
}

pub type Socket =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

pub fn request(base: &str) -> tokio_tungstenite::tungstenite::handshake::client::Request {
    let mut request = format!("{}/voice/v1/", base.replacen("http", "ws", 1))
        .into_client_request()
        .unwrap();
    let headers = request.headers_mut();
    headers.insert("Protocol-Version", "1".parse().unwrap());
    headers.insert("Device-Id", "device".parse().unwrap());
    headers.insert("Client-Id", "transcript-test".parse().unwrap());
    request
}

pub fn canonical_opus_packet() -> Vec<u8> {
    let mut encoder = Encoder::new(16_000, Channels::Mono, Application::Voip).unwrap();
    let mut packet = [0; 4_000];
    let bytes = encoder.encode(&[1_000; 960], &mut packet).unwrap();
    packet[..bytes].to_vec()
}

pub async fn admit(base: &str) -> Socket {
    let (mut socket, response) = connect_async(request(base)).await.unwrap();
    assert_eq!(response.status(), StatusCode::SWITCHING_PROTOCOLS);
    socket
        .send(Message::Text(
            serde_json::json!({
                "type": "hello", "version": 1, "transport": "websocket",
                "audio_params": {"format": "opus", "sample_rate": 16000, "channels": 1, "frame_duration": 60}
            })
            .to_string()
            .into(),
        ))
        .await
        .unwrap();
    let hello = timeout(Duration::from_secs(2), socket.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert!(matches!(hello, Message::Text(_)));
    socket
}

/// What one manual turn actually delivered, so a dropped record cannot be confused with a turn
/// that never ran.
pub struct TurnOutcome {
    pub transcript: Option<String>,
    pub answer: String,
    pub audio_frames: usize,
}

pub async fn speak_and_observe(socket: &mut Socket) -> TurnOutcome {
    socket
        .send(Message::Text(
            serde_json::json!({"type": "listen", "state": "start", "mode": "manual"})
                .to_string()
                .into(),
        ))
        .await
        .unwrap();
    socket
        .send(Message::Binary(canonical_opus_packet().into()))
        .await
        .unwrap();
    socket
        .send(Message::Text(
            serde_json::json!({"type": "listen", "state": "stop"})
                .to_string()
                .into(),
        ))
        .await
        .unwrap();
    let mut observed = TurnOutcome {
        transcript: None,
        answer: String::new(),
        audio_frames: 0,
    };
    loop {
        match timeout(Duration::from_secs(5), socket.next()).await {
            Ok(Some(Ok(Message::Text(text)))) => {
                let value: serde_json::Value = serde_json::from_str(&text).unwrap();
                match value["type"].as_str() {
                    Some("stt") => observed.transcript = value["text"].as_str().map(str::to_owned),
                    Some("llm") => {
                        observed.answer = value["text"].as_str().unwrap_or_default().to_owned()
                    }
                    Some("tts") if value["state"] == "sentence_start" => {
                        observed.answer = value["text"].as_str().unwrap_or_default().to_owned()
                    }
                    Some("tts") if value["state"] == "stop" => return observed,
                    _ => {}
                }
            }
            Ok(Some(Ok(Message::Binary(_)))) => observed.audio_frames += 1,
            Ok(Some(Ok(_))) => {}
            Ok(Some(Err(error))) => panic!("WebSocket error: {error}"),
            Ok(None) | Err(_) => panic!("the session closed before answering"),
        }
    }
}

// ---------------------------------------------------------------- archive observation

#[derive(Clone, Debug, FromRow, PartialEq, Eq)]
pub struct Row {
    pub session_id: String,
    pub sequence: i64,
    pub turn_id: Option<String>,
    pub role: String,
    pub text: String,
    pub template_id: Option<i64>,
}

pub async fn rows(pool: &SqlitePool) -> Vec<Row> {
    sqlx::query_as::<_, Row>(
        "SELECT session_id, sequence, turn_id, role, text, template_id FROM history_messages \
         ORDER BY session_id, sequence",
    )
    .fetch_all(pool)
    .await
    .unwrap()
}

/// The archival write is asynchronous by contract, so a test observes the archive the way an
/// operator does: it waits for the archive to settle instead of assuming it already has.
pub async fn await_rows(pool: &SqlitePool, expected: usize) -> Vec<Row> {
    for _ in 0..240 {
        let rows = rows(pool).await;
        if rows.len() == expected {
            return rows;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    panic!(
        "the archive never reached {expected} rows; it holds {:?}",
        rows(pool).await
    );
}

pub async fn await_drops(metrics: &Arc<HistoryWriterMetrics>, expected: u64) {
    for _ in 0..240 {
        if metrics.counters().dropped >= expected {
            return;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    panic!(
        "the writer never dropped {expected} records; it counted {:?}",
        metrics.counters()
    );
}

/// Waits until every record the archive was offered has either been stored or dropped.
pub async fn await_settled(metrics: &Arc<HistoryWriterMetrics>) {
    for _ in 0..240 {
        if metrics.counters().is_settled() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    panic!(
        "the writer never settled; it counted {:?}",
        metrics.counters()
    );
}

pub fn roles_and_texts(rows: &[Row]) -> Vec<(String, String)> {
    rows.iter()
        .map(|row| (row.role.clone(), row.text.clone()))
        .collect()
}
