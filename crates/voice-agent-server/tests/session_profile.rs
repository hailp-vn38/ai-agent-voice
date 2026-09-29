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
use tokio::{net::TcpListener, task::JoinHandle, time::timeout};
use tokio_tungstenite::{
    connect_async,
    tungstenite::{Message, client::IntoClientRequest, http::StatusCode},
};
use url::Url;
use voice_agent_server::{
    app::AppState,
    config::{
        AppConfig, AudioConfig, AuthConfig, BargeInConfig, DatabaseConfig, DatabaseDevicesConfig,
        DeploymentConfig, LimitsConfig, LlmConfig, McpConfig, ProviderDefaultsConfig,
        ProvidersConfig, RuntimeConfig, ServerConfig, SileroOnnxConfig, SpeechOutputConfig,
        TtsConfig, VadInstanceConfig, VisionConfig, WebsocketConfig, WorkersConfig,
    },
    database::Database,
    providers::{
        AsrError, AsrEvent, AsrProvider, AsrResult, AsrSession, LlmError, LlmEvent, LlmProvider,
        ProviderSet, TtsError, TtsProvider, VadError, VadInput, VadProbability, VadProvider,
        VadSession,
        llm::{ChatMessage, LlmEventStream, LlmRequest, ToolCall, ToolDefinition},
    },
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
        llm: LlmConfig::default(),
        tts: TtsConfig::default(),
        speech_output: SpeechOutputConfig::default(),
        barge_in: BargeInConfig::default(),
        mcp: McpConfig::default(),
        vision: VisionConfig::default(),
        database: DatabaseConfig {
            enabled: true,
            url,
            devices: DatabaseDevicesConfig {
                admission_enabled: true,
                ..Default::default()
            },
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

/// Binds the four required slots.  An omitted kind leaves that slot unbound, which must make the
/// Template invalid rather than partially defaulted.
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
async fn an_agent_with_only_a_disabled_assignment_never_falls_back_to_server_defaults() {
    let (base, url, task) = start(with_server_default_prompt(config(
        "127.0.0.1:0".parse().unwrap(),
        database_url(),
    )))
    .await;
    let pool = seed(&url).await;
    let template = insert_template(&pool, "primary", TEMPLATE_PROMPT_V1, true).await;
    bind(&pool, template, &full_bindings("test")).await;
    assign(&pool, template, true, false).await;

    let error = connect_async(request(&base)).await.unwrap_err();
    assert_eq!(rejected_status(error), StatusCode::SERVICE_UNAVAILABLE);
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
async fn a_default_template_missing_a_binding_never_mixes_in_defaults() {
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

    let error = connect_async(request(&base)).await.unwrap_err();
    assert_eq!(rejected_status(error), StatusCode::SERVICE_UNAVAILABLE);
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
