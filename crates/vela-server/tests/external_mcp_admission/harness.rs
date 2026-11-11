pub use std::{
    sync::{Arc, Mutex},
    time::Duration,
};

pub use axum::{
    Router,
    body::Bytes,
    extract::State,
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::post,
};
pub use sqlx::SqlitePool;
pub use tokio::{net::TcpListener, task::JoinHandle, time::timeout};
pub use tokio_tungstenite::{
    connect_async,
    tungstenite::{client::IntoClientRequest, http::StatusCode as WsStatusCode},
};
pub use url::Url;
pub use vela_server::audio::PcmF32Mono;
pub use vela_server::{
    app::AppState,
    config::{
        AppConfig, AudioConfig, AuthConfig, BargeInConfig, DatabaseConfig, DeploymentConfig,
        ExternalMcpConfig, ExternalMcpLimitsConfig, ExternalMcpNetworkConfig, LimitsConfig,
        LlmConfig, McpConfig, ProviderDefaultsConfig, ProvidersConfig, RuntimeConfig, ServerConfig,
        SileroOnnxConfig, SpeechOutputConfig, TtsConfig, VadInstanceConfig, VisionConfig,
        WebsocketConfig, WorkersConfig,
    },
    database::{
        Database,
        secrets::{SecretRef, SecretResolveError, SecretResolver, SecretValue},
    },
    providers::{
        AsrError, AsrEvent, AsrProvider, AsrResult, AsrSession, LlmError, LlmProvider, ProviderSet,
        TtsError, TtsProvider, VadError, VadInput, VadProbability, VadProvider, VadSession,
        llm::LlmRequest,
    },
    session::EffectiveSessionProfile,
    telemetry::{
        EXTERNAL_MCP_SESSION_TOOL_CAP_EXCEEDED_TOTAL, MCP_RESOLVE_FAILURE_TOTAL,
        MCP_RESOLVE_SUCCESS_TOTAL, RecordingTelemetry,
    },
    tools::external_mcp::{ExternalMcpExclusionReason, ExternalMcpManager, SessionExternalMcp},
};

// ---------------------------------------------------------------------------
// Scripted MCP server
// ---------------------------------------------------------------------------

#[derive(Clone, Default)]
pub struct McpBehaviour {
    pub initialize_gate: Option<Arc<tokio::sync::Notify>>,
    pub tools: Vec<serde_json::Value>,
    /// Tools per `tools/list` page; `0` means one unbounded page.
    pub page_size: usize,
    /// When set, `tools/list` answers with a redirect to this path instead of a catalog.
    pub redirect_tools_list_to: Option<String>,
    /// Answer `tools/call` with this status instead of a result.
    pub call_status: Option<u16>,
    /// Answer `tools/call` with this result document.
    pub call_result: Option<serde_json::Value>,
    /// Delay every `tools/call` answer.
    pub call_delay: Duration,
    /// A modern stateful Streamable HTTP fixture returns response events over SSE and mints this
    /// identity at `initialize`.
    pub session_id: Option<String>,
    pub calls: Arc<Mutex<Vec<serde_json::Value>>>,
    /// What the assembled request actually carried, so header assembly is observable end to end.
    pub seen: Arc<Mutex<Vec<ObservedHeaders>>>,
    /// `(method, mcp-session-id)` records the stateful lifecycle, not only an SSE content type.
    pub sessions: Arc<Mutex<Vec<ObservedSession>>>,
}

impl McpBehaviour {
    pub fn with_tools(tools: Vec<serde_json::Value>) -> Self {
        Self {
            tools,
            ..Self::default()
        }
    }

    pub fn call_count(&self) -> usize {
        self.calls
            .lock()
            .expect("the script mailbox is not poisoned")
            .len()
    }

    pub fn seen(&self) -> Vec<ObservedHeaders> {
        self.seen
            .lock()
            .expect("the script mailbox is not poisoned")
            .clone()
    }
}

pub type SharedBehaviour = Arc<Mutex<McpBehaviour>>;

/// What one assembled request carried: `(Authorization, X-Tenant)`.
pub type ObservedHeaders = (Option<String>, Option<String>);
pub type ObservedSession = (String, Option<String>);

