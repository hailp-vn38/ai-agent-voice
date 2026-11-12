use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

pub use futures_util::{SinkExt, StreamExt, stream};
use opus2::{Application, Channels, Encoder};
use sqlx::SqlitePool;
use tokio::{
    net::TcpListener,
    sync::{Notify, mpsc},
    task::JoinHandle,
    time::timeout,
};
use tokio_tungstenite::{
    connect_async,
    tungstenite::{Message, client::IntoClientRequest, http::StatusCode},
};
use url::Url;
use vela_server::{
    app::AppState,
    config::{
        AppConfig, AudioConfig, AuthConfig, BargeInConfig, DatabaseConfig, DeploymentConfig,
        LimitsConfig, LlmConfig, McpConfig, ProviderDefaultsConfig, ProvidersConfig, RuntimeConfig,
        ServerConfig, SileroOnnxConfig, SpeechOutputConfig, TtsConfig, VadInstanceConfig,
        VisionConfig, WebsocketConfig, WorkersConfig,
    },
    database::Database,
    providers::{
        AsrError, AsrEvent, AsrProvider, AsrResult, AsrSession, LlmError, LlmEvent, LlmProvider,
        ProviderSet, TtsError, TtsProvider, VadError, VadInput, VadProbability, VadProvider,
        VadSession,
        llm::{ChatMessage, LlmEventStream, LlmRequest, ToolCall, ToolDefinition},
    },
    session::{TurnId, WriterOutcomeProbe, WriterTurnOutcome},
    tools::builtin::SWITCH_TEMPLATE_TOOL_NAME,
};

pub const DEFAULT_PROMPT_MARKER: &str = "SERVER-DEFAULT-PROMPT";
pub const TEMPLATE_PROMPT_V1: &str = "TEMPLATE-PROMPT-V1";
pub const TEMPLATE_PROMPT_V2: &str = "TEMPLATE-PROMPT-V2";
pub const TEMPLATE_PROMPT_V3: &str = "TEMPLATE-PROMPT-V3";

/// Echoes the system prompt it was given so the Effective Session Profile's prompt is observable
/// through the public LLM message rather than through an internal side channel.
pub struct EchoLlm {
    pub label: &'static str,
}

impl LlmProvider for EchoLlm {
    fn adapter(&self) -> &'static str {
        "echo-llm"
    }

    fn complete(&self, request: &LlmRequest) -> Result<String, LlmError> {
        Ok(format!("{}[{}]", self.label, system_prompt(request)))
    }
}

/// A Template script that the model requests in order, recording every tool advertisement it was
/// offered.  It answers a turn with `label[system]` once its script is exhausted, so one provider
/// can both drive a switch and prove which prompt the next turn actually ran under.
pub struct SwitchScriptLlm {
    label: &'static str,
    templates: Mutex<Vec<String>>,
    /// Holds the first tool continuation open so a test can interrupt inside the very turn that
    /// requested a switch, before that turn reaches its boundary.
    continuation_stall: Duration,
    stalled_once: AtomicBool,
    offered_tools: Arc<Mutex<Vec<Vec<ToolDefinition>>>>,
}

impl SwitchScriptLlm {
    pub fn new(
        label: &'static str,
        templates: &[&str],
    ) -> (Self, Arc<Mutex<Vec<Vec<ToolDefinition>>>>) {
        Self::with_stall(label, templates, Duration::ZERO)
    }

    pub fn with_stall(
        label: &'static str,
        templates: &[&str],
        continuation_stall: Duration,
    ) -> (Self, Arc<Mutex<Vec<Vec<ToolDefinition>>>>) {
        let offered_tools = Arc::new(Mutex::new(Vec::new()));
        (
            Self {
                label,
                templates: Mutex::new(templates.iter().map(|key| (*key).to_owned()).collect()),
                continuation_stall,
                stalled_once: AtomicBool::new(false),
                offered_tools: Arc::clone(&offered_tools),
            },
            offered_tools,
        )
    }
}

