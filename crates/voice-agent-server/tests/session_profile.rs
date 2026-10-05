use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use futures_util::{SinkExt, StreamExt, stream};
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
use voice_agent_server::{
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

const DEFAULT_PROMPT_MARKER: &str = "SERVER-DEFAULT-PROMPT";
const TEMPLATE_PROMPT_V1: &str = "TEMPLATE-PROMPT-V1";
const TEMPLATE_PROMPT_V2: &str = "TEMPLATE-PROMPT-V2";
const TEMPLATE_PROMPT_V3: &str = "TEMPLATE-PROMPT-V3";

/// Echoes the system prompt it was given so the Effective Session Profile's prompt is observable
/// through the public LLM message rather than through an internal side channel.
struct EchoLlm {
    label: &'static str,
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
struct SwitchScriptLlm {
    label: &'static str,
    templates: Mutex<Vec<String>>,
    /// Holds the first tool continuation open so a test can interrupt inside the very turn that
    /// requested a switch, before that turn reaches its boundary.
    continuation_stall: Duration,
    stalled_once: AtomicBool,
    offered_tools: Arc<Mutex<Vec<Vec<ToolDefinition>>>>,
}

impl SwitchScriptLlm {
    fn new(
        label: &'static str,
        templates: &[&str],
    ) -> (Self, Arc<Mutex<Vec<Vec<ToolDefinition>>>>) {
        Self::with_stall(label, templates, Duration::ZERO)
    }

    fn with_stall(
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

fn system_prompt(request: &LlmRequest) -> String {
    request
        .messages
        .iter()
        .find_map(|message| match message {
            ChatMessage::System { content } => Some(content.clone()),
            _ => None,
        })
        .unwrap_or_default()
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

fn database_url() -> String {
    format!(
        "sqlite://{}",
        std::env::temp_dir()
            .join(format!("voice-agent-profile-{}.db", uuid::Uuid::new_v4()))
            .display()
    )
}

fn config(address: std::net::SocketAddr, url: String) -> AppConfig {
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
    AppConfig {
        server: ServerConfig {
            bind: address,
            public_ws_url: Url::parse(&format!("ws://{address}/voice/v1/")).unwrap(),
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
fn with_server_default_prompt(mut config: AppConfig) -> AppConfig {
    config.effective_agent.prompt_template = format!("{{{{persona}}}} {DEFAULT_PROMPT_MARKER}");
    config
}

async fn start(app_config: AppConfig) -> (String, String, JoinHandle<()>) {
    start_with_state(app_config, |state| state).await
}

/// Registers one more already-loaded LLM runtime under its own instance id, so a stored Template
/// can select a runtime other than the deployment's server default.
async fn start_with_template_llm(app_config: AppConfig) -> (String, String, JoinHandle<()>) {
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

async fn start_with_state(
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
async fn start_with_llm(
    app_config: AppConfig,
    llm: Arc<dyn LlmProvider>,
    extend: impl FnOnce(AppState) -> AppState,
) -> (String, String, JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let url = app_config.database.url.clone();
    let state = AppState::from_provider_set_with_database(
        app_config,
        Arc::new(ProviderSet::with_all(
            Arc::new(SilentVad),
            Arc::new(FinalAsr),
            llm,
            Arc::new(ShortTts),
        )),
        Some(
            Database::connect(&config(address, url.clone()).database)
                .await
                .unwrap(),
        ),
    );
    let state = extend(state);
    let task = tokio::spawn(async move {
        axum::serve(listener, voice_agent_server::app::router_with_state(state))
            .await
            .unwrap()
    });
    (format!("http://{address}"), url, task)
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

async fn insert_template(pool: &SqlitePool, key: &str, prompt: &str, enabled: bool) -> i64 {
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

async fn assign(pool: &SqlitePool, template_id: i64, is_default: bool, enabled: bool) {
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
async fn bind(pool: &SqlitePool, template_id: i64, kinds: &[(&str, &str)]) {
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

fn full_bindings(llm: &str) -> Vec<(&'static str, &'static str)> {
    vec![
        ("vad", "test"),
        ("asr", "test"),
        ("tts", "test"),
        ("llm", Box::leak(llm.to_owned().into_boxed_str())),
    ]
}

fn request(base: &str) -> tokio_tungstenite::tungstenite::handshake::client::Request {
    let mut request = format!("{}/voice/v1/", base.replacen("http", "ws", 1))
        .into_client_request()
        .unwrap();
    let headers = request.headers_mut();
    headers.insert("Protocol-Version", "1".parse().unwrap());
    headers.insert("Device-Id", "device".parse().unwrap());
    headers.insert("Client-Id", "profile-test".parse().unwrap());
    request
}

fn rejected_status(error: tokio_tungstenite::tungstenite::Error) -> StatusCode {
    match error {
        tokio_tungstenite::tungstenite::Error::Http(response) => response.status(),
        other => panic!("expected an HTTP WebSocket rejection, got {other:?}"),
    }
}

type Socket =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

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

/// What one manual turn actually delivered to the client, so a dropped capture frame or a lost
/// turn boundary is observable rather than merely inferred from a missing answer.
struct TurnOutcome {
    transcript: Option<String>,
    answer: String,
    playback_started: bool,
    audio_frames: usize,
}

/// Runs one manual turn, back-to-back with whatever ran before it, and returns everything the
/// client received up to and including `tts:stop`.
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
async fn speak_once(socket: &mut Socket) -> String {
    speak_and_observe(socket).await.answer
}

#[tokio::test]
async fn an_agent_without_any_assignment_row_uses_server_defaults() {
    let (base, url, task) = start(with_server_default_prompt(config(
        "127.0.0.1:0".parse().unwrap(),
        database_url(),
    )))
    .await;
    let _pool = seed(&url).await;
    let mut socket = admit(&base).await;
    assert!(
        speak_once(&mut socket)
            .await
            .contains(DEFAULT_PROMPT_MARKER),
        "an unassigned agent keeps the deployment prompt"
    );
    task.abort();
}

#[tokio::test]
async fn an_agent_with_only_a_disabled_assignment_uses_server_defaults() {
    let (base, url, task) = start(with_server_default_prompt(config(
        "127.0.0.1:0".parse().unwrap(),
        database_url(),
    )))
    .await;
    let pool = seed(&url).await;
    let template = insert_template(&pool, "primary", TEMPLATE_PROMPT_V1, true).await;
    bind(&pool, template, &full_bindings("test")).await;
    assign(&pool, template, true, false).await;

    let mut socket = admit(&base).await;
    assert!(
        speak_once(&mut socket)
            .await
            .contains(DEFAULT_PROMPT_MARKER),
        "a soft-unlinked assignment does not block the deployment profile"
    );
    task.abort();
}

#[tokio::test]
async fn an_agent_without_an_enabled_default_assignment_returns_coarse_503() {
    let (base, url, task) = start(with_server_default_prompt(config(
        "127.0.0.1:0".parse().unwrap(),
        database_url(),
    )))
    .await;
    let pool = seed(&url).await;
    let template = insert_template(&pool, "secondary", TEMPLATE_PROMPT_V1, true).await;
    bind(&pool, template, &full_bindings("test")).await;
    assign(&pool, template, false, true).await;

    let error = connect_async(request(&base)).await.unwrap_err();
    assert_eq!(rejected_status(error), StatusCode::SERVICE_UNAVAILABLE);
    task.abort();
}

#[tokio::test]
async fn a_disabled_default_template_fails_closed() {
    let (base, url, task) = start(with_server_default_prompt(config(
        "127.0.0.1:0".parse().unwrap(),
        database_url(),
    )))
    .await;
    let pool = seed(&url).await;
    let template = insert_template(&pool, "primary", TEMPLATE_PROMPT_V1, false).await;
    bind(&pool, template, &full_bindings("test")).await;
    assign(&pool, template, true, true).await;

    let error = connect_async(request(&base)).await.unwrap_err();
    assert_eq!(rejected_status(error), StatusCode::SERVICE_UNAVAILABLE);
    task.abort();
}

#[tokio::test]
async fn a_default_template_missing_a_binding_uses_the_server_default_for_that_slot() {
    let (base, url, task) = start(with_server_default_prompt(config(
        "127.0.0.1:0".parse().unwrap(),
        database_url(),
    )))
    .await;
    let pool = seed(&url).await;
    let template = insert_template(&pool, "primary", TEMPLATE_PROMPT_V1, true).await;
    bind(
        &pool,
        template,
        &[("vad", "test"), ("asr", "test"), ("llm", "test")],
    )
    .await;
    assign(&pool, template, true, true).await;

    let mut socket = admit(&base).await;
    assert_eq!(
        speak_once(&mut socket).await,
        format!("default-llm[{TEMPLATE_PROMPT_V1}]")
    );
    task.abort();
}

#[tokio::test]
async fn a_default_template_bound_to_an_unloaded_provider_fails_closed() {
    let (base, url, task) = start(with_server_default_prompt(config(
        "127.0.0.1:0".parse().unwrap(),
        database_url(),
    )))
    .await;
    let pool = seed(&url).await;
    let template = insert_template(&pool, "primary", TEMPLATE_PROMPT_V1, true).await;
    bind(
        &pool,
        template,
        &[
            ("vad", "test"),
            ("asr", "test"),
            ("tts", "test"),
            ("llm", "never-loaded"),
        ],
    )
    .await;
    assign(&pool, template, true, true).await;

    let error = connect_async(request(&base)).await.unwrap_err();
    assert_eq!(rejected_status(error), StatusCode::SERVICE_UNAVAILABLE);
    task.abort();
}

#[tokio::test]
async fn a_default_template_bound_to_a_disabled_provider_fails_closed() {
    let (base, url, task) = start(with_server_default_prompt(config(
        "127.0.0.1:0".parse().unwrap(),
        database_url(),
    )))
    .await;
    let pool = seed(&url).await;
    let template = insert_template(&pool, "primary", TEMPLATE_PROMPT_V1, true).await;
    bind(&pool, template, &full_bindings("test")).await;
    assign(&pool, template, true, true).await;
    sqlx::query("UPDATE providers SET enabled = 0 WHERE key = 'test'")
        .execute(&pool)
        .await
        .unwrap();

    let error = connect_async(request(&base)).await.unwrap_err();
    assert_eq!(rejected_status(error), StatusCode::SERVICE_UNAVAILABLE);
    task.abort();
}

#[tokio::test]
async fn database_backed_admission_uses_the_default_template_prompt_and_language() {
    let (base, url, task) = start(with_server_default_prompt(config(
        "127.0.0.1:0".parse().unwrap(),
        database_url(),
    )))
    .await;
    let pool = seed(&url).await;
    let template = insert_template(&pool, "primary", TEMPLATE_PROMPT_V1, true).await;
    bind(&pool, template, &full_bindings("test")).await;
    assign(&pool, template, true, true).await;

    let mut socket = admit(&base).await;
    assert_eq!(
        speak_once(&mut socket).await,
        format!("default-llm[{TEMPLATE_PROMPT_V1}]")
    );
    task.abort();
}

#[tokio::test]
async fn a_default_template_binding_selects_a_loaded_runtime_other_than_the_server_default() {
    let (base, url, task) = start_with_template_llm(with_server_default_prompt(config(
        "127.0.0.1:0".parse().unwrap(),
        database_url(),
    )))
    .await;
    let pool = seed(&url).await;
    let template = insert_template(&pool, "primary", TEMPLATE_PROMPT_V1, true).await;
    bind(&pool, template, &full_bindings("db-llm")).await;
    assign(&pool, template, true, true).await;

    let mut socket = admit(&base).await;
    assert_eq!(
        speak_once(&mut socket).await,
        "template-llm[TEMPLATE-PROMPT-V1]",
        "the stored binding must select the runtime it names, not the server default"
    );
    task.abort();
}

#[tokio::test]
async fn an_admitted_profile_stays_immutable_while_a_new_session_sees_the_mutation() {
    let (base, url, task) = start(with_server_default_prompt(config(
        "127.0.0.1:0".parse().unwrap(),
        database_url(),
    )))
    .await;
    let pool = seed(&url).await;
    let template = insert_template(&pool, "primary", TEMPLATE_PROMPT_V1, true).await;
    bind(&pool, template, &full_bindings("test")).await;
    assign(&pool, template, true, true).await;

    let mut admitted = admit(&base).await;
    assert_eq!(
        speak_once(&mut admitted).await,
        "default-llm[TEMPLATE-PROMPT-V1]"
    );

    sqlx::query("UPDATE agent_templates SET prompt = ? WHERE key = 'primary'")
        .bind(TEMPLATE_PROMPT_V2)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("UPDATE providers SET revision = revision + 1 WHERE key = 'test'")
        .execute(&pool)
        .await
        .unwrap();

    assert_eq!(
        speak_once(&mut admitted).await,
        "default-llm[TEMPLATE-PROMPT-V1]",
        "an admitted session must never be reconfigured by a database mutation"
    );
    let mut reconnected = admit(&base).await;
    assert_eq!(
        speak_once(&mut reconnected).await,
        "default-llm[TEMPLATE-PROMPT-V2]",
        "a new session resolves the mutated desired state"
    );
    task.abort();
}

#[tokio::test]
async fn an_invalid_non_default_candidate_is_excluded_while_a_valid_default_still_starts() {
    let (base, url, task) = start(with_server_default_prompt(config(
        "127.0.0.1:0".parse().unwrap(),
        database_url(),
    )))
    .await;
    let pool = seed(&url).await;
    let default_template = insert_template(&pool, "primary", TEMPLATE_PROMPT_V1, true).await;
    bind(&pool, default_template, &full_bindings("test")).await;
    assign(&pool, default_template, true, true).await;
    // A non-default candidate bound to a provider this process never loaded must be excluded,
    // not allowed to fail the session that already has a valid default.
    let broken = insert_template(&pool, "secondary", TEMPLATE_PROMPT_V2, true).await;
    bind(
        &pool,
        broken,
        &[
            ("vad", "test"),
            ("asr", "test"),
            ("tts", "test"),
            ("llm", "never-loaded"),
        ],
    )
    .await;
    assign(&pool, broken, false, true).await;

    let mut socket = admit(&base).await;
    assert_eq!(
        speak_once(&mut socket).await,
        "default-llm[TEMPLATE-PROMPT-V1]"
    );
    task.abort();
}

#[tokio::test]
async fn a_disabled_device_is_still_denied_before_profile_resolution() {
    let (base, url, task) = start(with_server_default_prompt(config(
        "127.0.0.1:0".parse().unwrap(),
        database_url(),
    )))
    .await;
    let pool = seed(&url).await;
    sqlx::query("UPDATE devices SET enabled = 0")
        .execute(&pool)
        .await
        .unwrap();

    let error = connect_async(request(&base)).await.unwrap_err();
    assert_eq!(rejected_status(error), StatusCode::FORBIDDEN);
    task.abort();
}

#[tokio::test]
async fn a_closed_database_pool_degrades_admission_to_coarse_503() {
    let url = database_url();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let app_config = with_server_default_prompt(config(address, url.clone()));
    let database = Database::connect(&app_config.database).await.unwrap();
    database.pool().close().await;
    let state = AppState::from_provider_set_with_database(
        app_config,
        Arc::new(ProviderSet::with_all(
            Arc::new(SilentVad),
            Arc::new(FinalAsr),
            Arc::new(EchoLlm {
                label: "default-llm",
            }),
            Arc::new(ShortTts),
        )),
        Some(database),
    );
    let task = tokio::spawn(async move {
        axum::serve(listener, voice_agent_server::app::router_with_state(state))
            .await
            .unwrap()
    });
    let error = connect_async(request(&format!("http://{address}")))
        .await
        .unwrap_err();
    assert_eq!(rejected_status(error), StatusCode::SERVICE_UNAVAILABLE);
    task.abort();
}

#[tokio::test]
async fn concurrent_sessions_each_resolve_their_own_immutable_profile() {
    let (base, url, task) = start(with_server_default_prompt(config(
        "127.0.0.1:0".parse().unwrap(),
        database_url(),
    )))
    .await;
    let pool = seed(&url).await;
    sqlx::query("INSERT INTO devices (device_id,agent_id,enabled,created_at,updated_at) VALUES ('second',1,1,1,1)")
        .execute(&pool)
        .await
        .unwrap();
    let template = insert_template(&pool, "primary", TEMPLATE_PROMPT_V1, true).await;
    bind(&pool, template, &full_bindings("test")).await;
    assign(&pool, template, true, true).await;

    let mut first = admit(&base).await;
    let mut second = admit(&base).await;
    let (left, right) = tokio::join!(speak_once(&mut first), speak_once(&mut second));
    assert_eq!(left, "default-llm[TEMPLATE-PROMPT-V1]");
    assert_eq!(right, "default-llm[TEMPLATE-PROMPT-V1]");
    task.abort();
}

/// A switch target whose Providers all resolve, so it belongs to the admission-time catalog.
async fn seed_switchable_pair(pool: &SqlitePool) {
    let primary = insert_template(pool, "primary", TEMPLATE_PROMPT_V1, true).await;
    bind(pool, primary, &full_bindings("test")).await;
    assign(pool, primary, true, true).await;
    let secondary = insert_template(pool, "sales", TEMPLATE_PROMPT_V2, true).await;
    bind(pool, secondary, &full_bindings("db-llm")).await;
    assign(pool, secondary, false, true).await;
}

/// Registers the second already-loaded LLM runtime a stored Template binds.
fn with_template_llm(state: AppState) -> AppState {
    state.with_llm_runtime_for_test(
        "db-llm",
        Arc::new(EchoLlm {
            label: "template-llm",
        }),
        1,
        Duration::from_secs(30),
    )
}

type OfferedTools = Arc<Mutex<Vec<Vec<ToolDefinition>>>>;

async fn start_with_scripted_switch(
    templates: &[&str],
) -> (String, String, JoinHandle<()>, OfferedTools) {
    start_with_stalled_switch(templates, Duration::ZERO).await
}

async fn start_with_stalled_switch(
    templates: &[&str],
    continuation_stall: Duration,
) -> (String, String, JoinHandle<()>, OfferedTools) {
    let (llm, offered_tools) =
        SwitchScriptLlm::with_stall("default-llm", templates, continuation_stall);
    let (base, url, task) = start_with_llm(
        with_server_default_prompt(config("127.0.0.1:0".parse().unwrap(), database_url())),
        Arc::new(llm),
        with_template_llm,
    )
    .await;
    (base, url, task, offered_tools)
}

#[tokio::test]
async fn a_session_without_a_template_assignment_is_never_offered_the_switch_tool() {
    let (llm, offered_tools) = SwitchScriptLlm::new("default-llm", &["primary"]);
    let (base, url, task) = start_with_llm(
        with_server_default_prompt(config("127.0.0.1:0".parse().unwrap(), database_url())),
        Arc::new(llm),
        |state| state,
    )
    .await;
    let _pool = seed(&url).await;

    let mut socket = admit(&base).await;
    assert!(
        speak_once(&mut socket)
            .await
            .contains(DEFAULT_PROMPT_MARKER)
    );
    assert!(
        offered_tools
            .lock()
            .unwrap()
            .iter()
            .flatten()
            .all(|tool| tool.name != SWITCH_TEMPLATE_TOOL_NAME),
        "an Agent with no assignment must not be offered a switch it cannot honor"
    );
    task.abort();
}

#[tokio::test]
async fn the_switch_tool_offers_exactly_the_candidates_admission_validated() {
    let (base, url, task, offered_tools) = start_with_scripted_switch(&[]).await;
    let pool = seed(&url).await;
    seed_switchable_pair(&pool).await;
    // Bound to a provider this process never loaded, so admission must exclude it from the catalog.
    let broken = insert_template(&pool, "broken", TEMPLATE_PROMPT_V3, true).await;
    bind(
        &pool,
        broken,
        &[
            ("vad", "test"),
            ("asr", "test"),
            ("tts", "test"),
            ("llm", "never-loaded"),
        ],
    )
    .await;
    assign(&pool, broken, false, true).await;

    let mut socket = admit(&base).await;
    assert_eq!(
        speak_once(&mut socket).await,
        "default-llm[TEMPLATE-PROMPT-V1]"
    );
    let switch_tool = offered_tools
        .lock()
        .unwrap()
        .first()
        .and_then(|tools| {
            tools
                .iter()
                .find(|tool| tool.name == SWITCH_TEMPLATE_TOOL_NAME)
        })
        .cloned()
        .expect("an Agent with admitted candidates may switch");
    assert_eq!(
        switch_tool.parameters["properties"]["template"]["enum"],
        serde_json::json!(["primary", "sales"]),
        "the model is offered this session's own candidates and nothing else"
    );
    task.abort();
}

#[tokio::test]
async fn a_switch_applies_at_the_next_turn_boundary_on_the_candidates_loaded_runtime() {
    let (base, url, task, _) = start_with_scripted_switch(&["sales"]).await;
    let pool = seed(&url).await;
    seed_switchable_pair(&pool).await;

    let mut socket = admit(&base).await;
    assert_eq!(
        speak_once(&mut socket).await,
        "default-llm[TEMPLATE-PROMPT-V1]",
        "the turn that requested the switch must finish under the profile it started with"
    );
    assert_eq!(
        speak_once(&mut socket).await,
        "template-llm[TEMPLATE-PROMPT-V2]",
        "the next turn must run the switched prompt on the switched already-loaded runtime"
    );
    task.abort();
}

#[tokio::test]
async fn a_switch_uses_the_admission_snapshot_even_after_the_database_changes() {
    let (base, url, task, _) = start_with_scripted_switch(&["sales"]).await;
    let pool = seed(&url).await;
    seed_switchable_pair(&pool).await;
    let mut socket = admit(&base).await;
    assert_eq!(
        speak_once(&mut socket).await,
        "default-llm[TEMPLATE-PROMPT-V1]"
    );

    sqlx::query("UPDATE agent_templates SET prompt = ? WHERE key = 'sales'")
        .bind(TEMPLATE_PROMPT_V3)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("UPDATE agent_template_assignments SET enabled = 0 WHERE template_id = 2")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("UPDATE providers SET revision = revision + 1")
        .execute(&pool)
        .await
        .unwrap();

    assert_eq!(
        speak_once(&mut socket).await,
        "template-llm[TEMPLATE-PROMPT-V2]",
        "a switch must never read live database state, so the admission snapshot stays authoritative"
    );
    task.abort();
}

#[tokio::test]
async fn a_candidate_excluded_at_admission_can_never_be_switched_to() {
    let (base, url, task, offered_tools) = start_with_scripted_switch(&["broken"]).await;
    let pool = seed(&url).await;
    seed_switchable_pair(&pool).await;
    let broken = insert_template(&pool, "broken", TEMPLATE_PROMPT_V3, true).await;
    bind(
        &pool,
        broken,
        &[
            ("vad", "test"),
            ("asr", "test"),
            ("tts", "test"),
            ("llm", "never-loaded"),
        ],
    )
    .await;
    assign(&pool, broken, false, true).await;

    let mut socket = admit(&base).await;
    assert_eq!(
        speak_once(&mut socket).await,
        "default-llm[TEMPLATE-PROMPT-V1]",
        "a rejected switch is answered by the ordinary tool continuation"
    );
    assert_eq!(
        speak_once(&mut socket).await,
        "default-llm[TEMPLATE-PROMPT-V1]",
        "a rejected switch must leave the active profile untouched"
    );
    let switch_tool = offered_tools
        .lock()
        .unwrap()
        .first()
        .and_then(|tools| {
            tools
                .iter()
                .find(|tool| tool.name == SWITCH_TEMPLATE_TOOL_NAME)
        })
        .cloned()
        .expect("an Agent with admitted candidates may switch");
    assert_eq!(
        switch_tool.parameters["properties"]["template"]["enum"],
        serde_json::json!(["primary", "sales"]),
        "an excluded candidate is never advertised, let alone switchable"
    );
    task.abort();
}

/// A switch the session armed is discarded when its turn is interrupted before the boundary, so a
/// later turn still runs the profile it started with and the revision never advances.
#[tokio::test]
async fn an_interrupted_turn_drops_the_switch_it_armed() {
    let (base, url, task, _) =
        start_with_stalled_switch(&["sales"], Duration::from_millis(750)).await;
    let pool = seed(&url).await;
    seed_switchable_pair(&pool).await;
    let mut socket = admit(&base).await;

    socket
        .send(Message::Text(
            serde_json::json!({"type": "listen", "state": "start", "mode": "manual"})
                .to_string()
                .into(),
        ))
        .await
        .unwrap();
    socket
        .send(Message::Text(
            serde_json::json!({"type": "listen", "state": "detect", "text": "chuyển sang sales"})
                .to_string()
                .into(),
        ))
        .await
        .unwrap();
    // The switch is armed and the turn is now parked in its continuation, well inside the stall.
    tokio::time::sleep(Duration::from_millis(150)).await;
    socket
        .send(Message::Text(
            serde_json::json!({"type": "abort"}).to_string().into(),
        ))
        .await
        .unwrap();

    assert_eq!(
        speak_once(&mut socket).await,
        "default-llm[TEMPLATE-PROMPT-V1]",
        "a turn interrupted before its boundary must not carry its switch forward"
    );
    task.abort();
}

/// Regression for the boundary ordering a switch depends on.
///
/// The writer reports `TurnClosed(Normal)` immediately after the client sees `tts:stop`, so the
/// turns below are sent back-to-back with no delay in between. Each must reach the new profile and
/// keep its captured audio intact — a client that starts a turn the instant a turn closes is
/// entitled to that.
///
/// There is no client-visible synchronization point between `tts:stop` and the writer's own report,
/// so a single turn is only a probabilistic probe. Repeating the back-to-back boundary several
/// times multiplies the opportunities to overtake it, which is what makes this a regression rather
/// than a formality.
#[tokio::test]
async fn the_turn_after_a_switch_keeps_every_audio_frame_and_runs_the_new_profile() {
    const BACK_TO_BACK_TURNS: usize = 4;

    let (base, url, task, _) = start_with_scripted_switch(&["sales"]).await;
    let pool = seed(&url).await;
    seed_switchable_pair(&pool).await;
    let mut socket = admit(&base).await;

    let switching = speak_and_observe(&mut socket).await;
    assert_eq!(
        switching.answer, "default-llm[TEMPLATE-PROMPT-V1]",
        "the turn that requested the switch must finish under the profile it started with"
    );
    assert!(switching.audio_frames > 0);

    for turn in 0..BACK_TO_BACK_TURNS {
        let switched = speak_and_observe(&mut socket).await;
        assert_eq!(
            switched.transcript.as_deref(),
            Some("utterance"),
            "turn {turn} started the instant the previous one closed, so its capture frames must \
             reach ASR rather than be dropped against a still-closing turn"
        );
        assert!(switched.playback_started, "turn {turn} must reach playback");
        assert_eq!(
            switched.audio_frames, switching.audio_frames,
            "turn {turn} must deliver the same audio as the turn before it"
        );
        assert_eq!(
            switched.answer, "template-llm[TEMPLATE-PROMPT-V2]",
            "turn {turn} must run the switched prompt on the switched already-loaded runtime"
        );
    }
    task.abort();
}

/// Holds the writer at its terminal-outcome boundary and withholds the reported outcome from the
/// actor's periodic drain, so client ingress is the only path left that can apply it.
///
/// That replaces the black-box race with a deterministic sequence: the harness releases the writer,
/// waits for proof the outcome is in the actor's mailbox, and only then delivers the next client
/// message. Production installs no probe, so none of this exists outside a test.
struct ControlledBoundary {
    at_boundary: mpsc::UnboundedReceiver<WriterTurnOutcome>,
    enqueued: mpsc::UnboundedReceiver<WriterTurnOutcome>,
    release: Arc<Notify>,
    hold: Arc<AtomicBool>,
}

impl ControlledBoundary {
    /// Runs one turn, then stops the writer exactly at that turn's terminal outcome and does not
    /// let it go until the outcome is provably sitting in the actor's mailbox.
    async fn run_turn_and_hold_at_its_boundary(&mut self, socket: &mut Socket) -> TurnOutcome {
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

async fn start_controlled_switch(
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
        with_server_default_prompt(config("127.0.0.1:0".parse().unwrap(), database_url())),
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
            with_template_llm(state).with_writer_outcome_probe(probe)
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

struct BoundaryProbe {
    at_boundary: mpsc::UnboundedSender<WriterTurnOutcome>,
    enqueued: mpsc::UnboundedSender<WriterTurnOutcome>,
    release: Arc<Notify>,
    hold: Arc<AtomicBool>,
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

/// Deterministic proof of AC2 at the WebSocket boundary.
///
/// The outcome is provably in the actor's mailbox and provably not applied, so the only way the
/// next turn can be captured and answered under the switched profile is that client ingress applies
/// the boundary before interpreting it. Removing that step fails this test every single run.
#[tokio::test]
async fn a_turn_arriving_after_a_reported_boundary_keeps_its_frames_and_uses_the_switched_profile()
{
    let (base, url, task, _, mut boundary) = start_controlled_switch(&["sales"]).await;
    let pool = seed(&url).await;
    seed_switchable_pair(&pool).await;
    let mut socket = admit(&base).await;

    let switching = boundary
        .run_turn_and_hold_at_its_boundary(&mut socket)
        .await;
    assert_eq!(
        switching.answer, "default-llm[TEMPLATE-PROMPT-V1]",
        "the turn that requested the switch must finish under the profile it started with"
    );

    let switched = speak_and_observe(&mut socket).await;

    assert_eq!(
        switched.transcript.as_deref(),
        Some("utterance"),
        "the reported boundary must be applied before this turn is interpreted, or its capture \
         frames are dropped against a turn that has already closed"
    );
    assert!(switched.playback_started);
    assert_eq!(switched.audio_frames, switching.audio_frames);
    assert_eq!(
        switched.answer, "template-llm[TEMPLATE-PROMPT-V2]",
        "the reported boundary must be applied before this turn is interpreted, so the switch it \
         armed takes effect exactly now"
    );
    boundary.hold.store(false, Ordering::SeqCst);
    task.abort();
}

/// Deterministic proof that a normal boundary stays normal.
///
/// An `abort` arriving after the writer reported `TurnClosed(Normal)` must not retroactively turn
/// that turn into an aborted one: the boundary commits first, switch included, and the abort then
/// applies to the fresh session.
#[tokio::test]
async fn an_abort_arriving_after_a_reported_boundary_cannot_undo_the_committed_switch() {
    let (base, url, task, _, mut boundary) = start_controlled_switch(&["sales"]).await;
    let pool = seed(&url).await;
    seed_switchable_pair(&pool).await;
    let mut socket = admit(&base).await;

    let switching = boundary
        .run_turn_and_hold_at_its_boundary(&mut socket)
        .await;
    assert_eq!(switching.answer, "default-llm[TEMPLATE-PROMPT-V1]");

    socket
        .send(Message::Text(
            serde_json::json!({"type": "abort"}).to_string().into(),
        ))
        .await
        .unwrap();
    let after_abort = speak_and_observe(&mut socket).await;

    assert_eq!(
        after_abort.answer, "template-llm[TEMPLATE-PROMPT-V2]",
        "the normal boundary commits its switch before the abort is interpreted; treating the abort \
         first would discard the switch as an interrupted turn"
    );
    assert_eq!(after_abort.transcript.as_deref(), Some("utterance"));
    boundary.hold.store(false, Ordering::SeqCst);
    task.abort();
}

type ManagedBuild = (i64, i64, String, String);

struct ManagedFixtureBuilder {
    config: AppConfig,
    builds: Arc<Mutex<Vec<ManagedBuild>>>,
    supervisor: Arc<voice_agent_server::workers::WorkerSupervisor>,
}
struct ManagedFixtureResource(voice_agent_server::providers::RuntimeCatalog);
impl voice_agent_server::services::provider_runtime::RuntimeResource for ManagedFixtureResource {
    fn unload(&self) -> bool {
        true
    }
    fn runtimes(&self) -> Option<voice_agent_server::providers::RuntimeCatalog> {
        Some(self.0.clone())
    }
}
impl voice_agent_server::services::provider_runtime::RuntimeMaterializer for ManagedFixtureBuilder {
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
        Ok(8)
    }
    fn build(
        &self,
        row: &voice_agent_server::database::DesiredProvider,
        _: Option<voice_agent_server::services::provider_runtime::PreparedRuntime>,
        _: voice_agent_server::workers::ProviderRuntimeAdmission,
    ) -> Result<
        Arc<dyn voice_agent_server::services::provider_runtime::RuntimeResource>,
        voice_agent_server::services::provider_runtime::RuntimeError,
    > {
        self.builds
            .lock()
            .unwrap()
            .push((row.id, row.revision, row.kind.clone(), row.key.clone()));
        let state = AppState::from_provider_set(
            self.config.clone(),
            Arc::new(ProviderSet::with_all(
                Arc::new(SilentVad),
                Arc::new(FinalAsr),
                Arc::new(EchoLlm {
                    label: if row.revision == 1 {
                        "managed-v1"
                    } else {
                        "managed-v2"
                    },
                }),
                Arc::new(ShortTts),
            )),
        );
        let runtimes = state
            .runtimes
            .resolve(&state.config.provider_defaults.effective_bindings())
            .unwrap();
        self.supervisor.observe_asr(runtimes.asr.clone());
        self.supervisor.observe_vad(runtimes.vad.clone());
        let kind = match row.kind.as_str() {
            "vad" => voice_agent_server::providers::DiagnosticRuntimeKind::Vad,
            "asr" => voice_agent_server::providers::DiagnosticRuntimeKind::Asr,
            "llm" => voice_agent_server::providers::DiagnosticRuntimeKind::Llm,
            "tts" => voice_agent_server::providers::DiagnosticRuntimeKind::Tts,
            _ => unreachable!(),
        };
        Ok(Arc::new(ManagedFixtureResource(
            voice_agent_server::providers::RuntimeCatalog::single_provider(
                kind,
                row.key.clone(),
                &runtimes,
            ),
        )))
    }
}

#[tokio::test]
async fn managed_template_acquires_deployment_runtimes_for_missing_slots() {
    use voice_agent_server::{
        database::{AdmittedAssignment, AdmittedProviderBinding, DesiredProvider},
        services::provider_runtime::{ProviderRuntimeManager, RuntimeLimits},
        session::{ManagedSessionProfileInput, resolve_managed_session_profile},
    };

    let state = AppState::from_provider_set(
        config("127.0.0.1:0".parse().unwrap(), database_url()),
        Arc::new(ProviderSet::with_all(
            Arc::new(SilentVad),
            Arc::new(FinalAsr),
            Arc::new(EchoLlm {
                label: "default-llm",
            }),
            Arc::new(ShortTts),
        )),
    );
    let mut profile_config = state.config.as_ref().clone();
    profile_config.provider_defaults.vad = "server_vad".into();
    profile_config.provider_defaults.asr = "server_asr".into();
    profile_config.provider_defaults.llm = "openai_primary".into();
    profile_config.provider_defaults.tts = "server_tts".into();
    let builds = Arc::new(Mutex::new(Vec::new()));
    let manager = ProviderRuntimeManager::new(
        RuntimeLimits {
            max_parallel_loads: 1,
            max_pending_loads: 0,
            max_waiters: 8,
            max_resident_bytes: 8,
            max_resources: 8,
            max_version_entries: 8,
            admission_timeout_ms: 2000,
            failure_cooldown_ms: 10,
            idle_ttl_ms: 1000,
        },
        Arc::new(ManagedFixtureBuilder {
            config: profile_config.clone(),
            builds: builds.clone(),
            supervisor: state.worker_supervisor.clone(),
        }),
        state.admission_gate().clone(),
    )
    .unwrap();
    let provider = |id: i64, kind: &str, key: &str| DesiredProvider {
        id,
        key: key.into(),
        kind: kind.into(),
        adapter: "fixture".into(),
        revision: 1,
        config_json: "{}".into(),
        secret_ref: None,
    };
    let assignment = AdmittedAssignment {
        template_id: 1,
        template_key: "partial".into(),
        template_name: "Partial".into(),
        language: "vi-VN".into(),
        prompt: TEMPLATE_PROMPT_V1.into(),
        template_enabled: true,
        template_revision: 1,
        is_default: true,
        assignment_enabled: true,
        bindings: [("asr", "db_asr", 1), ("tts", "db_tts", 2)]
            .map(|(kind, key, id)| AdmittedProviderBinding {
                provider_type: kind.into(),
                provider_key: key.into(),
                provider_enabled: true,
                snapshot: Some(Arc::new(provider(id, kind, key))),
            })
            .to_vec(),
    };
    let deployment = [
        provider(0, "vad", "server_vad"),
        provider(0, "llm", "openai_primary"),
    ];
    let profile = resolve_managed_session_profile(ManagedSessionProfileInput {
        device_db_id: 2,
        template_override_id: None,
        agent_id: 1,
        agent_key: "home",
        assignments: &[assignment],
        config: &profile_config,
        manager: &manager,
        deployment_snapshots: &deployment,
    })
    .await
    .unwrap();
    assert_eq!(profile.providers.vad, "server_vad");
    assert_eq!(profile.providers.asr, "db_asr");
    assert_eq!(profile.providers.llm, "openai_primary");
    assert_eq!(profile.providers.tts, "db_tts");
    assert!(profile.selected_runtimes.is_some());
    let actual = builds
        .lock()
        .unwrap()
        .iter()
        .map(|build| build.3.clone())
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(
        actual,
        std::collections::BTreeSet::from([
            "server_vad".to_owned(),
            "db_asr".to_owned(),
            "openai_primary".to_owned(),
            "db_tts".to_owned(),
        ])
    );
}

#[tokio::test]
async fn public_api_created_provider_is_used_by_new_ws_and_patch_keeps_old_session_version() {
    use voice_agent_server::services::provider_runtime::{ProviderRuntimeManager, RuntimeLimits};
    let url = database_url();
    let mut app_config = config("127.0.0.1:0".parse().unwrap(), url);
    app_config.api.enabled = true;
    app_config.api.admin_token = "managed-test-token".into();
    let builds = Arc::new(Mutex::new(Vec::new()));
    let recorded = builds.clone();
    let (base, url, task) = start_with_state(app_config, |state| {
        let manager = ProviderRuntimeManager::new(
            RuntimeLimits {
                max_parallel_loads: 1,
                max_pending_loads: 0,
                max_waiters: 32,
                max_resident_bytes: 16,
                max_resources: 16,
                max_version_entries: 32,
                admission_timeout_ms: 2000,
                failure_cooldown_ms: 10,
                idle_ttl_ms: 1000,
            },
            Arc::new(ManagedFixtureBuilder {
                config: state.config.as_ref().clone(),
                builds: recorded,
                supervisor: state.worker_supervisor.clone(),
            }),
            state.admission_gate().clone(),
        )
        .unwrap();
        state.with_runtime_manager(manager)
    })
    .await;
    let _pool = seed(&url).await;
    let client = reqwest::Client::new();
    let auth = "managed-test-token";
    let providers = format!("{base}/api/admin/providers");
    let mut generated = Vec::new();
    for (kind, adapter, config_json) in [
        ("vad", "silero_onnx", serde_json::json!({})),
        (
            "asr",
            "gipformer_sherpa_offline",
            serde_json::json!({"decoding_method":"greedy_search","max_active_paths":4}),
        ),
        (
            "llm",
            "openai",
            serde_json::json!({"base_url":"https://example.test/v1","model":"version-one"}),
        ),
        ("tts", "zerotts_onnx", serde_json::json!({"voice":"maichi"})),
    ] {
        let response = client.post(&providers).bearer_auth(auth).json(&serde_json::json!({"name":kind,"type":kind,"adapter":adapter,"config_json":config_json})).send().await.unwrap();
        let status = response.status();
        let body: serde_json::Value = response.json().await.unwrap();
        assert_eq!(status, reqwest::StatusCode::CREATED, "{body}");
        generated.push((kind, body["key"].as_str().unwrap().to_owned()));
    }
    let llm_key = &generated[2].1;
    let response = client.post(format!("{base}/api/admin/templates")).bearer_auth(auth).json(&serde_json::json!({"key":"managed","name":"Managed","language":"vi-VN","prompt":"MANAGED-PROMPT"})).send().await.unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::CREATED);
    let mut template: serde_json::Value = response.json().await.unwrap();
    for (kind, provider_key) in &generated {
        let response = client
            .put(format!(
                "{base}/api/admin/templates/managed/providers/{kind}"
            ))
            .bearer_auth(auth)
            .header("if-match", format!("\"{}\"", template["revision"]))
            .json(&serde_json::json!({"provider_key":provider_key}))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), reqwest::StatusCode::OK);
        template = response.json().await.unwrap();
    }
    let response = client
        .put(format!(
            "{base}/api/admin/agents/agent/default-template/managed"
        ))
        .bearer_auth(auth)
        .header("if-match", "\"1\"")
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::OK);
    assert!(
        builds.lock().unwrap().is_empty(),
        "CRUD must not build runtimes"
    );
    let cold: serde_json::Value = client
        .get(format!("{providers}/{llm_key}"))
        .bearer_auth(auth)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(cold["requires_restart"], false);
    assert_eq!(cold["runtime"]["desired_state"], "cold");
    assert!(builds.lock().unwrap().is_empty());
    let mut first = admit(&base).await;
    assert!(
        speak_and_observe(&mut first)
            .await
            .answer
            .starts_with("managed-v1[MANAGED-PROMPT]")
    );
    assert_eq!(builds.lock().unwrap().len(), 4);
    let response = client.patch(format!("{providers}/{llm_key}")).bearer_auth(auth).header("if-match", "\"1\"").json(&serde_json::json!({"config_json":{"base_url":"https://example.test/v1","model":"version-two"}})).send().await.unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::OK);
    let changed: serde_json::Value = client
        .get(format!("{providers}/{llm_key}"))
        .bearer_auth(auth)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(changed["runtime"]["desired_revision"], 2);
    assert_eq!(changed["runtime"]["desired_state"], "cold");
    assert_eq!(
        changed["runtime"]["ready_revisions"],
        serde_json::json!([1])
    );
    assert_eq!(builds.lock().unwrap().len(), 4);
    let mut second = admit(&base).await;
    assert!(
        speak_and_observe(&mut second)
            .await
            .answer
            .starts_with("managed-v2[MANAGED-PROMPT]")
    );
    assert!(
        speak_and_observe(&mut first)
            .await
            .answer
            .starts_with("managed-v1[MANAGED-PROMPT]")
    );
    assert_eq!(
        builds.lock().unwrap().len(),
        5,
        "only changed exact version is built"
    );
    let created = client.post(&providers).bearer_auth(auth).json(&serde_json::json!({"name":"Unbound","type":"llm","adapter":"openai","config_json":{"base_url":"https://example.test/v1","model":"unbound"}})).send().await.unwrap();
    assert_eq!(created.status(), reqwest::StatusCode::CREATED);
    let created: serde_json::Value = created.json().await.unwrap();
    let unbound = created["key"].as_str().unwrap().to_owned();
    let prepared = client
        .post(format!("{providers}/{unbound}/prepare"))
        .bearer_auth(auth)
        .json(&serde_json::json!({}))
        .send()
        .await
        .unwrap();
    assert!(
        [reqwest::StatusCode::OK, reqwest::StatusCode::ACCEPTED].contains(&prepared.status()),
        "{}",
        prepared.text().await.unwrap()
    );
    let prepared: serde_json::Value = prepared.json().await.unwrap();
    assert_eq!(prepared["provider_key"], unbound);
    assert_eq!(prepared["desired_revision"], 1);
    let invalid = client
        .post(format!("{providers}/{unbound}/prepare"))
        .bearer_auth(auth)
        .json(&serde_json::json!({"config_json":{"model":"override"}}))
        .send()
        .await
        .unwrap();
    assert_eq!(invalid.status(), reqwest::StatusCode::BAD_REQUEST);
    let ready = client
        .post(format!("{providers}/{unbound}/prepare"))
        .bearer_auth(auth)
        .json(&serde_json::json!({}))
        .send()
        .await
        .unwrap();
    assert_eq!(ready.status(), reqwest::StatusCode::OK);
    assert_eq!(
        builds.lock().unwrap().len(),
        6,
        "repeated prepare is idempotent"
    );
    let tested = client
        .post(format!("{providers}/{unbound}/test/llm"))
        .bearer_auth(auth)
        .json(&serde_json::json!({"input":"fixture"}))
        .send()
        .await
        .unwrap();
    assert_eq!(
        tested.status(),
        reqwest::StatusCode::OK,
        "{}",
        tested.text().await.unwrap()
    );
    let diagnostic: serde_json::Value = tested.json().await.unwrap();
    assert_eq!(diagnostic["runtime"]["tested_revision"], 1);
    assert_eq!(diagnostic["runtime"]["requires_restart"], false);
    assert!(
        diagnostic["result"]["text"]
            .as_str()
            .unwrap()
            .starts_with("managed-v1")
    );
    assert_eq!(builds.lock().unwrap().len(), 6);
    let disabled = client
        .patch(format!("{providers}/{unbound}"))
        .bearer_auth(auth)
        .header("If-Match", "\"1\"")
        .json(&serde_json::json!({"enabled":false}))
        .send()
        .await
        .unwrap();
    assert_eq!(disabled.status(), reqwest::StatusCode::OK);
    let denied = client
        .post(format!("{providers}/{unbound}/prepare"))
        .bearer_auth(auth)
        .json(&serde_json::json!({}))
        .send()
        .await
        .unwrap();
    assert_eq!(denied.status(), reqwest::StatusCode::CONFLICT);
    assert_eq!(builds.lock().unwrap().len(), 6);
    first.close(None).await.unwrap();
    second.close(None).await.unwrap();
    task.abort();
}

struct PrepareGateBuilder {
    inner: ManagedFixtureBuilder,
    entered: tokio::sync::mpsc::UnboundedSender<()>,
    release: Mutex<std::sync::mpsc::Receiver<()>>,
}
impl voice_agent_server::services::provider_runtime::RuntimeMaterializer for PrepareGateBuilder {
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
        Ok(2)
    }
    fn build(
        &self,
        row: &voice_agent_server::database::DesiredProvider,
        prepared: Option<voice_agent_server::services::provider_runtime::PreparedRuntime>,
        quota: voice_agent_server::workers::ProviderRuntimeAdmission,
    ) -> Result<
        Arc<dyn voice_agent_server::services::provider_runtime::RuntimeResource>,
        voice_agent_server::services::provider_runtime::RuntimeError,
    > {
        self.entered.send(()).unwrap();
        self.release.lock().unwrap().recv().unwrap();
        self.inner.build(row, prepared, quota)
    }
}
#[tokio::test]
async fn prepare_public_api_returns_accepted_then_ready_and_rejects_loader_flood() {
    use voice_agent_server::services::provider_runtime::{ProviderRuntimeManager, RuntimeLimits};
    let mut cfg = config("127.0.0.1:0".parse().unwrap(), database_url());
    cfg.api.enabled = true;
    cfg.api.admin_token = "prepare-test-token".into();
    let (entered, mut entered_rx) = tokio::sync::mpsc::unbounded_channel();
    let (release, release_rx) = std::sync::mpsc::channel();
    let (base, _, task) = start_with_state(cfg, |state| {
        let manager = ProviderRuntimeManager::new(
            RuntimeLimits {
                max_parallel_loads: 1,
                max_pending_loads: 0,
                max_waiters: 4,
                max_resident_bytes: 4,
                max_resources: 4,
                max_version_entries: 8,
                admission_timeout_ms: 2000,
                failure_cooldown_ms: 10,
                idle_ttl_ms: 1000,
            },
            Arc::new(PrepareGateBuilder {
                inner: ManagedFixtureBuilder {
                    config: state.config.as_ref().clone(),
                    builds: Arc::new(Mutex::new(vec![])),
                    supervisor: state.worker_supervisor.clone(),
                },
                entered,
                release: Mutex::new(release_rx),
            }),
            state.lifecycle.gate().clone(),
        )
        .unwrap();
        state.with_runtime_manager(manager)
    })
    .await;
    let client = reqwest::Client::new();
    let providers = format!("{base}/api/admin/providers");
    let mut generated = Vec::new();
    for name in ["first", "second"] {
        let created = client.post(&providers).bearer_auth("prepare-test-token")
            .json(&serde_json::json!({"name":name,"type":"llm","adapter":"openai","config_json":{"base_url":"https://example.test/v1","model":"fixture"}}))
            .send().await.unwrap();
        assert_eq!(created.status(), reqwest::StatusCode::CREATED);
        let created: serde_json::Value = created.json().await.unwrap();
        generated.push(created["key"].as_str().unwrap().to_owned());
    }
    let (first_key, second_key) = (&generated[0], &generated[1]);
    let first = client
        .post(format!("{providers}/{first_key}/prepare"))
        .bearer_auth("prepare-test-token")
        .json(&serde_json::json!({}))
        .send()
        .await
        .unwrap();
    assert_eq!(first.status(), reqwest::StatusCode::ACCEPTED);
    let accepted: serde_json::Value = first.json().await.unwrap();
    assert_eq!(accepted["provider_key"], *first_key);
    assert_eq!(accepted["desired_revision"], 1);
    entered_rx.recv().await.unwrap();
    let blocked = client
        .post(format!("{providers}/{second_key}/prepare"))
        .bearer_auth("prepare-test-token")
        .json(&serde_json::json!({}))
        .send()
        .await
        .unwrap();
    assert_eq!(blocked.status(), reqwest::StatusCode::TOO_MANY_REQUESTS);
    release.send(()).unwrap();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(1);
    loop {
        let response = client
            .get(format!("{providers}/{first_key}"))
            .bearer_auth("prepare-test-token")
            .send()
            .await
            .unwrap();
        let row: serde_json::Value = response.json().await.unwrap();
        if row["runtime"]["desired_state"] == "ready" {
            break;
        }
        assert!(tokio::time::Instant::now() < deadline);
        tokio::task::yield_now().await;
    }
    let ready = client
        .post(format!("{providers}/{first_key}/prepare"))
        .bearer_auth("prepare-test-token")
        .json(&serde_json::json!({}))
        .send()
        .await
        .unwrap();
    assert_eq!(ready.status(), reqwest::StatusCode::OK);
    task.abort();
}