pub fn tool(name: &str) -> serde_json::Value {
    serde_json::json!({
        "name": name,
        "description": format!("{name} tool"),
        "inputSchema": {"type": "object", "properties": {}, "additionalProperties": false}
    })
}

pub fn recursive_tool() -> serde_json::Value {
    serde_json::json!({
        "name": "Light",
        "description": "recursive",
        "inputSchema": {"type": "object", "$ref": "#/definitions/light"}
    })
}

pub async fn mcp_endpoint(
    State(shared): State<SharedBehaviour>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let Ok(request) = serde_json::from_slice::<serde_json::Value>(&body) else {
        return (StatusCode::BAD_REQUEST, "malformed").into_response();
    };
    let script = shared
        .lock()
        .expect("the script mailbox is not poisoned")
        .clone();
    script
        .seen
        .lock()
        .expect("the script mailbox is not poisoned")
        .push((
            header(&headers, "authorization"),
            header(&headers, "x-tenant"),
        ));
    let method = request
        .get("method")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default();
    let id = request.get("id").cloned();
    let session = header(&headers, "mcp-session-id");
    script
        .sessions
        .lock()
        .expect("the script mailbox is not poisoned")
        .push((method.to_owned(), session.clone()));
    if script
        .session_id
        .as_deref()
        .is_some_and(|expected| method != "initialize" && session.as_deref() != Some(expected))
    {
        return StatusCode::BAD_REQUEST.into_response();
    }

    match method {
        "" => StatusCode::ACCEPTED.into_response(),
        method if method.starts_with("notifications/") => StatusCode::ACCEPTED.into_response(),
        "initialize" => {
            if let Some(gate) = &script.initialize_gate {
                gate.notified().await;
            }
            streamable_response(
                id,
                serde_json::json!({
                    "protocolVersion": "2024-11-05",
                    "capabilities": {"tools": {}},
                    "serverInfo": {"name": "scripted", "version": "1"}
                }),
                script.session_id.as_deref(),
            )
        }
        "tools/list" => {
            if let Some(target) = script.redirect_tools_list_to.as_deref() {
                return (
                    StatusCode::FOUND,
                    [(axum::http::header::LOCATION, target.to_owned())],
                )
                    .into_response();
            }
            let cursor = request["params"]["cursor"]
                .as_str()
                .and_then(|cursor| cursor.parse::<usize>().ok())
                .unwrap_or(0);
            let page_size = script.page_size.max(1);
            let end = (cursor + page_size).min(script.tools.len());
            let page = script.tools.get(cursor..end).unwrap_or_default();
            let mut result = serde_json::json!({"tools": page});
            if end < script.tools.len() {
                result["nextCursor"] = serde_json::json!(end.to_string());
            }
            streamable_response(id, result, script.session_id.as_deref())
        }
        "tools/call" => {
            script
                .calls
                .lock()
                .expect("the script mailbox is not poisoned")
                .push(request.clone());
            if !script.call_delay.is_zero() {
                tokio::time::sleep(script.call_delay).await;
            }
            if let Some(status) = script.call_status {
                return StatusCode::from_u16(status)
                    .unwrap_or(StatusCode::BAD_GATEWAY)
                    .into_response();
            }
            streamable_response(
                id,
                script.call_result.unwrap_or_else(
                    || serde_json::json!({"content": [{"type": "text", "text": "ok"}]}),
                ),
                script.session_id.as_deref(),
            )
        }
        _ => error_response(id).into_response(),
    }
}

pub fn header(headers: &HeaderMap, name: &str) -> Option<String> {
    headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned)
}

pub fn streamable_response(
    id: Option<serde_json::Value>,
    result: serde_json::Value,
    session_id: Option<&str>,
) -> Response {
    let mut document = serde_json::json!({"jsonrpc": "2.0", "result": result});
    if let Some(id) = id {
        document["id"] = id;
    }
    let Some(session_id) = session_id else {
        return raw_json(document);
    };
    (
        StatusCode::OK,
        [
            (axum::http::header::CONTENT_TYPE, "text/event-stream"),
            (
                axum::http::HeaderName::from_static("mcp-session-id"),
                session_id,
            ),
        ],
        format!("event: message\ndata: {document}\n\n"),
    )
        .into_response()
}