#[async_trait::async_trait]
impl LlmProvider for SwitchScriptLlm {
    fn adapter(&self) -> &'static str {
        "switch-script-llm"
    }

    async fn stream(&self, request: LlmRequest) -> Result<LlmEventStream, LlmError> {
        self.offered_tools
            .lock()
            .unwrap()
            .push(request.tools.clone());
        let script_exhausted = self.templates.lock().unwrap().is_empty();
        // A tool round carries its own result back, so it is the same Conversational Turn and never
        // draws another script entry.
        let answered_tool_round = request
            .messages
            .iter()
            .any(|message| matches!(message, ChatMessage::ToolResult { .. }));
        if !script_exhausted && !answered_tool_round {
            let template = self.templates.lock().unwrap().remove(0);
            return Ok(Box::pin(stream::iter(vec![
                Ok(LlmEvent::ToolCall(ToolCall {
                    id: "switch-1".into(),
                    name: SWITCH_TEMPLATE_TOOL_NAME.into(),
                    arguments: serde_json::json!({"template": template}),
                })),
                Ok(LlmEvent::Finished),
            ])));
        }
        if answered_tool_round
            && !self.continuation_stall.is_zero()
            && !self.stalled_once.swap(true, Ordering::SeqCst)
        {
            tokio::time::sleep(self.continuation_stall).await;
        }
        Ok(Box::pin(stream::iter(vec![
            Ok(LlmEvent::TextDelta(format!(
                "{}[{}]",
                self.label,
                system_prompt(&request)
            ))),
            Ok(LlmEvent::Finished),
        ])))
    }
}

pub fn system_prompt(request: &LlmRequest) -> String {
    request
        .messages
        .iter()
        .find_map(|message| match message {
            ChatMessage::System { content } => Some(content.clone()),
            _ => None,
        })
        .unwrap_or_default()
}

pub struct FinalAsr;

impl AsrProvider for FinalAsr {
    fn open(&self) -> Result<Box<dyn AsrSession>, AsrError> {
        Ok(Box::new(FinalAsrSession))
    }
}

pub struct FinalAsrSession;

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

pub struct SilentVadSession;

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

pub fn database_url() -> String {
    format!(
        "sqlite://{}",
        std::env::temp_dir()
            .join(format!("vela-profile-{}.db", uuid::Uuid::new_v4()))
            .display()
    )
}

pub fn config(address: std::net::SocketAddr, url: String) -> AppConfig {
    let mut providers = ProvidersConfig::default();
    providers.vad.instances.insert(
        "test".into(),
        VadInstanceConfig::SileroOnnx(SileroOnnxConfig {
            min_speech_ms: 32,
            end_silence_ms: 32,
            ..Default::default()
        }),
    );
    AppConfig {
        orchestrator: Default::default(),
        wiki: Default::default(),
        memory: Default::default(),
        speaker_recognition: Default::default(),
        server: ServerConfig {
            bind: address,
            public_ws_url: Url::parse(&format!("ws://{address}/voice/v1/")).unwrap(),
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
            devices: Default::default(),
            ..Default::default()
        },
        api: Default::default(),
        shutdown: Default::default(),
        agent: None,
        effective_agent: Default::default(),
    }
}

/// The deployment's own agent prompt is replaced with a marker so a stored Template prompt and
/// the server default prompt are distinguishable through the public LLM message.
pub fn with_server_default_prompt(mut config: AppConfig) -> AppConfig {
    config.effective_agent.prompt_template = format!("{{{{persona}}}} {DEFAULT_PROMPT_MARKER}");
    config
}

pub async fn start(app_config: AppConfig) -> (String, String, JoinHandle<()>) {
    start_with_state(app_config, |state| state).await
}

/// Registers one more already-loaded LLM runtime under its own instance id, so a stored Template
/// can select a runtime other than the deployment's server default.
pub async fn start_with_template_llm(app_config: AppConfig) -> (String, String, JoinHandle<()>) {
    start_with_state(app_config, |state| {
        state.with_llm_runtime_for_test(
            "db-llm",
            Arc::new(EchoLlm {
                label: "template-llm",
            }),
            1,
            Duration::from_secs(30),
        )
    })
    .await
}

pub async fn start_with_state(
    app_config: AppConfig,
    extend: impl FnOnce(AppState) -> AppState,
) -> (String, String, JoinHandle<()>) {
    start_with_llm(
        app_config,
        Arc::new(EchoLlm {
            label: "default-llm",
        }),
        extend,
    )
    .await
}

