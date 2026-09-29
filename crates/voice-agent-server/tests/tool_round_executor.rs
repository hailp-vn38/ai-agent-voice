//! The shared Tool-round Executor, observed at the boundary a Voice Protocol Client can see.
//!
//! Every test here drives a real WebSocket session with a real Device MCP peer and a real External
//! MCP server, and asserts on the observable order of what the model asked for, what the two
//! transports were actually called with, and what the next LLM request was given back.  Nothing
//! asserts on a private field, and nothing needs a real model.
//!
//! The one thing this file deliberately does not prove is the late-response *discard* path and the
//! counters that go with cancellation.  Over a socket, a response that has been posted and an
//! abort that has been sent race inside a one millisecond actor tick, so which of the two the
//! executor sees first is not something a black-box test gets to choose.  Those two are proven
//! where they can be made deterministic: on the actor itself, in `session::actor::tools`.

use std::{
    net::SocketAddr,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use axum::{
    Router,
    body::Bytes,
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::post,
};
use futures_util::{SinkExt, StreamExt, stream};
use sqlx::sqlite::SqlitePoolOptions;
use tokio::{
    net::TcpListener,
    sync::{Notify, watch},
    task::JoinHandle,
    time::timeout,
};
use tokio_tungstenite::{
    connect_async,
    tungstenite::{Message, client::IntoClientRequest},
};
use url::Url;
use voice_agent_server::{
    app::{AppState, router_with_state},
    audio::PcmF32Mono,
    config::{
        AppConfig, AudioConfig, AuthConfig, BargeInConfig, DatabaseConfig, DatabaseDevicesConfig,
        DeploymentConfig, ExternalMcpConfig, ExternalMcpLimitsConfig, ExternalMcpNetworkConfig,
        LimitsConfig, LlmConfig, LlmToolsConfig, McpConfig, ProviderDefaultsConfig,
        ProvidersConfig, RuntimeConfig, ServerConfig, SileroOnnxConfig, SpeechOutputConfig,
        TtsConfig, VadInstanceConfig, VisionConfig, WebsocketConfig, WorkersConfig,
    },
    database::{
        Database,
        secrets::{SecretRef, SecretResolveError, SecretResolver, SecretValue},
    },
    providers::{
        AsrError, AsrEvent, AsrProvider, AsrResult, AsrSession, LlmError, LlmEvent, LlmProvider,
        ProviderSet, TtsError, TtsProvider, VadError, VadInput, VadProbability, VadProvider,
        VadSession,
        llm::{ChatMessage, LlmEventStream, LlmRequest, ToolCall},
    },
    tools::builtin::EXIT_TOOL_NAME,
};

const DEVICE_TOOL: &str = "self.lamp.on";
/// The Device MCP name the model sees: sanitization cannot produce a dot, so this is unambiguous.
const DEVICE_LLM_NAME: &str = "self_lamp_on";
/// The published External MCP name: a fixed namespace plus the normalized server and tool.
const EXTERNAL_LLM_NAME: &str = "external.weather.forecast";

// ---------------------------------------------------------------------------
// Scripted LLM
// ---------------------------------------------------------------------------

/// One LLM round the scripted provider returns.
///
/// `ToolCalls` returns calls and nothing else; `Text` returns a final answer.  A turn's script is
/// consumed in order, so a test states exactly what the model does instead of inferring it from
/// what the executor happens to do.
#[derive(Clone)]
enum Round {
    ToolCalls(Vec<(&'static str, &'static str, serde_json::Value)>),
    Text(&'static str),
}

/// Everything a scripted provider observed, so a test can assert what the continuation was handed
/// rather than only that something was sent.
#[derive(Default)]
struct Observed {
    /// The tool names offered on each round.
    offered: Vec<Vec<String>>,
    /// Each round's incoming messages, which is where a completed tool round must appear paired.
    requests: Vec<Vec<ChatMessage>>,
}

struct ScriptedLlm {
    script: Arc<Mutex<Vec<Round>>>,
    observed: Arc<Mutex<Observed>>,
}

impl ScriptedLlm {
    fn new(script: Vec<Round>) -> (Self, Arc<Mutex<Observed>>) {
        let observed = Arc::new(Mutex::new(Observed::default()));
        (
            Self {
                script: Arc::new(Mutex::new(script)),
                observed: Arc::clone(&observed),
            },
            observed,
        )
    }
}

#[async_trait::async_trait]
impl LlmProvider for ScriptedLlm {
    fn adapter(&self) -> &'static str {
        "tool-round-scripted"
    }

    async fn stream(&self, request: LlmRequest) -> Result<LlmEventStream, LlmError> {
        {
            let mut observed = self
                .observed
                .lock()
                .expect("the observation mailbox is not poisoned");
            observed
                .offered
                .push(request.tools.iter().map(|tool| tool.name.clone()).collect());
            observed.requests.push(request.messages.clone());
        }
        // A script that runs out means the turn asked for another round, which is itself what the
        // cap tests are looking for; answering with a final turn keeps the session converging
        // instead of hanging.
        let next = self
            .script
            .lock()
            .expect("the script mailbox is not poisoned")
            .first()
            .cloned();
        if let Some(round) = next {
            self.script
                .lock()
                .expect("the script mailbox is not poisoned")
                .remove(0);
            return Ok(match round {
                Round::ToolCalls(calls) => Box::pin(stream::iter(
                    calls
                        .into_iter()
                        .map(|(id, name, arguments)| {
                            Ok(LlmEvent::ToolCall(ToolCall {
                                id: id.to_owned(),
                                name: name.to_owned(),
                                arguments,
                            }))
                        })
                        .chain(std::iter::once(Ok(LlmEvent::Finished)))
                        .collect::<Vec<_>>(),
                )),
                Round::Text(text) => Box::pin(stream::iter(vec![
                    Ok(LlmEvent::TextDelta(text.to_owned())),
                    Ok(LlmEvent::Finished),
                ])),
            });
        }
        Ok(Box::pin(stream::iter(vec![
            Ok(LlmEvent::TextDelta("done".to_owned())),
            Ok(LlmEvent::Finished),
        ])))
    }
}

// ---------------------------------------------------------------------------
// Audio providers
// ---------------------------------------------------------------------------

struct FinalAsr;

impl AsrProvider for FinalAsr {
    fn open(&self) -> Result<Box<dyn AsrSession>, AsrError> {
        Ok(Box::new(FinalAsrSession))
    }
}

struct FinalAsrSession;

impl AsrSession for FinalAsrSession {
    fn push_pcm(&mut self, _: &PcmF32Mono) -> Result<Vec<AsrEvent>, AsrError> {
        Ok(Vec::new())
    }
    fn finish(&mut self) -> Result<AsrResult, AsrError> {
        Ok(AsrResult::new("utterance"))
    }
    fn cancel(&mut self) {}
}

struct SilentVad;

impl VadProvider for SilentVad {
    fn adapter(&self) -> &'static str {
        "silent-vad"
    }
    fn open(&self) -> Result<Box<dyn VadSession>, VadError> {
        Ok(Box::new(SilentVadSession))
    }
}