pub fn error_response(id: Option<serde_json::Value>) -> Response {
    raw_json(serde_json::json!({
        "jsonrpc": "2.0",
        "id": id.unwrap_or(serde_json::Value::Null),
        "error": {"code": -32601, "message": "no such method"}
    }))
}

pub fn raw_json(document: serde_json::Value) -> Response {
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

pub struct McpServer {
    pub address: std::net::SocketAddr,
    pub behaviour: SharedBehaviour,
    pub task: JoinHandle<()>,
}

impl McpServer {
    pub fn url(&self) -> String {
        format!("http://{}/mcp", self.address)
    }

    pub fn rewrite(&self, edit: impl FnOnce(&mut McpBehaviour)) {
        let mut script = self
            .behaviour
            .lock()
            .expect("the script mailbox is not poisoned");
        edit(&mut script);
    }

    pub fn calls(&self) -> usize {
        self.behaviour
            .lock()
            .expect("the script mailbox is not poisoned")
            .call_count()
    }

    pub fn seen(&self) -> Vec<ObservedHeaders> {
        self.behaviour
            .lock()
            .expect("the script mailbox is not poisoned")
            .seen()
    }
}

pub async fn start_mcp(behaviour: McpBehaviour) -> McpServer {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let router = Router::new()
        .route("/mcp", post(mcp_endpoint))
        .route("/elsewhere", post(mcp_endpoint));
    let shared: SharedBehaviour = Arc::new(Mutex::new(behaviour));
    let state = Arc::clone(&shared);
    let task = tokio::spawn(async move {
        axum::serve(listener, router.with_state(state))
            .await
            .unwrap()
    });
    McpServer {
        address,
        behaviour: shared,
        task,
    }
}

pub struct EchoLlm;

impl LlmProvider for EchoLlm {
    fn adapter(&self) -> &'static str {
        "echo-llm"
    }

    fn complete(&self, _request: &LlmRequest) -> Result<String, LlmError> {
        Ok("echo".into())
    }
}

pub struct FinalAsr;

impl AsrProvider for FinalAsr {
    fn open(&self) -> Result<Box<dyn AsrSession>, AsrError> {
        Ok(Box::new(FinalAsrSession))
    }
}

pub struct FinalAsrSession;

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
    fn open(&self) -> Result<Box<dyn VadSession>, VadError> {
        Ok(Box::new(SilentVadSession))
    }

    fn adapter(&self) -> &'static str {
        "silent-vad"
    }
}

pub struct SilentVadSession;

