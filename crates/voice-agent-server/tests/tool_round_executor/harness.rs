use std::{
    net::SocketAddr,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

pub use axum::{
    Router,
    body::Bytes,
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::post,
};
pub use futures_util::{SinkExt, StreamExt, stream};
pub use sqlx::sqlite::SqlitePoolOptions;
pub use tokio::{
    net::TcpListener,
    sync::{Notify, watch},
    task::JoinHandle,
    time::timeout,
};
pub use tokio_tungstenite::{
    connect_async,
    tungstenite::{Message, client::IntoClientRequest},
};
pub use url::Url;
pub use voice_agent_server::{
    app::{AppState, router_with_state},
    audio::PcmF32Mono,
    config::{
        AppConfig, AudioConfig, AuthConfig, BargeInConfig, DatabaseConfig, DeploymentConfig,
        ExternalMcpConfig, ExternalMcpLimitsConfig, ExternalMcpNetworkConfig, LimitsConfig,
        LlmConfig, LlmToolsConfig, McpConfig, ProviderDefaultsConfig, ProvidersConfig,
        RuntimeConfig, ServerConfig, SileroOnnxConfig, SpeechOutputConfig, TtsConfig,
        VadInstanceConfig, VisionConfig, WebsocketConfig, WorkersConfig,
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

pub const DEVICE_TOOL: &str = "self.lamp.on";
/// The Device MCP name the model sees: sanitization cannot produce a dot, so this is unambiguous.
pub const DEVICE_LLM_NAME: &str = "self_lamp_on";
/// The published External MCP name: a fixed namespace plus the normalized server and tool.
pub const EXTERNAL_LLM_NAME: &str = "external.weather.forecast";

// ---------------------------------------------------------------------------
// Scripted LLM
// ---------------------------------------------------------------------------

/// One LLM round the scripted provider returns.
///
/// `ToolCalls` returns calls and nothing else; `Text` returns a final answer.  A turn's script is
/// consumed in order, so a test states exactly what the model does instead of inferring it from
/// what the executor happens to do.
#[derive(Clone)]
pub enum Round {
    ToolCalls(Vec<(&'static str, &'static str, serde_json::Value)>),
    Text(&'static str),
}

/// Everything a scripted provider observed, so a test can assert what the continuation was handed
/// rather than only that something was sent.
#[derive(Default)]
pub struct Observed {
    /// The tool names offered on each round.
    pub offered: Vec<Vec<String>>,
    /// Each round's incoming messages, which is where a completed tool round must appear paired.
    pub requests: Vec<Vec<ChatMessage>>,
}

pub struct ScriptedLlm {
    script: Arc<Mutex<Vec<Round>>>,
    observed: Arc<Mutex<Observed>>,
}

impl ScriptedLlm {
    pub fn new(script: Vec<Round>) -> (Self, Arc<Mutex<Observed>>) {
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

pub struct FinalAsr;

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

pub struct SilentVad;

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

pub struct ShortTts;

impl TtsProvider for ShortTts {
    fn adapter(&self) -> &'static str {
        "short-tts"
    }
    fn synthesize(&self, _: &str) -> Result<PcmF32Mono, TtsError> {
        Ok(PcmF32Mono::new(vec![0.1; 2_400], 48_000))
    }
}

pub struct ConstantSecrets(pub Option<String>);

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
pub struct CallGate {
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
    pub fn new() -> Self {
        let (opener, open) = watch::channel(false);
        Self {
            held: Arc::new(Notify::new()),
            held_calls: Arc::new(AtomicUsize::new(0)),
            open,
            opener,
        }
    }

    /// The server side: report that a call is now held, then wait for the test to open the gate.
    pub async fn hold(&self) {
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
    pub async fn wait_until_held(&self, count: usize) {
        timeout(Duration::from_secs(5), async {
            while self.held_calls.load(Ordering::SeqCst) < count {
                self.held.notified().await;
            }
        })
        .await
        .expect("the reviewed external call reaches the gate");
    }

    pub fn open(&self) {
        let _ = self.opener.send(true);
    }
}

/// What the fake External MCP server observed, in the order it observed it.
#[derive(Clone)]
pub struct ExternalScript {
    pub tools: Vec<serde_json::Value>,
    /// Answer `tools/call` with this HTTP status instead of a result.
    pub call_status: Option<u16>,
    pub gate: Option<CallGate>,
    pub calls: Arc<Mutex<Vec<String>>>,
    pub concurrent: Arc<Mutex<(usize, usize)>>,
}

impl ExternalScript {
    pub fn with_tools(tools: Vec<serde_json::Value>) -> Self {
        Self {
            tools,
            call_status: None,
            gate: None,
            calls: Arc::new(Mutex::new(Vec::new())),
            concurrent: Arc::new(Mutex::new((0, 0))),
        }
    }

    pub fn calls(&self) -> Vec<String> {
        self.calls
            .lock()
            .expect("the script mailbox is not poisoned")
            .clone()
    }

    /// The highest number of `tools/call` requests the server ever had open at once.
    pub fn peak_concurrency(&self) -> usize {
        self.concurrent
            .lock()
            .expect("the script mailbox is not poisoned")
            .1
    }
}

pub type SharedExternal = Arc<Mutex<ExternalScript>>;

pub fn external_tool(name: &str) -> serde_json::Value {
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

pub struct ExternalServer {
    pub url: String,
    pub script: SharedExternal,
    #[allow(dead_code)]
    pub task: JoinHandle<()>,
}

impl ExternalServer {
    pub async fn start(script: ExternalScript) -> Self {
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

    pub fn script(&self) -> ExternalScript {
        self.script
            .lock()
            .expect("the script mailbox is not poisoned")
            .clone()
    }
}

// ---------------------------------------------------------------------------
// Voice server
// ---------------------------------------------------------------------------

pub fn database_url() -> String {
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

pub fn config(url: String, tools: LlmToolsConfig) -> AppConfig {
    config_with_external(url, tools, ExternalMcpConfig::default())
}

pub fn config_with_external(
    url: String,
    tools: LlmToolsConfig,
    external: ExternalMcpConfig,
) -> AppConfig {
    let address: SocketAddr = ([127, 0, 0, 1], 0).into();
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
        llm: LlmConfig {
            tools,
            ..LlmConfig::default()
        },
        tts: TtsConfig::default(),
        speech_output: SpeechOutputConfig::default(),
        barge_in: BargeInConfig::default(),
        system_mcp: Default::default(),
        mcp: McpConfig {
            external: ExternalMcpConfig {
                network: ExternalMcpNetworkConfig {
                    allowed_hosts: vec![],
                },
                limits: ExternalMcpLimitsConfig::default(),
                ..external
            },
            ..McpConfig::default()
        },
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

pub struct Voice {
    pub base: String,
    #[allow(dead_code)]
    pub task: JoinHandle<()>,
}

pub async fn start(mut app_config: AppConfig, llm: ScriptedLlm) -> Voice {
    app_config.api.enabled = true;
    app_config.api.admin_token = "review-token".into();
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
        voice_agent_server::lifecycle::RuntimeLifecycle::new(std::time::Duration::from_millis(
            1_024,
        )),
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
pub async fn seed(url: &str, external_url: &str) {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect(url)
        .await
        .unwrap();
    sqlx::query("INSERT INTO agents (key,name,enabled,created_at,updated_at) VALUES ('agent','Agent',1,1,1)")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO devices (device_id,agent_id,enabled,created_at,updated_at) VALUES ('device',1,1,1,1), ('second',1,1,1,1)")
        .execute(&pool)
        .await
        .unwrap();
    let server_id: i64 = sqlx::query_scalar(
        "INSERT INTO mcp_servers (key,name,url,headers_json,auth_type,auth_header_name,\
         connect_timeout_ms,request_timeout_ms,enabled,created_at,updated_at) \
         VALUES ('weather','Weather',?,'{}','bearer',NULL,5000,5000,1,1,1) RETURNING id",
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

pub async fn seed_reviewed(base: &str, url: &str, external_url: &str) {
    seed(url, external_url).await;
    approve_external_tool(base).await;
}

pub async fn approve_external_tool(base: &str) {
    // Discovery records the contract; approval takes effect on the next WebSocket.
    let observer = Peer::open(base).await;
    drop(observer);
    let client = reqwest::Client::new();
    let path = format!("{base}/api/admin/agents/agent/tool-allowlist");
    let observation = client
        .get(&path)
        .bearer_auth("review-token")
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json::<serde_json::Value>()
        .await
        .unwrap();
    let contract = &observation["items"][0];
    assert_eq!(contract["original_name"], "Forecast");
    let reviewed = client
        .put(&path)
        .bearer_auth("review-token")
        .header("If-Match", "\"1\"")
        .json(&serde_json::json!({
            "server_key": "weather", "original_name": "Forecast",
            "observed_revision": contract["observed_revision"],
            "fingerprint": contract["fingerprint"], "allowed": true, "sensitive": false
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(reviewed.status(), reqwest::StatusCode::OK);
}

// ---------------------------------------------------------------------------
// The Voice Protocol Client, also playing the Device MCP peer
// ---------------------------------------------------------------------------

/// One Device MCP `tools/call` this peer was actually asked to make, in order.
#[derive(Clone, Debug, PartialEq)]
pub struct DeviceCall {
    pub name: String,
    pub arguments: serde_json::Value,
}

pub type Socket =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

pub struct Peer {
    pub socket: Socket,
    pub session_id: String,
    pub device_calls: Vec<DeviceCall>,
    /// Every `llm` text segment the client was shown, in order.
    pub spoken: Vec<String>,
    pub direct_spoken: Vec<String>,
    pub playback: Vec<&'static str>,
    pub turn_closed: bool,
    /// When false, the peer records each `tools/call` and never answers it, so a test can watch a
    /// turn run out of time on a request that really was made.
    pub answer_device_calls: bool,
    /// Set once this peer has answered `tools/list`, which is when the session's Device MCP
    /// catalog exists.  A real client waits for discovery before it speaks, and so must the
    /// harness, or a turn would be built against a catalog that had not arrived yet.
    pub discovered: bool,
}

impl Peer {
    pub async fn open(base: &str) -> Self {
        Self::open_device(base, "device").await
    }

    pub async fn open_device(base: &str, device_id: &str) -> Self {
        let ota: serde_json::Value = reqwest::Client::new()
            .post(format!("{base}/voice/ota/"))
            .header("Device-Id", device_id)
            .header("Client-Id", "tool-round")
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
            headers.insert("Device-Id", device_id.parse().unwrap());
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
            direct_spoken: Vec::new(),
            playback: Vec::new(),
            turn_closed: false,
            answer_device_calls: true,
            discovered: false,
        }
    }

    /// Answers the server's discovery until the session's Device MCP catalog exists.
    pub async fn wait_for_discovery(&mut self) {
        self.pump_until(Duration::from_secs(5), |peer| peer.discovered)
            .await;
        assert!(
            self.discovered,
            "the session never discovered its Device MCP tools"
        );
    }

    pub async fn send(&mut self, value: serde_json::Value) {
        self.socket
            .send(Message::Text(value.to_string().into()))
            .await
            .unwrap();
    }

    pub async fn run_turn(&mut self) {
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

    pub async fn abort(&mut self) {
        let session_id = self.session_id.clone();
        self.send(serde_json::json!({"type":"abort","session_id":session_id}))
            .await;
    }

    /// Reads until the turn's terminal `tts:stop`, answering every Device MCP request on the way.
    pub async fn wait_for_turn_end(&mut self) {
        self.pump_until(Duration::from_secs(5), |peer| peer.turn_closed)
            .await
    }

    /// Reads for a bounded while without requiring the turn to close, for the tests whose whole
    /// point is that nothing is delivered.
    pub async fn pump(&mut self, within: Duration) {
        self.pump_until(within, |_| false).await
    }

    pub async fn pump_until(&mut self, budget: Duration, done: impl Fn(&Self) -> bool) {
        let deadline = tokio::time::Instant::now() + budget;
        loop {
            if done(self) {
                return;
            }
            let Ok(Some(Ok(frame))) = tokio::time::timeout_at(deadline, self.socket.next()).await
            else {
                return;
            };
            if let Message::Binary(packet) = &frame {
                let mut decoder = opus2::Decoder::new(24_000, opus2::Channels::Mono).unwrap();
                assert_eq!(
                    decoder.decode(packet, &mut [0_i16; 1_440], false).unwrap(),
                    1_440
                );
                self.playback.push("audio");
            }
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
                Some("tts") if value["state"] == "start" => self.playback.push("start"),
                Some("tts") if value["state"] == "sentence_start" => {
                    self.playback.push("sentence_start");
                    if let Some(text) = value["text"].as_str() {
                        self.direct_spoken.push(text.to_owned());
                    }
                }
                Some("tts") if value["state"] == "stop" => {
                    self.playback.push("stop");
                    self.turn_closed = true;
                }
                _ => {}
            }
        }
    }

    pub async fn answer_device_mcp(&mut self, value: &serde_json::Value) {
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

pub fn peer_tool(name: &str) -> serde_json::Value {
    serde_json::json!({
        "name": name,
        "description": "device tool",
        "inputSchema": {"type": "object", "properties": {}, "additionalProperties": false}
    })
}

pub async fn next_text(socket: &mut Socket, what: &str) -> String {
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
pub fn completed_pairs(messages: &[ChatMessage]) -> Vec<(String, String)> {
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

pub fn pair_ids(pairs: &[(String, String)]) -> Vec<&str> {
    pairs.iter().map(|(id, _)| id.as_str()).collect()
}

/// Every `AssistantToolCalls` the continuation received, in order.
pub fn assistant_call_ids(messages: &[ChatMessage]) -> Vec<Vec<String>> {
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