/// Serves the scripted LLM as the server default instance so a stored Template can still bind a
/// second, distinguishable runtime through `with_llm_runtime_for_test`.
pub async fn start_with_llm(
    app_config: AppConfig,
    llm: Arc<dyn LlmProvider>,
    extend: impl FnOnce(AppState) -> AppState,
) -> (String, String, JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let url = app_config.database.url.clone();
    let session_db = Database::connect(&config(address, url.clone()).database)
        .await
        .unwrap();
    crate::admin_auth::seed_default_admin_password(session_db.pool()).await;
    let state = AppState::from_provider_set_with_database(
        app_config,
        Arc::new(ProviderSet::with_all(
            Arc::new(SilentVad),
            Arc::new(FinalAsr),
            llm,
            Arc::new(ShortTts),
        )),
        Some(session_db),
    );
    let state = extend(state);
    let task = tokio::spawn(async move {
        axum::serve(listener, vela_server::app::router_with_state(state))
            .await
            .unwrap()
    });
    (format!("http://{address}"), url, task)
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

pub async fn insert_template(pool: &SqlitePool, key: &str, prompt: &str, enabled: bool) -> i64 {
    sqlx::query(
        "INSERT INTO agent_templates (key,name,language,prompt,enabled,created_at,updated_at) \
         VALUES (?, ?, 'vi-VN', ?, ?, 1, 1)",
    )
    .bind(key)
    .bind(key)
    .bind(prompt)
    .bind(i64::from(enabled))
    .execute(pool)
    .await
    .unwrap();
    sqlx::query_scalar("SELECT id FROM agent_templates WHERE key = ?")
        .bind(key)
        .fetch_one(pool)
        .await
        .unwrap()
}

pub async fn assign(pool: &SqlitePool, template_id: i64, is_default: bool, enabled: bool) {
    sqlx::query(
        "INSERT INTO agent_template_assignments (agent_id,template_id,is_default,enabled,created_at) \
         VALUES (1, ?, ?, ?, 1)",
    )
    .bind(template_id)
    .bind(i64::from(is_default))
    .bind(i64::from(enabled))
    .execute(pool)
    .await
    .unwrap();
}

/// Binds explicit provider slots. Omitted kinds inherit their server defaults at admission.
pub async fn bind(pool: &SqlitePool, template_id: i64, kinds: &[(&str, &str)]) {
    for (kind, key) in kinds {
        let provider_id: i64 = match sqlx::query_scalar("SELECT id FROM providers WHERE key = ?")
            .bind(key)
            .fetch_optional(pool)
            .await
            .unwrap()
        {
            Some(id) => id,
            None => {
                sqlx::query(
                    "INSERT INTO providers (key,name,type,adapter,config_json,enabled,created_at,updated_at) \
                     VALUES (?, ?, ?, 'openai', '{}', 1, 1, 1)",
                )
                .bind(key)
                .bind(key)
                .bind(kind)
                .execute(pool)
                .await
                .unwrap();
                sqlx::query_scalar("SELECT id FROM providers WHERE key = ?")
                    .bind(key)
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

pub fn full_bindings(llm: &str) -> Vec<(&'static str, &'static str)> {
    vec![
        ("vad", "test"),
        ("asr", "test"),
        ("tts", "test"),
        ("llm", Box::leak(llm.to_owned().into_boxed_str())),
    ]
}

pub fn request(base: &str) -> tokio_tungstenite::tungstenite::handshake::client::Request {
    let mut request = format!("{}/voice/v1/", base.replacen("http", "ws", 1))
        .into_client_request()
        .unwrap();
    let headers = request.headers_mut();
    headers.insert("Protocol-Version", "1".parse().unwrap());
    headers.insert("Device-Id", "device".parse().unwrap());
    headers.insert("Client-Id", "profile-test".parse().unwrap());
    request
}

pub fn rejected_status(error: tokio_tungstenite::tungstenite::Error) -> StatusCode {
    match error {
        tokio_tungstenite::tungstenite::Error::Http(response) => response.status(),
        other => panic!("expected an HTTP WebSocket rejection, got {other:?}"),
    }
}

pub type Socket =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

pub fn canonical_opus_packet() -> Vec<u8> {
    let mut encoder = Encoder::new(16_000, Channels::Mono, Application::Voip).unwrap();
    let mut packet = [0; 4_000];
    let bytes = encoder.encode(&[1_000; 960], &mut packet).unwrap();
    packet[..bytes].to_vec()
}

pub async fn admit(base: &str) -> Socket {
    admit_with_direct(base, false).await
}

pub async fn admit_with_direct(base: &str, direct_tts: bool) -> Socket {
    admit_device(base, "device", direct_tts).await
}

pub async fn admit_device(base: &str, device_id: &str, direct_tts: bool) -> Socket {
    let mut connection = request(base);
    connection
        .headers_mut()
        .insert("Device-Id", device_id.parse().unwrap());
    let (mut socket, response) = connect_async(connection).await.unwrap();
    assert_eq!(response.status(), StatusCode::SWITCHING_PROTOCOLS);
    socket
        .send(Message::Text(
            serde_json::json!({
                "type": "hello", "version": 1, "transport": "websocket",
                "features": {"direct_tts": direct_tts},
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

/// What one manual turn actually delivered to the client, so a dropped capture frame or a lost
/// turn boundary is observable rather than merely inferred from a missing answer.
pub struct TurnOutcome {
    pub transcript: Option<String>,
    pub answer: String,
    pub playback_started: bool,
    pub audio_frames: usize,
}

/// Runs one manual turn, back-to-back with whatever ran before it, and returns everything the
/// client received up to and including `tts:stop`.
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
        playback_started: false,
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
                    Some("tts") => match value["state"].as_str() {
                        Some("start") => observed.playback_started = true,
                        Some("stop") => {
                            assert!(
                                !observed.answer.is_empty(),
                                "a closed turn must carry its assistant answer"
                            );
                            return observed;
                        }
                        _ => {}
                    },
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

/// Runs one manual turn and returns the protocol-visible assistant answer.
pub async fn speak_once(socket: &mut Socket) -> String {
    speak_and_observe(socket).await.answer
}

pub async fn seed_switchable_pair(pool: &SqlitePool) {
    let primary = insert_template(pool, "primary", TEMPLATE_PROMPT_V1, true).await;
    bind(pool, primary, &full_bindings("test")).await;
    assign(pool, primary, true, true).await;
    let secondary = insert_template(pool, "sales", TEMPLATE_PROMPT_V2, true).await;
    bind(pool, secondary, &full_bindings("db-llm")).await;
    assign(pool, secondary, false, true).await;
}

/// Registers the second already-loaded LLM runtime a stored Template binds.
pub fn with_template_llm(state: AppState) -> AppState {
    state.with_llm_runtime_for_test(
        "db-llm",
        Arc::new(EchoLlm {
            label: "template-llm",
        }),
        1,
        Duration::from_secs(30),
    )
}

pub type OfferedTools = Arc<Mutex<Vec<Vec<ToolDefinition>>>>;

pub async fn start_with_scripted_switch(
    templates: &[&str],
) -> (String, String, JoinHandle<()>, OfferedTools) {
    start_with_stalled_switch(templates, Duration::ZERO).await
}

pub async fn start_with_stalled_switch(
    templates: &[&str],
    continuation_stall: Duration,
) -> (String, String, JoinHandle<()>, OfferedTools) {
    let (llm, offered_tools) =
        SwitchScriptLlm::with_stall("default-llm", templates, continuation_stall);
    let (base, url, task) = start_with_llm(
        with_signing_secret(with_server_default_prompt(config(
            "127.0.0.1:0".parse().unwrap(),
            database_url(),
        ))),
        Arc::new(llm),
        with_template_llm,
    )
    .await;
    (base, url, task, offered_tools)
}

/// Holds the writer at its terminal-outcome boundary and withholds the reported outcome from the
/// actor's periodic drain, so client ingress is the only path left that can apply it.
///
/// That replaces the black-box race with a deterministic sequence: the harness releases the writer,
/// waits for proof the outcome is in the actor's mailbox, and only then delivers the next client
/// message. Production installs no probe, so none of this exists outside a test.
pub struct ControlledBoundary {
    pub at_boundary: mpsc::UnboundedReceiver<WriterTurnOutcome>,
    pub enqueued: mpsc::UnboundedReceiver<WriterTurnOutcome>,
    pub release: Arc<Notify>,
    pub hold: Arc<AtomicBool>,
}

impl ControlledBoundary {
    /// Runs one turn, then stops the writer exactly at that turn's terminal outcome and does not
    /// let it go until the outcome is provably sitting in the actor's mailbox.
    pub async fn run_turn_and_hold_at_its_boundary(&mut self, socket: &mut Socket) -> TurnOutcome {
        let outcome = speak_and_observe(socket).await;
        let reported = self
            .at_boundary
            .recv()
            .await
            .expect("the writer always reaches a terminal boundary");
        assert_eq!(
            reported,
            WriterTurnOutcome::Normal,
            "the scripted turn must close normally for this ordering to mean anything"
        );
        self.release.notify_one();
        assert_eq!(
            self.enqueued.recv().await,
            Some(WriterTurnOutcome::Normal),
            "the outcome must reach the actor's mailbox before any further client message"
        );
        outcome
    }
}

fn with_signing_secret(
    mut config: vela_server::config::AppConfig,
) -> vela_server::config::AppConfig {
    config.api.jwt_secret =
        vela_server::config::SecretString::new("session-profile-switch-signing-secret-0123456789");
    config
}

pub async fn start_controlled_switch(
    templates: &[&str],
) -> (
    String,
    String,
    JoinHandle<()>,
    OfferedTools,
    ControlledBoundary,
) {
    let (llm, offered_tools) = SwitchScriptLlm::new("default-llm", templates);
    let slot: Arc<Mutex<Option<ControlledBoundary>>> = Arc::new(Mutex::new(None));
    let installed = Arc::clone(&slot);
    let (base, url, task) = start_with_llm(
        with_signing_secret(with_server_default_prompt(config(
            "127.0.0.1:0".parse().unwrap(),
            database_url(),
        ))),
        Arc::new(llm),
        move |state| {
            let (at_boundary, at_boundary_rx) = mpsc::unbounded_channel();
            let (enqueued, enqueued_rx) = mpsc::unbounded_channel();
            let release = Arc::new(Notify::new());
            let hold = Arc::new(AtomicBool::new(true));
            let probe = Arc::new(BoundaryProbe {
                at_boundary,
                enqueued,
                release: Arc::clone(&release),
                hold: Arc::clone(&hold),
            });
            *installed.lock().unwrap() = Some(ControlledBoundary {
                at_boundary: at_boundary_rx,
                enqueued: enqueued_rx,
                release,
                hold,
            });
            let mut state = with_template_llm(state).with_writer_outcome_probe(probe);
            let config = Arc::make_mut(&mut state.config);
            config.api.enabled = true;
            config.api.jwt_secret =
                vela_server::config::SecretString::new("session-profile-signing-secret-0123456789");
            state
        },
    )
    .await;
    let boundary = slot
        .lock()
        .unwrap()
        .take()
        .expect("the router state extension installed the boundary probe");
    (base, url, task, offered_tools, boundary)
}

pub struct BoundaryProbe {
    pub at_boundary: mpsc::UnboundedSender<WriterTurnOutcome>,
    pub enqueued: mpsc::UnboundedSender<WriterTurnOutcome>,
    pub release: Arc<Notify>,
    pub hold: Arc<AtomicBool>,
}

#[async_trait::async_trait]
impl WriterOutcomeProbe for BoundaryProbe {
    async fn before_terminal_outcome(&self, _: TurnId, outcome: WriterTurnOutcome) {
        let _ = self.at_boundary.send(outcome.clone());
        self.release.notified().await;
    }

    async fn after_terminal_outcome_reported(&self, _: TurnId, outcome: WriterTurnOutcome) {
        let _ = self.enqueued.send(outcome);
    }

    fn holds_writer_outcomes(&self) -> bool {
        self.hold.load(Ordering::SeqCst)
    }
}