impl VadSession for SilentVadSession {
    fn push(&mut self, _input: VadInput) -> Result<VadProbability, VadError> {
        Ok(VadProbability {
            start_sample: 0,
            end_sample: 512,
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
        Ok(PcmF32Mono::new(vec![0.1; 4_800], 48_000))
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

pub fn database_url() -> String {
    format!(
        "sqlite://{}",
        std::env::temp_dir()
            .join(format!("vela-external-mcp-{}.db", uuid::Uuid::new_v4()))
            .display()
    )
}

pub fn config(url: String) -> AppConfig {
    let address: std::net::SocketAddr = ([127, 0, 0, 1], 0).into();
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
        system_mcp: Default::default(),
        mcp: McpConfig {
            external: ExternalMcpConfig::default(),
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
    pub state: AppState,
    pub base: String,
    pub url: String,
    pub task: JoinHandle<()>,
}

pub async fn start(secrets: ConstantSecrets) -> Voice {
    start_with_config(config(database_url()), secrets).await
}

pub async fn start_with_config(
    app_config: AppConfig,
    secrets: impl SecretResolver + 'static,
) -> Voice {
    start_with_materializer(app_config, secrets, None).await
}

pub async fn start_with_materializer(
    app_config: AppConfig,
    secrets: impl SecretResolver + 'static,
    materializer: Option<Arc<dyn vela_server::services::provider_runtime::RuntimeMaterializer>>,
) -> Voice {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let url = app_config.database.url.clone();
    let database = Database::connect(&app_config.database).await.unwrap();
    crate::admin_web::seed_default_admin(database.pool()).await;
    let mut state = AppState::from_provider_set_with_database_resolver_and_shutdown(
        app_config,
        Arc::new(ProviderSet::with_all(
            Arc::new(SilentVad),
            Arc::new(FinalAsr),
            Arc::new(EchoLlm),
            Arc::new(ShortTts),
        )),
        Some(database),
        Arc::new(secrets),
        vela_server::lifecycle::RuntimeLifecycle::new(std::time::Duration::from_millis(1_024)),
    );
    if let Some(materializer) = materializer {
        use vela_server::services::provider_runtime::{ProviderRuntimeManager, RuntimeLimits};
        let manager = ProviderRuntimeManager::new(
            RuntimeLimits {
                max_parallel_loads: 1,
                max_pending_loads: 8,
                max_waiters: 16,
                max_resident_bytes: 16,
                max_resources: 8,
                max_version_entries: 16,
                admission_timeout_ms: 5000,
                failure_cooldown_ms: 100,
                idle_ttl_ms: 60000,
            },
            materializer,
            state.admission_gate().clone(),
        )
        .unwrap();
        state = state.with_runtime_manager(manager);
    }
    let served = state.clone();
    let task = tokio::spawn(async move {
        axum::serve(listener, vela_server::app::router_with_state(served))
            .await
            .unwrap()
    });
    Voice {
        state,
        base: format!("http://{address}"),
        url,
        task,
    }
}

impl Voice {
    pub async fn database(&self) -> SqlitePool {
        SqlitePool::connect(&self.url).await.unwrap()
    }

    pub async fn admit(&self) -> EffectiveSessionProfile {
        self.state
            .resolve_session_profile("device")
            .await
            .expect("the seeded Device is admitted")
    }

    pub async fn upgrade(&self) -> WsStatusCode {
        let mut request = format!("{}/voice/v1/", self.base.replacen("http", "ws", 1))
            .into_client_request()
            .unwrap();
        let headers = request.headers_mut();
        headers.insert("Protocol-Version", "1".parse().unwrap());
        headers.insert("Device-Id", "device".parse().unwrap());
        headers.insert("Client-Id", "external-mcp-test".parse().unwrap());
        match timeout(Duration::from_secs(5), connect_async(request))
            .await
            .expect("admission answers within its own budget")
        {
            Ok((_socket, response)) => response.status(),
            Err(error) => match error {
                tokio_tungstenite::tungstenite::Error::Http(response) => response.status(),
                other => panic!("expected an HTTP WebSocket outcome, got {other:?}"),
            },
        }
    }
}

pub async fn seed(pool: &SqlitePool) {
    sqlx::query("INSERT INTO agents (key,name,enabled,created_at,updated_at) VALUES ('agent','Agent',1,1,1)")
        .execute(pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO devices (device_id,agent_id,enabled,created_at,updated_at) VALUES ('device',1,1,1,1)")
        .execute(pool)
        .await
        .unwrap();
}

#[allow(clippy::too_many_arguments)]
pub async fn bind_server(
    pool: &SqlitePool,
    key: &str,
    url: &str,
    headers: &str,
    auth: (&str, Option<&str>, Option<&str>),
) {
    let (auth_type, auth_header_name, _secret_ref) = auth;
    let server_id: i64 = sqlx::query_scalar(
        "INSERT INTO mcp_servers (key,name,url,headers_json,auth_type,auth_header_name,\
         connect_timeout_ms,request_timeout_ms,enabled,created_at,updated_at) \
         VALUES (?, ?, ?, ?, ?, ?, 2000, 2000, 1, 1, 1) RETURNING id",
    )
    .bind(key)
    .bind(key)
    .bind(url)
    .bind(headers)
    .bind(auth_type)
    .bind(auth_header_name)
    .fetch_one(pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO agent_mcp_bindings (agent_id,mcp_server_id,enabled,required,created_at) \
         VALUES (1, ?, 1, 0, 1)",
    )
    .bind(server_id)
    .execute(pool)
    .await
    .unwrap();
}

pub fn names(catalog: &SessionExternalMcp) -> Vec<String> {
    catalog
        .servers()
        .iter()
        .flat_map(|server| {
            server
                .tools
                .iter()
                .map(|tool| tool.llm_name.clone())
                .collect::<Vec<_>>()
        })
        .collect()
}
