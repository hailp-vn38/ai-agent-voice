//! The optional Persistent Transcript, proved at the two seams it is allowed to touch: a real
//! WebSocket Voice Session and the authenticated Admin API.
//!
//! The archive is asynchronous by contract, so a test waits for it the way an operator does rather
//! than assuming the write landed with the turn.  Nothing here asserts on an internal counter for
//! something a client can see; the counters are read only for the drop classes a turn cannot show.

use std::{
    collections::VecDeque,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use futures_util::{SinkExt, StreamExt, stream};
use opus2::{Application, Channels, Encoder};
use sqlx::{FromRow, SqlitePool};
use tokio::sync::Notify;
use tokio::{net::TcpListener, task::JoinHandle, time::timeout};
use tokio_tungstenite::{
    connect_async,
    tungstenite::{Message, client::IntoClientRequest, http::StatusCode},
};
use url::Url;
use voice_agent_server::{
    app::{AppState, router_with_state},
    config::{
        AdminApiConfig, AppConfig, AudioConfig, AuthConfig, BargeInConfig, DatabaseConfig,
        DatabaseDevicesConfig, DatabaseHistoryConfig, DeploymentConfig, LimitsConfig, LlmConfig,
        McpConfig, ProviderDefaultsConfig, ProvidersConfig, RuntimeConfig, ServerConfig,
        SileroOnnxConfig, SpeechOutputConfig, TtsConfig, VadInstanceConfig, VisionConfig,
        WebsocketConfig, WorkersConfig,
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

const PROMPT_MARKER: &str = "SERVER-DEFAULT-PROMPT";
const TEMPLATE_PROMPT_V1: &str = "TEMPLATE-PROMPT-V1";
const TEMPLATE_PROMPT_V2: &str = "TEMPLATE-PROMPT-V2";
const GOODBYE_ARGUMENT: &str = "SECRET-GOODBYE-FROM-A-TOOL-ARGUMENT";
const ADMIN_TOKEN: &str = "transcript-admin-token";

// ---------------------------------------------------------------- scripted providers

/// One scripted first round of a Conversational Turn.  A tool round is answered by a continuation
/// of the same turn, so the script is only consumed by a round that has no ToolResult in it.
enum Step {
    Answer(&'static str),
    Call {
        name: &'static str,
        arguments: serde_json::Value,
    },
}

/// Answers from a script and records every request it was handed, so a test can assert what the
/// prompt actually contained rather than that something was sent.
struct ScriptLlm {
    label: &'static str,
    steps: Mutex<VecDeque<Step>>,
    prompts: Mutex<Vec<usize>>,
    /// How long the very first request stalls before answering.
    ///
    /// An interrupt has to be applied by the actor, and a WebSocket send only flushes to the socket,
    /// so a test cannot use "I sent the abort" as proof the actor has seen it.  A stall the interrupt
    /// comfortably lands inside is what makes that ordering a fact; `asked` is what makes the stall
    /// start at a point the test chose.
    stall: Duration,
    stalled_once: AtomicBool,
    asked: Arc<Notify>,
}

impl ScriptLlm {
    fn new(label: &'static str, steps: Vec<Step>) -> Self {
        Self {
            label,
            steps: Mutex::new(steps.into()),
            prompts: Mutex::new(Vec::new()),
            stall: Duration::ZERO,
            stalled_once: AtomicBool::new(false),
            asked: Arc::new(Notify::new()),
        }
    }

    fn stalling(label: &'static str, steps: Vec<Step>, stall: Duration) -> (Self, Arc<Notify>) {
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

    fn prompt_sizes(&self) -> Vec<usize> {
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

struct FinalAsr;

impl AsrProvider for FinalAsr {
    fn open(&self) -> Result<Box<dyn AsrSession>, AsrError> {
        Ok(Box::new(FinalAsrSession))
    }
}

struct FinalAsrSession;

impl AsrSession for FinalAsrSession {
    fn push_pcm(
        &mut self,
        _: &voice_agent_server::audio::PcmF32Mono,
    ) -> Result<Vec<AsrEvent>, AsrError> {
        Ok(Vec::new())
    }

    fn finish(&mut self) -> Result<AsrResult, AsrError> {
        Ok(AsrResult::new("utterance"))
    }

    fn cancel(&mut self) {}
}

/// Manual turns never consult VAD; this provider exists so the runtime slot is a real handle.
struct SilentVad;

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

struct ShortTts;

impl TtsProvider for ShortTts {
    fn adapter(&self) -> &'static str {
        "short-tts"
    }

    fn synthesize(&self, _: &str) -> Result<voice_agent_server::audio::PcmF32Mono, TtsError> {
        Ok(voice_agent_server::audio::PcmF32Mono::new(
            vec![0.1; 4_800],
            48_000,
        ))
    }
}

// ---------------------------------------------------------------- harness

fn database_url() -> String {
    format!(
        "sqlite://{}",
        std::env::temp_dir()
            .join(format!(
                "voice-agent-transcript-{}.db",
                uuid::Uuid::new_v4()
            ))
            .display()
    )
}

fn config(url: String, history: DatabaseHistoryConfig) -> AppConfig {
    let mut providers = ProvidersConfig::default();
    let VadInstanceConfig::SileroOnnx(vad) = providers
        .vad
        .instances
        .entry("test".into())
        .or_insert_with(|| VadInstanceConfig::SileroOnnx(SileroOnnxConfig::default()));
    {
        vad.min_speech_ms = 32;
        vad.end_silence_ms = 32;
    }
    let mut config = AppConfig {
        server: ServerConfig {
            bind: "127.0.0.1:0".parse().unwrap(),
            public_ws_url: Url::parse("ws://127.0.0.1:0/voice/v1/").unwrap(),
            hello_timeout_ms: 500,
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
        vision: VisionConfig::default(),
        database: DatabaseConfig {
            enabled: true,
            url,
            busy_timeout_ms: 30_000,
            devices: DatabaseDevicesConfig {
                admission_enabled: true,
                ..Default::default()
            },
            history,
            ..Default::default()
        },
        api: AdminApiConfig {
            enabled: true,
            admin_token: ADMIN_TOKEN.to_owned(),
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

struct Harness {
    base: String,
    url: String,
    metrics: Option<Arc<HistoryWriterMetrics>>,
    llm: Arc<ScriptLlm>,
    task: JoinHandle<()>,
}

async fn start(llm: Arc<ScriptLlm>, history: DatabaseHistoryConfig) -> Harness {
    start_with_database(llm, history, 30_000).await
}

async fn start_with_database(
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
        url,
        metrics,
        llm,
        task,
    }
}

async fn seed(url: &str) -> SqlitePool {
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

async fn insert_template(pool: &SqlitePool, key: &str, prompt: &str) -> i64 {
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
async fn bind_providers(pool: &SqlitePool, template_id: i64) {
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
async fn seed_switchable_pair(pool: &SqlitePool) -> (i64, i64) {
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

type Socket =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

fn request(base: &str) -> tokio_tungstenite::tungstenite::handshake::client::Request {
    let mut request = format!("{}/voice/v1/", base.replacen("http", "ws", 1))
        .into_client_request()
        .unwrap();
    let headers = request.headers_mut();
    headers.insert("Protocol-Version", "1".parse().unwrap());
    headers.insert("Device-Id", "device".parse().unwrap());
    headers.insert("Client-Id", "transcript-test".parse().unwrap());
    request
}

fn canonical_opus_packet() -> Vec<u8> {
    let mut encoder = Encoder::new(16_000, Channels::Mono, Application::Voip).unwrap();
    let mut packet = [0; 4_000];
    let bytes = encoder.encode(&[1_000; 960], &mut packet).unwrap();
    packet[..bytes].to_vec()
}

async fn admit(base: &str) -> Socket {
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
struct TurnOutcome {
    transcript: Option<String>,
    answer: String,
    audio_frames: usize,
}

async fn speak_and_observe(socket: &mut Socket) -> TurnOutcome {
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
struct Row {
    session_id: String,
    sequence: i64,
    turn_id: Option<String>,
    role: String,
    text: String,
    template_id: Option<i64>,
}

async fn rows(pool: &SqlitePool) -> Vec<Row> {
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
async fn await_rows(pool: &SqlitePool, expected: usize) -> Vec<Row> {
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

async fn await_drops(metrics: &Arc<HistoryWriterMetrics>, expected: u64) {
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
async fn await_settled(metrics: &Arc<HistoryWriterMetrics>) {
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

fn roles_and_texts(rows: &[Row]) -> Vec<(String, String)> {
    rows.iter()
        .map(|row| (row.role.clone(), row.text.clone()))
        .collect()
}

// ---------------------------------------------------------------- the capture itself

#[tokio::test]
async fn capture_is_off_by_default_so_a_turn_archives_nothing() {
    let harness = start(
        Arc::new(ScriptLlm::new(
            "default-llm",
            vec![Step::Answer("hello from the model")],
        )),
        DatabaseHistoryConfig::default(),
    )
    .await;
    let pool = seed(&harness.url).await;
    assert!(
        harness.metrics.is_none(),
        "capture is opt-in, so the default configuration has no archival writer at all — not an \
         idle one, and no queue for a record to reach"
    );
    let mut socket = admit(&harness.base).await;

    let turn = speak_and_observe(&mut socket).await;

    assert_eq!(turn.answer, "hello from the model");
    assert!(
        rows(&pool).await.is_empty(),
        "capture is opt-in, so the default configuration must not enqueue or write anything"
    );
    harness.task.abort();
}

/// Retention is a property of the archive rather than of capture, so it keeps running with capture
/// off — and it keeps running because the archive is still there, not because a writer was left
/// behind to justify it.
#[tokio::test]
async fn capture_off_still_prunes_what_an_earlier_deployment_archived() {
    let url = database_url();
    let app_config = config(url.clone(), DatabaseHistoryConfig::default());
    let database = Database::connect(&app_config.database).await.unwrap();
    let pool = seed(&url).await;
    let now = unix_millis_now();
    let day: i64 = 24 * 60 * 60 * 1_000;
    sqlx::query(
        "INSERT INTO history_messages \
         (session_id, device_id, agent_id, template_id, sequence, turn_id, role, text, created_at) \
         VALUES ('aged', 1, 1, NULL, 1, '1', 'user', 'expired', ?)",
    )
    .bind(now - 40 * day)
    .execute(&pool)
    .await
    .unwrap();
    let shutdown = tokio_util::sync::CancellationToken::new();
    let _cleaner =
        RetentionCleaner::start(&database, 30, Duration::from_millis(50), shutdown.clone());

    await_rows(&pool, 0).await;
    shutdown.cancel();
}

#[tokio::test]
async fn an_enabled_archive_stores_only_the_final_user_text_and_the_delivered_answer() {
    let harness = start(
        Arc::new(ScriptLlm::new(
            "default-llm",
            vec![Step::Answer("the delivered answer")],
        )),
        DatabaseHistoryConfig {
            enabled: true,
            ..Default::default()
        },
    )
    .await;
    let pool = seed(&harness.url).await;
    let mut socket = admit(&harness.base).await;

    let turn = speak_and_observe(&mut socket).await;
    assert_eq!(turn.transcript.as_deref(), Some("utterance"));
    assert_eq!(turn.answer, "the delivered answer");

    let archived = await_rows(&pool, 2).await;
    assert_eq!(
        roles_and_texts(&archived),
        vec![
            ("user".to_owned(), "utterance".to_owned()),
            ("assistant".to_owned(), "the delivered answer".to_owned())
        ],
        "exactly the accepted final user text and the Delivered Assistant Response"
    );
    assert!(
        archived.iter().all(|row| !row.text.contains(PROMPT_MARKER)),
        "the deployed system prompt is never archived"
    );
    assert!(
        archived
            .iter()
            .all(|row| row.turn_id.as_deref() == Some("1")),
        "both records belong to the one turn that produced them"
    );
    assert_eq!(
        archived.iter().map(|row| row.sequence).collect::<Vec<_>>(),
        vec![1, 2],
        "a session numbers its own records, so the archive stays ordered without a query"
    );
    assert!(
        archived[0].session_id == archived[1].session_id,
        "one WebSocket connection is one archive session"
    );
    harness.task.abort();
}

#[tokio::test]
async fn a_turn_that_speaks_a_tool_argument_never_archives_it() {
    let harness = start(
        Arc::new(ScriptLlm::new(
            "default-llm",
            vec![Step::Call {
                name: EXIT_TOOL_NAME,
                arguments: serde_json::json!({"say_goodbye": GOODBYE_ARGUMENT}),
            }],
        )),
        DatabaseHistoryConfig {
            enabled: true,
            ..Default::default()
        },
    )
    .await;
    let pool = seed(&harness.url).await;
    let mut socket = admit(&harness.base).await;

    let turn = speak_and_observe(&mut socket).await;
    assert_eq!(
        turn.answer, GOODBYE_ARGUMENT,
        "the tool's own answer really was delivered to the client"
    );

    let archived = await_rows(&pool, 1).await;
    assert_eq!(
        roles_and_texts(&archived),
        vec![("user".to_owned(), "utterance".to_owned())],
        "a delivered tool result is not a Delivered Assistant Response"
    );
    assert!(
        !roles_and_texts(&archived)
            .iter()
            .any(|(_, text)| text.contains(GOODBYE_ARGUMENT)),
        "a tool argument the model chose must never reach the archive"
    );
    harness.task.abort();
}

#[tokio::test]
async fn an_interrupted_turn_archives_its_user_text_and_no_answer() {
    let (llm, asked) = ScriptLlm::stalling(
        "default-llm",
        vec![Step::Answer("answered after the interrupt")],
        Duration::from_millis(750),
    );
    let harness = start(
        Arc::new(llm),
        DatabaseHistoryConfig {
            enabled: true,
            ..Default::default()
        },
    )
    .await;
    let pool = seed(&harness.url).await;
    let mut socket = admit(&harness.base).await;

    // Start a turn and interrupt it while the model is stalled on its first request: the turn is
    // provably past its user text and short of its answer, and the stall is long enough that the
    // abort is applied long before the answer could have been delivered.
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
    asked.notified().await;
    tokio::time::sleep(Duration::from_millis(150)).await;
    socket
        .send(Message::Text(
            serde_json::json!({"type": "abort"}).to_string().into(),
        ))
        .await
        .unwrap();
    // The stall expires on its own; the answer it releases is discarded by the interrupted turn, so
    // the test waits for the archive rather than for a client-visible outcome that never comes.

    let archived = await_rows(&pool, 1).await;
    assert_eq!(
        roles_and_texts(&archived),
        vec![("user".to_owned(), "utterance".to_owned())],
        "a partial exchange is accepted: the user text that was accepted stays, the answer that \
         was never delivered does not exist"
    );

    // The same session keeps working, and its next exchange is archived under its own numbers.
    let resumed = speak_and_observe(&mut socket).await;
    assert_eq!(resumed.answer, "answered after the interrupt");
    let resumed = await_rows(&pool, 3).await;
    assert_eq!(
        resumed.iter().map(|row| row.sequence).collect::<Vec<_>>(),
        vec![1, 2, 3],
        "a dropped or absent answer leaves no hole in the session's own numbering"
    );
    harness.task.abort();
}

#[tokio::test]
async fn a_switched_template_is_attributed_to_the_turn_that_ran_under_it() {
    let harness = start(
        Arc::new(ScriptLlm::new(
            "default-llm",
            vec![Step::Call {
                name: voice_agent_server::tools::builtin::SWITCH_TEMPLATE_TOOL_NAME,
                arguments: serde_json::json!({"template": "sales"}),
            }],
        )),
        DatabaseHistoryConfig {
            enabled: true,
            ..Default::default()
        },
    )
    .await;
    let pool = seed(&harness.url).await;
    let (primary, secondary) = seed_switchable_pair(&pool).await;
    let mut socket = admit(&harness.base).await;

    let switching = speak_and_observe(&mut socket).await;
    assert_eq!(switching.answer, "default-llm[continued]");
    let switched = speak_and_observe(&mut socket).await;
    assert_eq!(switched.answer, "default-llm[continued]");

    let archived = await_rows(&pool, 4).await;
    assert_eq!(
        archived
            .iter()
            .map(|row| (row.turn_id.as_deref(), row.template_id))
            .collect::<Vec<_>>(),
        vec![
            (Some("1"), Some(primary)),
            (Some("1"), Some(primary)),
            (Some("2"), Some(secondary)),
            (Some("2"), Some(secondary)),
        ],
        "a switch armed but not yet applied leaves the switching turn attributed to the Template \
         it ran on, and the next turn to the one it ran under"
    );
    assert_eq!(
        archived
            .iter()
            .map(|row| row.text.as_str())
            .collect::<Vec<_>>(),
        vec![
            "utterance",
            "default-llm[continued]",
            "utterance",
            "default-llm[continued]"
        ],
        "the switching turn ran a tool round, and neither the call's arguments nor its result \
         appears among its two archived texts"
    );
    harness.task.abort();
}

#[tokio::test]
async fn a_server_default_session_archives_without_a_template() {
    let harness = start(
        Arc::new(ScriptLlm::new(
            "default-llm",
            vec![Step::Answer("no template here")],
        )),
        DatabaseHistoryConfig {
            enabled: true,
            ..Default::default()
        },
    )
    .await;
    let pool = seed(&harness.url).await;
    let mut socket = admit(&harness.base).await;

    speak_and_observe(&mut socket).await;

    let archived = await_rows(&pool, 2).await;
    assert!(
        archived.iter().all(|row| row.template_id.is_none()),
        "an Agent with no assignment archives with no Template rather than a fabricated one"
    );
    harness.task.abort();
}

// ---------------------------------------------------------------- the drop policy

/// Holds the database's single write lock, so the archival writer cannot finish a record and the
/// bounded hand-off is what a record actually meets.
async fn lock_writes(pool: &SqlitePool) -> sqlx::pool::PoolConnection<sqlx::Sqlite> {
    let mut connection = pool.acquire().await.unwrap();
    sqlx::query("BEGIN EXCLUSIVE")
        .execute(&mut *connection)
        .await
        .unwrap();
    connection
}

#[tokio::test]
async fn a_full_archive_queue_drops_one_record_and_never_the_turn() {
    let harness = start_with_database(
        Arc::new(ScriptLlm::new(
            "default-llm",
            vec![Step::Answer("first answer"), Step::Answer("second answer")],
        )),
        DatabaseHistoryConfig {
            enabled: true,
            queue_capacity: 1,
            ..Default::default()
        },
        30_000,
    )
    .await;
    let pool = seed(&harness.url).await;
    let metrics = harness.metrics.clone().expect("a database owns an archive");
    let mut socket = admit(&harness.base).await;
    let mut write_lock = lock_writes(&pool).await;

    // The writer is stuck on its first record, so the single queue slot is all the room the
    // archive has: the rest of these turns' records have nowhere to go but be dropped.
    let first = speak_and_observe(&mut socket).await;
    let second = speak_and_observe(&mut socket).await;

    assert_eq!(first.answer, "first answer");
    assert_eq!(second.answer, "second answer");
    assert!(
        first.audio_frames > 0 && second.audio_frames > 0,
        "a blocked archive must not cost the client a single audio frame"
    );
    await_drops(&metrics, 1).await;
    assert!(
        metrics.counters().dropped_queue_full >= 1,
        "the record that met a full queue is counted as such, not as a database failure"
    );

    sqlx::query("ROLLBACK")
        .execute(&mut *write_lock)
        .await
        .unwrap();
    await_settled(&metrics).await;
    let counters = metrics.counters();
    let archived = rows(&pool).await;
    assert_eq!(
        archived.len() as u64,
        counters.written,
        "the archive holds exactly what the writer stored"
    );
    assert_eq!(
        counters.enqueued + counters.dropped_queue_full,
        4,
        "four records were produced, every one either reached the hand-off or was refused by it"
    );
    assert_eq!(
        counters.dropped_database, 0,
        "a queue that refused a record is not a database failure"
    );
    assert!(
        archived
            .iter()
            .all(|row| row.text == "utterance" || row.text.ends_with("answer")),
        "a dropped record leaves a gap in the archive; it never leaves a different text in it"
    );
    harness.task.abort();
}

#[tokio::test]
async fn a_database_that_refuses_a_record_drops_it_and_never_the_turn() {
    let harness = start(
        Arc::new(ScriptLlm::new(
            "default-llm",
            vec![Step::Answer("first"), Step::Answer("second")],
        )),
        DatabaseHistoryConfig {
            enabled: true,
            ..Default::default()
        },
    )
    .await;
    let pool = seed(&harness.url).await;
    let metrics = harness.metrics.clone().expect("a database owns an archive");
    let mut socket = admit(&harness.base).await;

    speak_and_observe(&mut socket).await;
    let _ = await_rows(&pool, 2).await;

    // An archive row belongs to its Device and Agent by foreign key, so removing the Device
    // leaves the writer with a record the archive cannot possibly hold.
    sqlx::query("DELETE FROM devices")
        .execute(&pool)
        .await
        .unwrap();
    let answered = speak_and_observe(&mut socket).await;

    assert_eq!(
        answered.answer, "second",
        "a turn the archive cannot record is still answered in full"
    );
    assert!(answered.audio_frames > 0);
    await_drops(&metrics, 2).await;
    assert_eq!(
        metrics.counters().dropped_database,
        2,
        "both records of the unarchivable turn are counted as database failures"
    );
    harness.task.abort();
}

// ---------------------------------------------------------------- retention

#[tokio::test]
async fn retention_runs_at_startup_and_again_on_its_schedule() {
    let url = database_url();
    let app_config = config(
        url.clone(),
        DatabaseHistoryConfig {
            retention_days: 30,
            ..Default::default()
        },
    );
    let database = Database::connect(&app_config.database).await.unwrap();
    let pool = seed(&url).await;
    let now = unix_millis_now();
    let day: i64 = 24 * 60 * 60 * 1_000;
    let insert = |sequence: i64, created_at: i64, text: &str| {
        let pool = pool.clone();
        let text = text.to_owned();
        async move {
            sqlx::query(
                "INSERT INTO history_messages \
                 (session_id, device_id, agent_id, template_id, sequence, turn_id, role, text, created_at) \
                 VALUES ('aged', 1, 1, NULL, ?, '1', 'user', ?, ?)",
            )
            .bind(sequence)
            .bind(text)
            .bind(created_at)
            .execute(&pool)
            .await
            .unwrap();
        }
    };
    insert(1, now - 40 * day, "expired before the first run").await;
    insert(2, now, "kept by the first run").await;
    let shutdown = tokio_util::sync::CancellationToken::new();
    let _retention =
        RetentionCleaner::start(&database, 30, Duration::from_millis(50), shutdown.clone());

    await_rows(&pool, 1).await;
    assert_eq!(
        roles_and_texts(&rows(&pool).await),
        vec![("user".to_owned(), "kept by the first run".to_owned())],
        "a record inside the retention window survives the startup run"
    );

    // A second expired record can only disappear if the schedule fired again.
    insert(3, now - 31 * day, "expired after the first run").await;
    let mut remaining = Vec::new();
    for _ in 0..240 {
        remaining = rows(&pool).await;
        if remaining.len() == 1 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    assert_eq!(
        roles_and_texts(&remaining),
        vec![("user".to_owned(), "kept by the first run".to_owned())],
        "retention keeps deleting on its schedule, not only once at startup"
    );
    shutdown.cancel();
}

#[tokio::test]
async fn a_contended_retention_run_is_abandoned_and_the_next_one_still_happens() {
    let url = database_url();
    let app_config = config(
        url.clone(),
        DatabaseHistoryConfig {
            retention_days: 30,
            ..Default::default()
        },
    );
    let database = Database::connect(&app_config.database).await.unwrap();
    let pool = seed(&url).await;
    let now = unix_millis_now();
    let day: i64 = 24 * 60 * 60 * 1_000;
    sqlx::query(
        "INSERT INTO history_messages \
         (session_id, device_id, agent_id, template_id, sequence, turn_id, role, text, created_at) \
         VALUES ('aged', 1, 1, NULL, 1, '1', 'user', 'expired', ?)",
    )
    .bind(now - 40 * day)
    .execute(&pool)
    .await
    .unwrap();

    // A separate owner holds the write lock for longer than the busy timeout, so the startup run
    // is contended and abandoned rather than waited on or retried.
    let blocking = SqlitePool::connect(&url).await.unwrap();
    let mut connection = blocking.acquire().await.unwrap();
    sqlx::query("BEGIN EXCLUSIVE")
        .execute(&mut *connection)
        .await
        .unwrap();
    let shutdown = tokio_util::sync::CancellationToken::new();
    let _retention =
        RetentionCleaner::start(&database, 30, Duration::from_millis(50), shutdown.clone());
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(
        rows(&pool).await.len(),
        1,
        "a contended run must not hold the lock the Voice Sessions' own writes need"
    );
    sqlx::query("ROLLBACK")
        .execute(&mut *connection)
        .await
        .unwrap();
    drop(connection);
    drop(blocking);

    let mut remaining = Vec::new();
    for _ in 0..240 {
        remaining = rows(&pool).await;
        if remaining.is_empty() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    assert!(
        remaining.is_empty(),
        "the abandoned run is not retried in a loop; the next scheduled run does the work"
    );
    shutdown.cancel();
}

// ---------------------------------------------------------------- the Admin API

fn admin(base: &str, path: &str) -> String {
    format!("{base}/api/admin{path}")
}

async fn purge_body(base: &str, body: serde_json::Value) -> reqwest::Response {
    reqwest::Client::new()
        .post(admin(base, "/history/purge"))
        .bearer_auth(ADMIN_TOKEN)
        .json(&body)
        .send()
        .await
        .unwrap()
}

#[tokio::test]
async fn the_archive_is_readable_and_prunable_through_the_authenticated_admin_api() {
    let harness = start(
        Arc::new(ScriptLlm::new(
            "default-llm",
            vec![Step::Answer("the delivered answer")],
        )),
        DatabaseHistoryConfig {
            enabled: true,
            ..Default::default()
        },
    )
    .await;
    let pool = seed(&harness.url).await;
    let mut socket = admit(&harness.base).await;
    speak_and_observe(&mut socket).await;
    let archived = await_rows(&pool, 2).await;
    let session_id = archived[0].session_id.clone();
    let client = reqwest::Client::new();

    let anonymous = client
        .get(admin(&harness.base, "/history"))
        .send()
        .await
        .unwrap();
    assert_eq!(
        anonymous.status(),
        StatusCode::UNAUTHORIZED,
        "the archive is readable only through the separately authenticated Admin API"
    );
    let anonymous_purge = client
        .post(admin(&harness.base, "/history/purge"))
        .json(&serde_json::json!({"all": "PURGE_ALL_HISTORY"}))
        .send()
        .await
        .unwrap();
    assert_eq!(
        anonymous_purge.status(),
        StatusCode::UNAUTHORIZED,
        "the one destructive operation is behind the same credential as the read"
    );

    let listed: serde_json::Value = client
        .get(admin(&harness.base, "/history"))
        .bearer_auth(ADMIN_TOKEN)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(listed["items"].as_array().unwrap().len(), 2);
    assert_eq!(listed["max_page_size"], 200);

    for query in [
        ("role=system", StatusCode::BAD_REQUEST),
        ("device_id=0", StatusCode::BAD_REQUEST),
        ("device_id=1; DROP TABLE agents", StatusCode::BAD_REQUEST),
        ("sort=text", StatusCode::BAD_REQUEST),
        ("page_size=1000", StatusCode::BAD_REQUEST),
    ] {
        let response = client
            .get(admin(&harness.base, &format!("/history?{}", query.0)))
            .bearer_auth(ADMIN_TOKEN)
            .send()
            .await
            .unwrap();
        assert_eq!(
            response.status(),
            query.1,
            "`{}` must be rejected rather than answered with something else",
            query.0
        );
    }
    let users_only: serde_json::Value = client
        .get(admin(
            &harness.base,
            &format!("/history?session_id={session_id}&role=user"),
        ))
        .bearer_auth(ADMIN_TOKEN)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(
        users_only["items"].as_array().unwrap().len(),
        1,
        "typed filters narrow the archive without any raw expression reaching SQL"
    );

    for (body, code) in [
        (serde_json::json!({}), "invalid_purge_scope"),
        (
            serde_json::json!({"session_id": session_id, "device_id": 1}),
            "invalid_purge_scope",
        ),
        (serde_json::json!({"all": "all"}), "confirmation_required"),
        (
            serde_json::json!({"all": "all", "confirm": "yes"}),
            "confirmation_required",
        ),
        (
            serde_json::json!({"all": "everything"}),
            "invalid_purge_scope",
        ),
        (
            serde_json::json!({"session_id": session_id, "confirm": "PURGE_ALL_HISTORY"}),
            "invalid_purge_scope",
        ),
        (serde_json::json!({"device_id": 0}), "invalid_purge_scope"),
        (
            serde_json::json!({
                "all": "all",
                "confirm": "PURGE_ALL_HISTORY",
                "session_id": session_id
            }),
            "invalid_purge_scope",
        ),
        (
            serde_json::json!({"session_id": session_id, "agent_id": 1}),
            "invalid_json",
        ),
    ] {
        let response = purge_body(&harness.base, body.clone()).await;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST, "body {body}");
        assert_eq!(
            response.json::<serde_json::Value>().await.unwrap()["error"]["code"],
            code
        );
    }
    assert_eq!(
        rows(&pool).await.len(),
        2,
        "no rejected request deletes anything"
    );

    let purged = purge_body(&harness.base, serde_json::json!({"session_id": session_id}))
        .await
        .json::<serde_json::Value>()
        .await
        .unwrap();
    assert_eq!(purged["deleted"], 2);
    assert!(rows(&pool).await.is_empty());
    let audit: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM admin_audit_events WHERE resource_type = 'history' AND action = 'purge'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(audit, 1, "a purge leaves exactly one countable trace");
    harness.task.abort();
}

#[tokio::test]
async fn a_purge_never_reaches_an_open_sessions_dialogue_history() {
    let llm = Arc::new(ScriptLlm::new(
        "default-llm",
        vec![Step::Answer("first answer"), Step::Answer("second answer")],
    ));
    let harness = start(
        llm.clone(),
        DatabaseHistoryConfig {
            enabled: true,
            ..Default::default()
        },
    )
    .await;
    let pool = seed(&harness.url).await;
    let mut socket = admit(&harness.base).await;
    speak_and_observe(&mut socket).await;
    let archived = await_rows(&pool, 2).await;
    let session_id = archived[0].session_id.clone();

    let purged: serde_json::Value =
        purge_body(&harness.base, serde_json::json!({"session_id": session_id}))
            .await
            .json()
            .await
            .unwrap();
    assert_eq!(purged["deleted"], 2);
    assert!(rows(&pool).await.is_empty());

    let second = speak_and_observe(&mut socket).await;
    assert_eq!(second.answer, "second answer");
    assert_eq!(
        harness.llm.prompt_sizes(),
        vec![2, 4],
        "the archive is empty and the session's prompt is not: the second turn was composed from \
         the system prompt, the first exchange and its own new user text, so the purge left \
         Dialogue History alone"
    );
    let after = await_rows(&pool, 2).await;
    assert_eq!(
        roles_and_texts(&after),
        vec![
            ("user".to_owned(), "utterance".to_owned()),
            ("assistant".to_owned(), "second answer".to_owned())
        ],
        "an open session keeps archiving into a fresh sequence after its earlier records were \
         purged"
    );
    assert_eq!(
        after.iter().map(|row| row.sequence).collect::<Vec<_>>(),
        vec![3, 4],
        "the session's own numbering is its own: a purge emptied the archive without renumbering \
         the session that owns it"
    );
    harness.task.abort();
}

#[tokio::test]
async fn the_archive_stays_readable_and_prunable_while_capture_is_off() {
    let harness = start(
        Arc::new(ScriptLlm::new("default-llm", Vec::new())),
        DatabaseHistoryConfig::default(),
    )
    .await;
    let pool = seed(&harness.url).await;
    sqlx::query(
        "INSERT INTO history_messages \
         (session_id, device_id, agent_id, template_id, sequence, turn_id, role, text, created_at) \
         VALUES ('archived-by-an-earlier-deployment', 1, 1, NULL, 1, '1', 'user', 'older', ?)",
    )
    .bind(unix_millis_now())
    .execute(&pool)
    .await
    .unwrap();
    let client = reqwest::Client::new();

    let mut socket = admit(&harness.base).await;
    speak_and_observe(&mut socket).await;

    let listed: serde_json::Value = client
        .get(admin(&harness.base, "/history?role=user"))
        .bearer_auth(ADMIN_TOKEN)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(
        listed["items"].as_array().unwrap().len(),
        1,
        "capture being off stops new records; it never hides the archive an operator already has"
    );
    assert_eq!(listed["items"][0]["text"], "older");
    let purged: serde_json::Value = purge_body(
        &harness.base,
        serde_json::json!({"session_id": "archived-by-an-earlier-deployment"}),
    )
    .await
    .json()
    .await
    .unwrap();
    assert_eq!(purged["deleted"], 1);
    harness.task.abort();
}

#[tokio::test]
async fn a_session_admitted_without_the_database_never_archives_anything() {
    let url = database_url();
    let mut app_config = config(
        url.clone(),
        DatabaseHistoryConfig {
            enabled: true,
            ..Default::default()
        },
    );
    app_config.database.devices.admission_enabled = false;
    let database = Database::connect(&app_config.database).await.unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    app_config.server.bind = address;
    let state = AppState::from_provider_set_with_database(
        app_config,
        Arc::new(ProviderSet::with_all(
            Arc::new(SilentVad),
            Arc::new(FinalAsr),
            Arc::new(ScriptLlm::new(
                "default-llm",
                vec![Step::Answer("no admission identity")],
            )),
            Arc::new(ShortTts),
        )),
        Some(database),
    );
    let task = tokio::spawn(async move {
        axum::serve(listener, router_with_state(state))
            .await
            .unwrap()
    });
    let pool = SqlitePool::connect(&url).await.unwrap();

    let mut socket = admit(&format!("http://{address}")).await;
    let turn = speak_and_observe(&mut socket).await;
    assert_eq!(turn.answer, "no admission identity");

    assert!(
        rows(&pool).await.is_empty(),
        "an archive row belongs to an admitted Device and Agent, and this session has neither"
    );
    task.abort();
}