struct SilentVadSession;

impl VadSession for SilentVadSession {
    fn push(&mut self, input: VadInput) -> Result<VadProbability, VadError> {
        Ok(VadProbability {
            start_sample: input.start_sample,
            end_sample: input.start_sample + input.pcm.len() as u64,
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
    fn synthesize(&self, _: &str) -> Result<PcmF32Mono, TtsError> {
        Ok(PcmF32Mono::new(vec![0.1; 2_400], 48_000))
    }
}

struct ConstantSecrets(Option<String>);

impl SecretResolver for ConstantSecrets {
    fn resolve(&self, _reference: &SecretRef) -> Result<SecretValue, SecretResolveError> {
        self.0
            .clone()
            .map(SecretValue::new)
            .ok_or(SecretResolveError::Unavailable)
    }
}

// ---------------------------------------------------------------------------
// Scripted External MCP server
// ---------------------------------------------------------------------------

/// Lets a test hold a `tools/call` open, so "a call is in flight" is an event the test chooses
/// rather than a sleep it hopes for.
#[derive(Clone)]
struct CallGate {
    /// Signalled each time a call starts being held.  `Notify::notify_one` keeps one permit, so a
    /// test that arrives late still learns about the call it missed.
    held: Arc<Notify>,
    held_calls: Arc<AtomicUsize>,
    /// The open/closed switch every held call waits on.  A watch rather than a notification,
    /// because a call that starts waiting after the test opened the gate must not wait forever.
    open: watch::Receiver<bool>,
    opener: watch::Sender<bool>,
}

impl CallGate {
    fn new() -> Self {
        let (opener, open) = watch::channel(false);
        Self {
            held: Arc::new(Notify::new()),
            held_calls: Arc::new(AtomicUsize::new(0)),
            open,
            opener,
        }
    }

    /// The server side: report that a call is now held, then wait for the test to open the gate.
    async fn hold(&self) {
        self.held_calls.fetch_add(1, Ordering::SeqCst);
        self.held.notify_one();
        let mut open = self.open.clone();
        while !*open.borrow_and_update() {
            open.changed()
                .await
                .expect("the gate outlives every call that waits on it");
        }
    }

    /// The test side: wait until at least `count` calls are being held.
    async fn wait_until_held(&self, count: usize) {
        while self.held_calls.load(Ordering::SeqCst) < count {
            self.held.notified().await;
        }
    }

    fn open(&self) {
        let _ = self.opener.send(true);
    }
}

/// What the fake External MCP server observed, in the order it observed it.
#[derive(Clone)]
struct ExternalScript {
    tools: Vec<serde_json::Value>,
    /// Answer `tools/call` with this HTTP status instead of a result.
    call_status: Option<u16>,
    gate: Option<CallGate>,
    calls: Arc<Mutex<Vec<String>>>,
    concurrent: Arc<Mutex<(usize, usize)>>,
}

impl ExternalScript {
    fn with_tools(tools: Vec<serde_json::Value>) -> Self {
        Self {
            tools,
            call_status: None,
            gate: None,
            calls: Arc::new(Mutex::new(Vec::new())),
            concurrent: Arc::new(Mutex::new((0, 0))),
        }
    }

    fn calls(&self) -> Vec<String> {
        self.calls
            .lock()
            .expect("the script mailbox is not poisoned")
            .clone()
    }

    /// The highest number of `tools/call` requests the server ever had open at once.
    fn peak_concurrency(&self) -> usize {
        self.concurrent
            .lock()
            .expect("the script mailbox is not poisoned")
            .1
    }
}

type SharedExternal = Arc<Mutex<ExternalScript>>;

fn external_tool(name: &str) -> serde_json::Value {
    serde_json::json!({
        "name": name,
        "description": format!("{name} tool"),
        "inputSchema": {"type": "object", "properties": {}, "additionalProperties": false}
    })
}

async fn external_endpoint(State(shared): State<SharedExternal>, body: Bytes) -> Response {
    let Ok(request) = serde_json::from_slice::<serde_json::Value>(&body) else {
        return (StatusCode::BAD_REQUEST, "malformed").into_response();
    };
    // Everything the handler needs is taken out of the script up front, so no lock is ever held
    // across the wait below: a handler future has to stay `Send`, and a std guard is not.
    let (tools, gate, calls, concurrent, call_status) = {
        let script = shared.lock().expect("the script mailbox is not poisoned");
        (
            script.tools.clone(),
            script.gate.clone(),
            Arc::clone(&script.calls),
            Arc::clone(&script.concurrent),
            script.call_status,
        )
    };
    let id = request.get("id").cloned();
    let method = request.get("method").and_then(serde_json::Value::as_str);
    match method {
        // A notification expects acceptance, not an answer: a JSON-RPC document here would be a
        // response to a request that was never made.
        None => StatusCode::ACCEPTED.into_response(),
        Some(method) if method.starts_with("notifications/") => {
            StatusCode::ACCEPTED.into_response()
        }
        Some("initialize") => rpc(
            id,
            serde_json::json!({
                "protocolVersion": "2024-11-05",
                "capabilities": {"tools": {}},
                "serverInfo": {"name": "scripted", "version": "1"}
            }),
        ),
        Some("tools/list") => rpc(id, serde_json::json!({"tools": tools})),
        Some("tools/call") => {
            let name = request["params"]["name"]
                .as_str()
                .unwrap_or_default()
                .to_owned();
            calls
                .lock()
                .expect("the script mailbox is not poisoned")
                .push(name.clone());
            {
                // The peak is taken on entry, so a call that is still being held counts as open:
                // that is exactly the overlap this test is looking for.
                let mut concurrent = concurrent
                    .lock()
                    .expect("the script mailbox is not poisoned");
                concurrent.0 += 1;
                concurrent.1 = concurrent.1.max(concurrent.0);
            }
            if let Some(gate) = gate {
                gate.hold().await;
            }
            if let Some(status) = call_status {
                concurrent
                    .lock()
                    .expect("the script mailbox is not poisoned")
                    .0 -= 1;
                return StatusCode::from_u16(status)
                    .unwrap_or(StatusCode::BAD_GATEWAY)
                    .into_response();
            }
            concurrent
                .lock()
                .expect("the script mailbox is not poisoned")
                .0 -= 1;
            rpc(
                id,
                serde_json::json!({
                    "content": [{"type": "text", "text": format!("external:{name}")}],
                    "isError": false
                }),
            )
        }
        Some(_) => rpc(
            id,
            serde_json::json!({"error": {"code": -32601, "message": "no such method"}}),
        ),
    }
}

fn rpc(id: Option<serde_json::Value>, result: serde_json::Value) -> Response {
    let mut document = serde_json::json!({"jsonrpc": "2.0", "result": result});
    if let Some(id) = id {
        document["id"] = id;
    }
    (
        StatusCode::OK,
        [(
            axum::http::header::CONTENT_TYPE,
            "application/json".to_owned(),
        )],
        document.to_string(),
    )
        .into_response()
}

struct ExternalServer {
    url: String,
    script: SharedExternal,
    #[allow(dead_code)]
    task: JoinHandle<()>,
}

impl ExternalServer {
    async fn start(script: ExternalScript) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let shared: SharedExternal = Arc::new(Mutex::new(script));
        let state = Arc::clone(&shared);
        let router = Router::new().route("/mcp", post(external_endpoint));
        let task = tokio::spawn(async move {
            axum::serve(listener, router.with_state(state))
                .await
                .unwrap()
        });
        Self {
            url: format!("http://{address}/mcp"),
            script: shared,
            task,
        }
    }

    fn script(&self) -> ExternalScript {
        self.script
            .lock()
            .expect("the script mailbox is not poisoned")
            .clone()
    }
}

// ---------------------------------------------------------------------------
// Voice server
// ---------------------------------------------------------------------------

fn database_url() -> String {
    format!(
        "sqlite://{}",
        std::env::temp_dir()
            .join(format!(
                "voice-agent-tool-round-{}.db",
                uuid::Uuid::new_v4()
            ))
            .display()
    )
}

fn config(url: String, tools: LlmToolsConfig) -> AppConfig {
    config_with_external(url, tools, ExternalMcpConfig::default())
}

fn config_with_external(
    url: String,
    tools: LlmToolsConfig,
    external: ExternalMcpConfig,
) -> AppConfig {
    let address: SocketAddr = ([127, 0, 0, 1], 0).into();
    let mut providers = ProvidersConfig::default();
    let VadInstanceConfig::SileroOnnx(vad) = providers
        .vad
        .instances
        .entry("test".into())
        .or_insert_with(|| VadInstanceConfig::SileroOnnx(SileroOnnxConfig::default()));
    vad.min_speech_ms = 32;
    vad.end_silence_ms = 32;
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
        llm: LlmConfig {
            tools,
            ..LlmConfig::default()
        },
        tts: TtsConfig::default(),
        speech_output: SpeechOutputConfig::default(),
        barge_in: BargeInConfig::default(),
        mcp: McpConfig {
            external: ExternalMcpConfig {
                // Loopback has to be named by the operator, exactly as a LAN range would be: these
                // tests prove the documented HTTP exception is usable, not that policy can be
                // skipped.
                network: ExternalMcpNetworkConfig {
                    allow_http_lan: true,
                    allowed_hosts: vec![],
                    allowed_cidrs: vec!["127.0.0.0/8".into()],
                },
                limits: ExternalMcpLimitsConfig::default(),
                ..external
            },
            ..McpConfig::default()
        },
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

struct Voice {
    base: String,
    #[allow(dead_code)]
    task: JoinHandle<()>,
}

async fn start(mut app_config: AppConfig, llm: ScriptedLlm) -> Voice {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    // The advertised URL is what a client reads out of `/voice/ota/`, so it has to name the port
    // this server actually got rather than the ephemeral placeholder in the fixture.
    app_config.server.public_ws_url = Url::parse(&format!("ws://{address}/voice/v1/")).unwrap();
    let database = Database::connect(&app_config.database).await.unwrap();
    let state = AppState::from_provider_set_with_database_resolver_and_shutdown(
        app_config,
        Arc::new(ProviderSet::with_all(
            Arc::new(SilentVad),
            Arc::new(FinalAsr),
            Arc::new(llm),
            Arc::new(ShortTts),
        )),
        Some(database),
        Arc::new(ConstantSecrets(Some("s3cr3t-bearer".into()))),
        tokio_util::sync::CancellationToken::new(),
    );
    let served = state.clone();
    let task = tokio::spawn(async move {
        axum::serve(listener, router_with_state(served))
            .await
            .unwrap()
    });
    Voice {
        base: format!("http://{address}"),
        task,
    }
}

/// Migrations, then a Device bound to an Agent, then one External MCP server bound to that Agent.
async fn seed(url: &str, external_url: &str) {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect(url)
        .await
        .unwrap();
    sqlx::query("INSERT INTO agents (key,name,enabled,created_at,updated_at) VALUES ('agent','Agent',1,1,1)")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO devices (device_id,agent_id,enabled,created_at,updated_at) VALUES ('device',1,1,1,1)")
        .execute(&pool)
        .await
        .unwrap();
    let server_id: i64 = sqlx::query_scalar(
        "INSERT INTO mcp_servers (key,name,url,headers_json,auth_type,auth_header_name,secret_ref,\
         connect_timeout_ms,request_timeout_ms,enabled,created_at,updated_at) \
         VALUES ('weather','Weather',?,'{}','bearer',NULL,'TOKEN',5000,5000,1,1,1) RETURNING id",
    )
    .bind(external_url)
    .fetch_one(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO agent_mcp_bindings (agent_id,mcp_server_id,enabled,required,created_at) \
         VALUES (1, ?, 1, 0, 1)",
    )
    .bind(server_id)
    .execute(&pool)
    .await
    .unwrap();
}

// ---------------------------------------------------------------------------
// The Voice Protocol Client, also playing the Device MCP peer
// ---------------------------------------------------------------------------

/// One Device MCP `tools/call` this peer was actually asked to make, in order.
#[derive(Clone, Debug, PartialEq)]
struct DeviceCall {
    name: String,
    arguments: serde_json::Value,
}

type Socket =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

struct Peer {
    socket: Socket,
    session_id: String,
    device_calls: Vec<DeviceCall>,
    /// Every `llm` text segment the client was shown, in order.
    spoken: Vec<String>,
    turn_closed: bool,
    /// When false, the peer records each `tools/call` and never answers it, so a test can watch a
    /// turn run out of time on a request that really was made.
    answer_device_calls: bool,
    /// Set once this peer has answered `tools/list`, which is when the session's Device MCP
    /// catalog exists.  A real client waits for discovery before it speaks, and so must the
    /// harness, or a turn would be built against a catalog that had not arrived yet.
    discovered: bool,
}

impl Peer {
    async fn open(base: &str) -> Self {
        let ota: serde_json::Value = reqwest::Client::new()
            .post(format!("{base}/voice/ota/"))
            .json(&serde_json::json!({}))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        let mut request = ota["websocket"]["url"]
            .as_str()
            .unwrap()
            .into_client_request()
            .unwrap();
        {
            let headers = request.headers_mut();
            headers.insert("Protocol-Version", "1".parse().unwrap());
            headers.insert("Device-Id", "device".parse().unwrap());
            headers.insert("Client-Id", "tool-round".parse().unwrap());
        }
        let (mut socket, _) = connect_async(request).await.unwrap();
        socket
            .send(Message::Text(
                serde_json::json!({"type":"hello","version":1,"transport":"websocket",
                    "features":{"mcp":true},
                    "audio_params":{"format":"opus","sample_rate":16000,"channels":1,
                                    "frame_duration":60}})
                .to_string()
                .into(),
            ))
            .await
            .unwrap();
        let session_id = loop {
            let hello = next_text(&mut socket, "server hello").await;
            let value: serde_json::Value = serde_json::from_str(&hello).unwrap();
            if value["type"] == "hello" {
                break value["session_id"].as_str().unwrap().to_owned();
            }
        };
        Self {
            socket,
            session_id,
            device_calls: Vec::new(),
            spoken: Vec::new(),
            turn_closed: false,
            answer_device_calls: true,
            discovered: false,
        }
    }

    /// Answers the server's discovery until the session's Device MCP catalog exists.
    async fn wait_for_discovery(&mut self) {
        self.pump_until(Duration::from_secs(5), |peer| peer.discovered)
            .await;
        assert!(
            self.discovered,
            "the session never discovered its Device MCP tools"
        );
    }

    async fn send(&mut self, value: serde_json::Value) {
        self.socket
            .send(Message::Text(value.to_string().into()))
            .await
            .unwrap();
    }

    async fn run_turn(&mut self) {
        self.wait_for_discovery().await;
        self.send(
            serde_json::json!({"type":"listen","session_id":self.session_id,
            "state":"start","mode":"manual"}),
        )
        .await;
        self.send(
            serde_json::json!({"type":"listen","session_id":self.session_id,
            "state":"detect","text":"utterance"}),
        )
        .await;
    }

    async fn abort(&mut self) {
        let session_id = self.session_id.clone();
        self.send(serde_json::json!({"type":"abort","session_id":session_id}))
            .await;
    }

    /// Reads until the turn's terminal `tts:stop`, answering every Device MCP request on the way.
    async fn wait_for_turn_end(&mut self) {
        self.pump_until(Duration::from_secs(5), |peer| peer.turn_closed)
            .await
    }

    /// Reads for a bounded while without requiring the turn to close, for the tests whose whole
    /// point is that nothing is delivered.
    async fn pump(&mut self, within: Duration) {
        self.pump_until(within, |_| false).await
    }

    async fn pump_until(&mut self, budget: Duration, done: impl Fn(&Self) -> bool) {
        let deadline = tokio::time::Instant::now() + budget;
        loop {
            if done(self) {
                return;
            }
            let Ok(Some(Ok(frame))) = tokio::time::timeout_at(deadline, self.socket.next()).await
            else {
                return;
            };
            let Message::Text(text) = frame else { continue };
            let Ok(value) = serde_json::from_str::<serde_json::Value>(&text) else {
                continue;
            };
            match value["type"].as_str() {
                Some("mcp") => self.answer_device_mcp(&value).await,
                Some("llm") => {
                    if let Some(text) = value["text"].as_str() {
                        self.spoken.push(text.to_owned());
                    }
                }
                Some("tts") if value["state"] == "stop" => self.turn_closed = true,
                _ => {}
            }
        }
    }

    async fn answer_device_mcp(&mut self, value: &serde_json::Value) {
        let payload = &value["payload"];
        let id = payload["id"].clone();
        let result = match payload["method"].as_str() {
            Some("initialize") => serde_json::json!({
                "protocolVersion": "2024-11-05",
                "capabilities": {"tools": {}},
                "serverInfo": {"name": "peer", "version": "1"}
            }),
            Some("tools/list") => {
                self.discovered = true;
                serde_json::json!({"tools": [peer_tool(DEVICE_TOOL)]})
            }
            Some("tools/call") => {
                let name = payload["params"]["name"]
                    .as_str()
                    .unwrap_or_default()
                    .to_owned();
                self.device_calls.push(DeviceCall {
                    name: name.clone(),
                    arguments: payload["params"]["arguments"].clone(),
                });
                if !self.answer_device_calls {
                    return;
                }
                serde_json::json!({
                    "content": [{"type": "text", "text": format!("device:{name}")}],
                    "isError": false
                })
            }
            _ => serde_json::json!({}),
        };
        self.send(serde_json::json!({
            "type": "mcp", "session_id": self.session_id,
            "payload": {"jsonrpc": "2.0", "id": id, "result": result}
        }))
        .await;
    }
}

fn peer_tool(name: &str) -> serde_json::Value {
    serde_json::json!({
        "name": name,
        "description": "device tool",
        "inputSchema": {"type": "object", "properties": {}, "additionalProperties": false}
    })
}

async fn next_text(socket: &mut Socket, what: &str) -> String {
    loop {
        let frame = timeout(Duration::from_secs(5), socket.next())
            .await
            .unwrap_or_else(|_| panic!("{what} arrives within its own budget"))
            .expect("the socket stays open")
            .expect("the socket has no error");
        if let Message::Text(text) = frame {
            return text.to_string();
        }
    }
}

// ---------------------------------------------------------------------------
// Assertions over what the continuation was given
// ---------------------------------------------------------------------------

/// The completed tool pairs of one request, in the order the continuation received them.
fn completed_pairs(messages: &[ChatMessage]) -> Vec<(String, String)> {
    messages
        .iter()
        .filter_map(|message| match message {
            ChatMessage::ToolResult {
                tool_call_id,
                content,
            } => Some((tool_call_id.clone(), content.clone())),
            _ => None,
        })
        .collect()
}

fn pair_ids(pairs: &[(String, String)]) -> Vec<&str> {
    pairs.iter().map(|(id, _)| id.as_str()).collect()
}

/// Every `AssistantToolCalls` the continuation received, in order.
fn assistant_call_ids(messages: &[ChatMessage]) -> Vec<Vec<String>> {
    messages
        .iter()
        .filter_map(|message| match message {
            ChatMessage::AssistantToolCalls { calls } => {
                Some(calls.iter().map(|call| call.id.clone()).collect::<Vec<_>>())
            }
            _ => None,
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Ordering across the origins one executor runs
// ---------------------------------------------------------------------------

/// One executor runs a Device MCP call and an External MCP call in the exact order the model
/// returned them, and the continuation is handed back one paired result per call, in that order.
#[tokio::test]
async fn a_mixed_round_runs_its_calls_in_model_order_and_pairs_every_result() {
    let external =
        ExternalServer::start(ExternalScript::with_tools(vec![external_tool("Forecast")])).await;
    let (llm, observed) = ScriptedLlm::new(vec![
        Round::ToolCalls(vec![
            (
                "call-1",
                DEVICE_LLM_NAME,
                serde_json::json!({"brightness": 1}),
            ),
            (
                "call-2",
                EXTERNAL_LLM_NAME,
                serde_json::json!({"city": "Hanoi"}),
            ),
        ]),
        Round::Text("the lamp is on and it is 21 degrees"),
    ]);
    let url = database_url();
    let voice = start(config(url.clone(), LlmToolsConfig::default()), llm).await;
    seed(&url, &external.url).await;

    let mut peer = Peer::open(&voice.base).await;
    peer.run_turn().await;
    peer.wait_for_turn_end().await;

    assert_eq!(
        peer.device_calls,
        vec![DeviceCall {
            name: DEVICE_TOOL.to_owned(),
            arguments: serde_json::json!({"brightness": 1}),
        }],
        "a Device MCP call is made once, with the original wire name the model never sees"
    );
    assert_eq!(
        external.script().calls(),
        vec!["Forecast"],
        "an External MCP call is made with its original wire name"
    );
    assert!(peer.turn_closed, "the turn reached its normal boundary");

    let observed = observed
        .lock()
        .expect("the observation mailbox is not poisoned");
    // Both origins are advertised in one round, which is the only reason the round was buffered.
    let offered = &observed.offered[0];
    assert!(offered.contains(&DEVICE_LLM_NAME.to_owned()), "{offered:?}");
    assert!(
        offered.contains(&EXTERNAL_LLM_NAME.to_owned()),
        "{offered:?}"
    );

    let continuation = &observed.requests[1];
    assert_eq!(
        assistant_call_ids(continuation),
        vec![vec!["call-1".to_owned(), "call-2".to_owned()]],
        "the whole round is replayed to the model in the order it asked for it"
    );
    let pairs = completed_pairs(continuation);
    assert_eq!(
        pair_ids(&pairs),
        vec!["call-1", "call-2"],
        "each result keeps its own tool_call_id and its position"
    );
    assert!(pairs[0].1.contains("device:self.lamp.on"), "{}", pairs[0].1);
    assert!(pairs[1].1.contains("external:Forecast"), "{}", pairs[1].1);
    assert!(
        pairs
            .iter()
            .all(|(_, content)| !content.contains("s3cr3t-bearer")),
        "no result carries a credential"
    );
}

/// The same executor also runs a session-local action, and the exit action's direct speech is
/// spoken only after every call before it in the model order has finished.  The exit action is the
/// session-local builtin the switch action shares its dispatch arm with.
#[tokio::test]
async fn a_session_local_action_runs_in_the_same_order_as_the_mcp_calls_around_it() {
    let external =
        ExternalServer::start(ExternalScript::with_tools(vec![external_tool("Forecast")])).await;
    let (llm, observed) = ScriptedLlm::new(vec![Round::ToolCalls(vec![
        ("call-1", EXTERNAL_LLM_NAME, serde_json::json!({})),
        (
            "call-2",
            EXIT_TOOL_NAME,
            serde_json::json!({"say_goodbye": "Tạm biệt!"}),
        ),
        ("call-3", DEVICE_LLM_NAME, serde_json::json!({})),
    ])]);
    let url = database_url();
    let voice = start(config(url.clone(), LlmToolsConfig::default()), llm).await;
    seed(&url, &external.url).await;

    let mut peer = Peer::open(&voice.base).await;
    peer.run_turn().await;
    peer.wait_for_turn_end().await;

    assert_eq!(
        external.script().calls(),
        vec!["Forecast"],
        "the call in front of the action runs before the action"
    );
    assert_eq!(
        peer.device_calls,
        vec![DeviceCall {
            name: DEVICE_TOOL.to_owned(),
            arguments: serde_json::json!({}),
        }],
        "the call the model placed after the action still runs, because model order is the whole \
         order and the action only decides what the client hears"
    );
    assert!(
        peer.spoken.iter().any(|text| text.contains("Tạm biệt")),
        "the action's own answer is what the client hears: {:?}",
        peer.spoken
    );
    let observed = observed
        .lock()
        .expect("the observation mailbox is not poisoned");
    assert_eq!(
        observed.requests.len(),
        1,
        "a direct answer ends the turn, so no continuation is built behind it"
    );
}

// ---------------------------------------------------------------------------
// Caps
// ---------------------------------------------------------------------------

/// The whole round is checked before call one, so a model that asks for more calls than the
/// deployment allows has none of them executed — not even the ones that came first in the list.
#[tokio::test]
async fn a_round_over_the_call_cap_executes_no_call_at_all() {
    let external =
        ExternalServer::start(ExternalScript::with_tools(vec![external_tool("Forecast")])).await;
    let (llm, observed) = ScriptedLlm::new(vec![Round::ToolCalls(vec![
        ("call-1", DEVICE_LLM_NAME, serde_json::json!({})),
        ("call-2", DEVICE_LLM_NAME, serde_json::json!({})),
        ("call-3", EXTERNAL_LLM_NAME, serde_json::json!({})),
    ])]);
    let url = database_url();
    let voice = start(
        config(
            url.clone(),
            LlmToolsConfig {
                max_calls_per_round: 2,
                ..Default::default()
            },
        ),
        llm,
    )
    .await;
    seed(&url, &external.url).await;

    let mut peer = Peer::open(&voice.base).await;
    peer.run_turn().await;
    peer.pump(Duration::from_millis(600)).await;

    assert!(
        peer.device_calls.is_empty(),
        "the first call in the list must not run merely because it came first: {:?}",
        peer.device_calls
    );
    assert!(
        external.script().calls().is_empty(),
        "an over-cap round sends nothing to an External MCP server either"
    );
    let observed = observed
        .lock()
        .expect("the observation mailbox is not poisoned");
    assert_eq!(
        observed.requests.len(),
        1,
        "the turn ends without a continuation, so no AssistantToolCall is left dangling"
    );
    assert!(
        peer.spoken.is_empty(),
        "a refused round produces no speech: {:?}",
        peer.spoken
    );
}

/// The round allowance is checked before the next round's request is sent, so a model that keeps
/// asking for tools runs out of rounds rather than running forever.
#[tokio::test]
async fn a_turn_over_the_round_cap_stops_before_another_round_starts() {
    let external =
        ExternalServer::start(ExternalScript::with_tools(vec![external_tool("Forecast")])).await;
    let (llm, observed) = ScriptedLlm::new(vec![
        Round::ToolCalls(vec![("call-1", DEVICE_LLM_NAME, serde_json::json!({}))]),
        Round::ToolCalls(vec![("call-2", DEVICE_LLM_NAME, serde_json::json!({}))]),
        Round::ToolCalls(vec![("call-3", DEVICE_LLM_NAME, serde_json::json!({}))]),
    ]);
    let url = database_url();
    let voice = start(
        config(
            url.clone(),
            LlmToolsConfig {
                max_rounds_per_turn: 1,
                ..Default::default()
            },
        ),
        llm,
    )
    .await;
    seed(&url, &external.url).await;

    let mut peer = Peer::open(&voice.base).await;
    peer.run_turn().await;
    peer.pump(Duration::from_millis(600)).await;

    let observed = observed
        .lock()
        .expect("the observation mailbox is not poisoned");
    assert_eq!(
        observed.requests.len(),
        2,
        "one tool round ran, and exactly one continuation was allowed to start"
    );
    let completed = completed_pairs(&observed.requests[1]);
    assert_eq!(
        pair_ids(&completed),
        vec!["call-1"],
        "the completed round is kept, so the turn's history still holds its paired result"
    );
}

/// The Tool Execution Budget is spent from the first call of the turn, and a call that outlives it
/// ends the turn rather than being continued.
///
/// The Device MCP origin is here for the same reason the External one is: the budget belongs to the
/// turn, not to a transport, so a call that spends it must stop the turn whichever transport spent
/// it.  The peer never answers, so what expires is the turn's time to use the request rather than
/// anything the transport did.
#[tokio::test]
async fn a_device_call_that_outlives_the_execution_budget_ends_the_turn_too() {
    let external =
        ExternalServer::start(ExternalScript::with_tools(vec![external_tool("Forecast")])).await;
    let (llm, observed) = ScriptedLlm::new(vec![
        Round::ToolCalls(vec![("call-1", DEVICE_LLM_NAME, serde_json::json!({}))]),
        Round::Text("never reached"),
    ]);
    let url = database_url();
    let voice = start(
        config(
            url.clone(),
            LlmToolsConfig {
                execution_budget_ms: 250,
                ..Default::default()
            },
        ),
        llm,
    )
    .await;
    seed(&url, &external.url).await;

    let mut peer = Peer::open(&voice.base).await;
    peer.answer_device_calls = false;
    peer.run_turn().await;
    peer.pump(Duration::from_secs(2)).await;

    assert_eq!(
        peer.device_calls.len(),
        1,
        "the call started inside the budget, so it was sent"
    );
    let observed = observed
        .lock()
        .expect("the observation mailbox is not poisoned");
    assert_eq!(
        observed.requests.len(),
        1,
        "a turn that ran out of budget gets no continuation, whichever transport spent it"
    );
    assert!(
        peer.spoken.is_empty(),
        "a turn that ran out of budget speaks nothing: {:?}",
        peer.spoken
    );
}

/// The same budget, spent by the other origin.
///
/// The External origin is here because the budget is the turn's, not a transport's: a call that
/// spends it must stop the turn whichever transport spent it, and a test that only ever exhausted
/// it one way would not notice if the other way diverged.
#[tokio::test]
async fn an_external_call_that_outlives_the_execution_budget_ends_the_turn() {
    let gate = CallGate::new();
    let external = ExternalServer::start(ExternalScript {
        gate: Some(gate.clone()),
        ..ExternalScript::with_tools(vec![external_tool("Forecast")])
    })
    .await;
    let (llm, observed) = ScriptedLlm::new(vec![
        Round::ToolCalls(vec![("call-1", EXTERNAL_LLM_NAME, serde_json::json!({}))]),
        Round::Text("never reached"),
    ]);
    let url = database_url();
    let voice = start(
        config(
            url.clone(),
            LlmToolsConfig {
                execution_budget_ms: 250,
                ..Default::default()
            },
        ),
        llm,
    )
    .await;
    seed(&url, &external.url).await;

    let mut peer = Peer::open(&voice.base).await;
    peer.run_turn().await;
    // The gate is opened only after the budget has had time to expire, so the call cannot be
    // answered inside it.  The request still went out, which is what makes this a call that was
    // started and then outlived its turn rather than one that was refused before it started.
    gate.wait_until_held(1).await;
    tokio::time::sleep(Duration::from_millis(600)).await;
    gate.open();
    peer.pump(Duration::from_millis(600)).await;

    assert_eq!(
        external.script().calls(),
        vec!["Forecast"],
        "the call started inside the budget, so it was sent"
    );
    let observed = observed
        .lock()
        .expect("the observation mailbox is not poisoned");
    assert_eq!(
        observed.requests.len(),
        1,
        "a turn that ran out of budget gets no continuation"
    );
    assert!(
        peer.spoken.is_empty(),
        "a turn that ran out of budget speaks nothing: {:?}",
        peer.spoken
    );
}

// ---------------------------------------------------------------------------
// Concurrency
// ---------------------------------------------------------------------------

/// Two sessions calling one server acquire from the process-global per-server bound, so one
/// session's call waits for a permit instead of a second call opening beside it.
#[tokio::test]
async fn two_sessions_calling_one_server_never_exceed_the_shared_per_server_bound() {
    let gate = CallGate::new();
    let external = ExternalServer::start(ExternalScript {
        gate: Some(gate.clone()),
        ..ExternalScript::with_tools(vec![external_tool("Forecast")])
    })
    .await;
    let (llm, _) = ScriptedLlm::new(vec![
        Round::ToolCalls(vec![("call-1", EXTERNAL_LLM_NAME, serde_json::json!({}))]),
        Round::ToolCalls(vec![("call-2", EXTERNAL_LLM_NAME, serde_json::json!({}))]),
    ]);
    let url = database_url();
    let voice = start(
        config_with_external(
            url.clone(),
            LlmToolsConfig {
                // Long enough that a call waiting for a permit is waited on rather than refused,
                // so what bounds the fan-out is the server's own concurrency and nothing else.
                execution_budget_ms: 30_000,
                ..Default::default()
            },
            ExternalMcpConfig {
                max_concurrent_calls_per_server: 1,
                ..ExternalMcpConfig::default()
            },
        ),
        llm,
    )
    .await;
    seed(&url, &external.url).await;
    let script = external.script();

    let mut first = Peer::open(&voice.base).await;
    first.run_turn().await;
    gate.wait_until_held(1).await;

    let mut second = Peer::open(&voice.base).await;
    second.run_turn().await;
    tokio::time::sleep(Duration::from_millis(200)).await;

    assert_eq!(
        script.calls().len(),
        1,
        "the second session's call waits for the first one's permit rather than opening a second"
    );
    assert_eq!(
        script.peak_concurrency(),
        1,
        "the process-global per-server bound is what a call acquires from"
    );

    gate.open();
    // The gate stays open, so the second call runs as soon as the first releases its permit.
    gate.wait_until_held(2).await;
    first.pump(Duration::from_millis(300)).await;
    second.pump(Duration::from_millis(300)).await;
    assert_eq!(
        script.calls().len(),
        2,
        "the second call runs once the first has released its permit"
    );
    assert_eq!(
        script.peak_concurrency(),
        1,
        "the bound held for the whole turn, not only for the first call"
    );
}

// ---------------------------------------------------------------------------
// Cancellation
// ---------------------------------------------------------------------------

/// An interrupted turn starts no further call and produces no ToolResult, no continuation and no
/// speech.  The call it had in flight is dropped, so the response the server was still holding has
/// nowhere to go.
#[tokio::test]
async fn an_interrupted_turn_drops_its_in_flight_call_and_never_continues() {
    let gate = CallGate::new();
    let external = ExternalServer::start(ExternalScript {
        gate: Some(gate.clone()),
        ..ExternalScript::with_tools(vec![external_tool("Forecast")])
    })
    .await;
    let (llm, observed) = ScriptedLlm::new(vec![
        Round::ToolCalls(vec![
            ("call-1", EXTERNAL_LLM_NAME, serde_json::json!({})),
            ("call-2", DEVICE_LLM_NAME, serde_json::json!({})),
        ]),
        Round::Text("never reached"),
    ]);
    let url = database_url();
    let voice = start(config(url.clone(), LlmToolsConfig::default()), llm).await;
    seed(&url, &external.url).await;

    let mut peer = Peer::open(&voice.base).await;
    peer.run_turn().await;
    // Wait for the call to actually be in flight before interrupting, so this measures a
    // cancellation of a live call rather than of a round that never started one.
    gate.wait_until_held(1).await;
    peer.abort().await;
    tokio::time::sleep(Duration::from_millis(100)).await;
    gate.open();
    peer.pump(Duration::from_millis(500)).await;

    assert!(
        peer.device_calls.is_empty(),
        "the call the model placed behind the cancelled one never starts"
    );
    assert!(
        external.script().calls().len() <= 1,
        "no retry follows the cancelled call"
    );
    let observed = observed
        .lock()
        .expect("the observation mailbox is not poisoned");
    assert_eq!(
        observed.requests.len(),
        1,
        "an interrupted turn never asks the model anything else, so no ToolResult is dangling"
    );
    assert!(
        peer.spoken.is_empty(),
        "an interrupted turn speaks nothing: {:?}",
        peer.spoken
    );
    assert!(
        !peer.turn_closed,
        "an interrupted turn never reaches a normal boundary, so a half-finished exchange is not \
         reported as a completed turn"
    );
}

// ---------------------------------------------------------------------------
// Controlled failure
// ---------------------------------------------------------------------------

/// A refusal from the remote is a fact about one call: it terminalizes that call with a typed,
/// content-free result, the round's remaining calls still run, and the turn continues.
#[tokio::test]
async fn a_refused_external_call_is_typed_and_does_not_stop_the_round() {
    let external = ExternalServer::start(ExternalScript {
        call_status: Some(401),
        ..ExternalScript::with_tools(vec![external_tool("Forecast")])
    })
    .await;
    let (llm, observed) = ScriptedLlm::new(vec![
        Round::ToolCalls(vec![
            ("call-1", EXTERNAL_LLM_NAME, serde_json::json!({})),
            ("call-2", DEVICE_LLM_NAME, serde_json::json!({})),
        ]),
        Round::Text("the lamp is on and the forecast is unavailable"),
    ]);
    let url = database_url();
    let voice = start(config(url.clone(), LlmToolsConfig::default()), llm).await;
    seed(&url, &external.url).await;

    let mut peer = Peer::open(&voice.base).await;
    peer.run_turn().await;
    peer.wait_for_turn_end().await;

    assert_eq!(
        peer.device_calls.len(),
        1,
        "a refused call does not stop the sibling call behind it in the same round"
    );
    assert!(
        peer.turn_closed,
        "a refused call is not a reason to close the Voice Session"
    );
    let observed = observed
        .lock()
        .expect("the observation mailbox is not poisoned");
    let pairs = completed_pairs(&observed.requests[1]);
    assert_eq!(pair_ids(&pairs), vec!["call-1", "call-2"]);
    assert_eq!(
        pairs[0].1, r#"{"error":"external_tool_auth_failed"}"#,
        "the result names the bounded class and nothing about what the server said"
    );
    assert!(pairs[1].1.contains("device:self.lamp.on"), "{}", pairs[1].1);
    assert_eq!(
        external.script().calls(),
        vec!["Forecast"],
        "one attempt, and no retry after the refusal"
    );
}

// ---------------------------------------------------------------------------
// Fixture sanity
// ---------------------------------------------------------------------------

/// A guard on the harness itself: without a Device MCP peer that answers, the ordering and cap
/// tests above would pass for the wrong reason.
#[tokio::test]
async fn the_peer_really_is_a_device_mcp_server_and_the_external_catalog_really_resolves() {
    let external =
        ExternalServer::start(ExternalScript::with_tools(vec![external_tool("Forecast")])).await;
    let (llm, observed) = ScriptedLlm::new(vec![
        Round::ToolCalls(vec![("call-1", DEVICE_LLM_NAME, serde_json::json!({}))]),
        Round::Text("on"),
    ]);
    let url = database_url();
    let voice = start(config(url.clone(), LlmToolsConfig::default()), llm).await;
    seed(&url, &external.url).await;

    let mut peer = Peer::open(&voice.base).await;
    peer.run_turn().await;
    peer.wait_for_turn_end().await;

    assert_eq!(peer.device_calls.len(), 1);
    let observed = observed
        .lock()
        .expect("the observation mailbox is not poisoned");
    let offered = &observed.offered[0];
    assert_eq!(
        offered
            .iter()
            .filter(|name| *name == DEVICE_LLM_NAME)
            .count(),
        1,
        "a Device MCP tool is advertised exactly once"
    );
    assert!(
        offered.contains(&EXTERNAL_LLM_NAME.to_owned()),
        "the External MCP catalog this session was admitted with is advertised too: {offered:?}"
    );
}
