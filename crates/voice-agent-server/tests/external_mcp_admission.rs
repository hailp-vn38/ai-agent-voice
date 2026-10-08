//! Ticket 09: the External MCP admission snapshot, proven through the admission boundary.
//!
//! Everything here goes through the boundary a Voice Session actually crosses: the application
//! state resolves one Effective Session Profile for a Device, and the WebSocket either upgrades or
//! does not.  The assertions are about what a session may call, what it may not, and what an
//! External MCP failure may never change.

use std::{
    sync::{Arc, Mutex},
    time::Duration,
};

use axum::{
    Router,
    body::Bytes,
    extract::State,
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::post,
};
use sqlx::SqlitePool;
use tokio::{net::TcpListener, task::JoinHandle, time::timeout};
use tokio_tungstenite::{
    connect_async,
    tungstenite::{client::IntoClientRequest, http::StatusCode as WsStatusCode},
};
use url::Url;
use voice_agent_server::audio::PcmF32Mono;
use voice_agent_server::{
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

/// What the fake MCP server does next.  A test rewrites it between calls, so "did the session
/// re-discover, or did it reuse what it saw last time?" is observable rather than assumed.
#[derive(Clone, Default)]
struct McpBehaviour {
    tools: Vec<serde_json::Value>,
    /// Tools per `tools/list` page; `0` means one unbounded page.
    page_size: usize,
    /// When set, `tools/list` answers with a redirect to this path instead of a catalog.
    redirect_tools_list_to: Option<String>,
    /// Answer `tools/call` with this status instead of a result.
    call_status: Option<u16>,
    /// Answer `tools/call` with this result document.
    call_result: Option<serde_json::Value>,
    /// Delay every `tools/call` answer.
    call_delay: Duration,
    /// A modern stateful Streamable HTTP fixture returns response events over SSE and mints this
    /// identity at `initialize`.
    session_id: Option<String>,
    calls: Arc<Mutex<Vec<serde_json::Value>>>,
    /// What the assembled request actually carried, so header assembly is observable end to end.
    seen: Arc<Mutex<Vec<ObservedHeaders>>>,
    /// `(method, mcp-session-id)` records the stateful lifecycle, not only an SSE content type.
    sessions: Arc<Mutex<Vec<ObservedSession>>>,
}

impl McpBehaviour {
    fn with_tools(tools: Vec<serde_json::Value>) -> Self {
        Self {
            tools,
            ..Self::default()
        }
    }

    fn call_count(&self) -> usize {
        self.calls
            .lock()
            .expect("the script mailbox is not poisoned")
            .len()
    }

    fn seen(&self) -> Vec<ObservedHeaders> {
        self.seen
            .lock()
            .expect("the script mailbox is not poisoned")
            .clone()
    }
}

type SharedBehaviour = Arc<Mutex<McpBehaviour>>;

/// What one assembled request carried: `(Authorization, X-Tenant)`.
type ObservedHeaders = (Option<String>, Option<String>);
type ObservedSession = (String, Option<String>);

fn tool(name: &str) -> serde_json::Value {
    serde_json::json!({
        "name": name,
        "description": format!("{name} tool"),
        "inputSchema": {"type": "object", "properties": {}, "additionalProperties": false}
    })
}

/// The one schema shape the supported subset refuses, used to prove an untrusted catalog is
/// validated rather than converted.
fn recursive_tool() -> serde_json::Value {
    serde_json::json!({
        "name": "Light",
        "description": "recursive",
        "inputSchema": {"type": "object", "$ref": "#/definitions/light"}
    })
}

async fn mcp_endpoint(
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
        // A notification expects acceptance, not an answer: a JSON-RPC document here would be a
        // response to a request that was never made.
        "" => StatusCode::ACCEPTED.into_response(),
        method if method.starts_with("notifications/") => StatusCode::ACCEPTED.into_response(),
        "initialize" => streamable_response(
            id,
            serde_json::json!({
                "protocolVersion": "2024-11-05",
                "capabilities": {"tools": {}},
                "serverInfo": {"name": "scripted", "version": "1"}
            }),
            script.session_id.as_deref(),
        ),
        "tools/list" => {
            if let Some(target) = script.redirect_tools_list_to.as_deref() {
                return (
                    StatusCode::FOUND,
                    [(axum::http::header::LOCATION, target.to_owned())],
                )
                    .into_response();
            }
            // A cursor is an opaque string the server minted, not a number the client chose.
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

fn header(headers: &HeaderMap, name: &str) -> Option<String> {
    headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned)
}

fn streamable_response(
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

fn error_response(id: Option<serde_json::Value>) -> Response {
    raw_json(serde_json::json!({
        "jsonrpc": "2.0",
        "id": id.unwrap_or(serde_json::Value::Null),
        "error": {"code": -32601, "message": "no such method"}
    }))
}

fn raw_json(document: serde_json::Value) -> Response {
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

struct McpServer {
    address: std::net::SocketAddr,
    behaviour: SharedBehaviour,
    task: JoinHandle<()>,
}

impl McpServer {
    fn url(&self) -> String {
        format!("http://{}/mcp", self.address)
    }

    fn rewrite(&self, edit: impl FnOnce(&mut McpBehaviour)) {
        let mut script = self
            .behaviour
            .lock()
            .expect("the script mailbox is not poisoned");
        edit(&mut script);
    }

    fn calls(&self) -> usize {
        self.behaviour
            .lock()
            .expect("the script mailbox is not poisoned")
            .call_count()
    }

    fn seen(&self) -> Vec<ObservedHeaders> {
        self.behaviour
            .lock()
            .expect("the script mailbox is not poisoned")
            .seen()
    }
}

async fn start_mcp(behaviour: McpBehaviour) -> McpServer {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    // The catalog behind `/elsewhere` is only reachable by following a redirect.
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

// ---------------------------------------------------------------------------
// Voice server
// ---------------------------------------------------------------------------

struct EchoLlm;

impl LlmProvider for EchoLlm {
    fn adapter(&self) -> &'static str {
        "echo-llm"
    }

    fn complete(&self, _request: &LlmRequest) -> Result<String, LlmError> {
        Ok("echo".into())
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
    fn open(&self) -> Result<Box<dyn VadSession>, VadError> {
        Ok(Box::new(SilentVadSession))
    }

    fn adapter(&self) -> &'static str {
        "silent-vad"
    }
}

struct SilentVadSession;

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

struct ShortTts;

impl TtsProvider for ShortTts {
    fn adapter(&self) -> &'static str {
        "short-tts"
    }

    fn synthesize(&self, _: &str) -> Result<PcmF32Mono, TtsError> {
        Ok(PcmF32Mono::new(vec![0.1; 4_800], 48_000))
    }
}

/// Resolves to one scripted value whatever the reference is.  A `SecretRef` is deliberately opaque
/// outside the crate, so a deployment's resolver is the only thing that may interpret one — and a
/// test proves the credential path without ever reading a real environment variable.
struct ConstantSecrets(Option<String>);

impl SecretResolver for ConstantSecrets {
    fn resolve(&self, _reference: &SecretRef) -> Result<SecretValue, SecretResolveError> {
        self.0
            .clone()
            .map(SecretValue::new)
            .ok_or(SecretResolveError::Unavailable)
    }
}

fn database_url() -> String {
    format!(
        "sqlite://{}",
        std::env::temp_dir()
            .join(format!(
                "voice-agent-external-mcp-{}.db",
                uuid::Uuid::new_v4()
            ))
            .display()
    )
}

fn config(url: String) -> AppConfig {
    let address: std::net::SocketAddr = ([127, 0, 0, 1], 0).into();
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
        speaker_recognition: Default::default(),
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

struct Voice {
    state: AppState,
    base: String,
    url: String,
    task: JoinHandle<()>,
}

/// Migrations run first so the control plane exists before a test writes a row: this mirrors the
/// production order, where the database is a startup dependency and never a runtime lookup.
async fn start(secrets: ConstantSecrets) -> Voice {
    start_with_config(config(database_url()), secrets).await
}

async fn start_with_config(app_config: AppConfig, secrets: ConstantSecrets) -> Voice {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let url = app_config.database.url.clone();
    let database = Database::connect(&app_config.database).await.unwrap();
    let state = AppState::from_provider_set_with_database_resolver_and_shutdown(
        app_config,
        Arc::new(ProviderSet::with_all(
            Arc::new(SilentVad),
            Arc::new(FinalAsr),
            Arc::new(EchoLlm),
            Arc::new(ShortTts),
        )),
        Some(database),
        Arc::new(secrets),
        voice_agent_server::lifecycle::RuntimeLifecycle::new(std::time::Duration::from_millis(
            1_024,
        )),
    );
    let served = state.clone();
    let task = tokio::spawn(async move {
        axum::serve(listener, voice_agent_server::app::router_with_state(served))
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
    /// The migrated control plane this server admitted against.
    async fn database(&self) -> SqlitePool {
        SqlitePool::connect(&self.url).await.unwrap()
    }

    async fn admit(&self) -> EffectiveSessionProfile {
        self.state
            .resolve_session_profile("device")
            .await
            .expect("the seeded Device is admitted")
    }

    /// The client-visible half of the same boundary: did the WebSocket actually upgrade?
    async fn upgrade(&self) -> WsStatusCode {
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

// ---------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------

async fn seed(pool: &SqlitePool) {
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
async fn bind_server(
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

fn names(catalog: &SessionExternalMcp) -> Vec<String> {
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

// ---------------------------------------------------------------------------
// The gate
// ---------------------------------------------------------------------------

/// One admission produces a fresh, bounded, fail-soft snapshot, and a Voice Session crosses the
/// boundary on it.
#[tokio::test]
async fn external_mcp_admission_snapshot_is_fresh_bounded_and_immutable() {
    let server = start_mcp(McpBehaviour::with_tools(vec![
        tool("Light/Turn-On"),
        tool("Dim"),
    ]))
    .await;
    let voice = start(ConstantSecrets(Some("s3cr3t-bearer".into()))).await;
    let pool = voice.database().await;
    seed(&pool).await;
    bind_server(
        &pool,
        "Home-Assistant",
        &server.url(),
        r#"{"X-Tenant": "kitchen"}"#,
        ("bearer", None, Some("WEATHER_TOKEN")),
    )
    .await;
    // A second bound server is unreachable.  It must cost only itself.
    bind_server(
        &pool,
        "dead",
        "http://127.0.0.1:1/mcp",
        "{}",
        ("none", None, None),
    )
    .await;

    let profile = voice.admit().await;
    assert_eq!(
        names(profile.external_mcp()),
        Vec::<String>::new(),
        "discovery records the unapproved contract but never publishes it"
    );
    assert_eq!(voice.upgrade().await, WsStatusCode::SWITCHING_PROTOCOLS);

    // Both the validated static header and the typed credential travelled, and the typed auth is
    // the only thing that ever puts a credential on the request.
    let seen = server.seen();
    assert!(
        seen.iter().all(|(authorization, tenant)| {
            authorization == &Some("Bearer s3cr3t-bearer".to_owned())
                && tenant == &Some("kitchen".to_owned())
        }),
        "every request carries the validated static header and the typed credential: {seen:?}"
    );
    assert!(!seen.is_empty());

    // Fresh, not cached: the remote catalog changing changes the next admission's snapshot, so a
    // previous session's verification is never authority for this one.
    server.rewrite(|script| script.tools = vec![tool("Only")]);
    assert_eq!(
        names(voice.admit().await.external_mcp()),
        Vec::<String>::new()
    );
    // And the earlier session's own catalog did not change under it.
    assert_eq!(
        profile.external_mcp().tool_count(),
        0,
        "a later admission never mutates an open session's snapshot"
    );

    voice.task.abort();
    server.task.abort();
}

// ---------------------------------------------------------------------------
// Fail-soft
// ---------------------------------------------------------------------------

/// The modern stateful variant is an SSE lifecycle, not a stateless JSON fixture wearing an SSE
/// content type: RMCP must retain the identity minted at initialize for its notification,
/// discovery and later tool invocation.
/// A credential that does not resolve excludes its server and discloses nothing.
#[tokio::test]
async fn external_mcp_secret_failure_excludes_its_server_and_redacts_its_reason() {
    let server = start_mcp(McpBehaviour::with_tools(vec![tool("Light")])).await;
    // The resolver knows nothing, so the reference cannot be resolved.
    let voice = start(ConstantSecrets(None)).await;
    let pool = voice.database().await;
    seed(&pool).await;
    bind_server(
        &pool,
        "guarded",
        &server.url(),
        "{}",
        ("bearer", None, Some("WEATHER_TOKEN")),
    )
    .await;

    let profile = voice.admit().await;
    assert!(
        profile.external_mcp().is_empty(),
        "a server whose credential did not resolve publishes no tools"
    );
    assert_eq!(
        server.calls(),
        0,
        "a credential that never resolved must not reach the network"
    );
    assert_eq!(voice.upgrade().await, WsStatusCode::SWITCHING_PROTOCOLS);
    // Neither the reference nor any resolved value may appear in anything a test can render.
    let rendered = format!(
        "{:?} {:?}",
        profile.external_mcp(),
        ExternalMcpExclusionReason::SecretResolutionFailed("secret_resolver_unavailable")
    );
    assert!(
        !rendered.contains("WEATHER_TOKEN"),
        "a diagnostic must never carry a secret reference: {rendered}"
    );
    assert!(rendered.contains("secret_resolver_unavailable"));

    voice.task.abort();
    server.task.abort();
}

/// One refusal case: the label under test, the bounded class it must be reported as, the catalog
/// the server announces, the page size it serves, and the limits the deployment runs with.
type Case = (
    &'static str,
    ExternalMcpExclusionReason,
    Vec<serde_json::Value>,
    usize,
    ExternalMcpLimitsConfig,
);

/// Every catalog cap and namespace rule refuses a server rather than shrinking one.
#[tokio::test]
async fn external_mcp_discovery_refuses_caps_namespaces_and_collisions() {
    let rejected = ExternalMcpExclusionReason::CatalogRejected;
    let cases: Vec<Case> = vec![
        (
            "a schema outside the supported subset",
            rejected,
            vec![recursive_tool()],
            0,
            ExternalMcpLimitsConfig::default(),
        ),
        (
            "a composition the conversion cannot honor",
            rejected,
            vec![serde_json::json!({
                "name": "Light",
                "description": "composed",
                "inputSchema": {"type": "object", "oneOf": [{"type": "object"}]}
            })],
            0,
            ExternalMcpLimitsConfig::default(),
        ),
        (
            "a required property the schema does not declare",
            rejected,
            vec![serde_json::json!({
                "name": "Light",
                "description": "unsatisfiable",
                "inputSchema": {"type": "object", "required": ["absent"]}
            })],
            0,
            ExternalMcpLimitsConfig::default(),
        ),
        (
            "a schema larger than the configured cap",
            rejected,
            vec![serde_json::json!({
                "name": "Light",
                "description": "oversized",
                "inputSchema": {
                    "type": "object",
                    "description": "x".repeat(128),
                    "properties": {}
                }
            })],
            0,
            ExternalMcpLimitsConfig {
                max_tool_schema_bytes: 16,
                ..ExternalMcpLimitsConfig::default()
            },
        ),
        (
            "a description larger than the configured cap",
            rejected,
            vec![serde_json::json!({
                "name": "Light",
                "description": "x".repeat(64),
                "inputSchema": {"type": "object", "properties": {}}
            })],
            0,
            ExternalMcpLimitsConfig {
                max_tool_description_bytes: 16,
                ..ExternalMcpLimitsConfig::default()
            },
        ),
        (
            "more tools than one server may publish",
            rejected,
            (0..5).map(|index| tool(&format!("Tool{index}"))).collect(),
            0,
            ExternalMcpLimitsConfig {
                max_tools_per_server: 2,
                ..ExternalMcpLimitsConfig::default()
            },
        ),
        (
            "more pages than one server may walk",
            rejected,
            (0..4).map(|index| tool(&format!("Tool{index}"))).collect(),
            1,
            ExternalMcpLimitsConfig {
                max_pages_per_server: 2,
                ..ExternalMcpLimitsConfig::default()
            },
        ),
        (
            "two tool names that normalize alike",
            ExternalMcpExclusionReason::ToolNameCollision,
            vec![tool("foo-bar"), tool("foo_bar")],
            0,
            ExternalMcpLimitsConfig::default(),
        ),
    ];

    for (label, expected, tools, page_size, limits) in cases {
        let server = start_mcp(McpBehaviour {
            page_size,
            ..McpBehaviour::with_tools(tools)
        })
        .await;
        let mut app_config = config(database_url());
        app_config.mcp.external.limits = limits;
        let voice = start_with_config(app_config, ConstantSecrets(None)).await;
        let pool = voice.database().await;
        seed(&pool).await;
        bind_server(&pool, "kitchen", &server.url(), "{}", ("none", None, None)).await;

        assert!(
            voice.admit().await.external_mcp().is_empty(),
            "{label} must leave the session without that server's tools"
        );
        // The session learns nothing, but the operator learns exactly which class of refusal it
        // was — a bounded reason, never a name, a URL or a credential.
        let snapshot = voice
            .state
            .external_mcp
            .as_ref()
            .expect("the shared transport is available")
            .resolve_snapshot(
                &Database::connect(&voice.state.config.database)
                    .await
                    .expect("the control plane is reachable")
                    .agent_mcp_servers(1)
                    .await
                    .expect("the bindings are readable"),
                &ConstantSecrets(None),
            )
            .await;
        assert_eq!(
            snapshot
                .exclusions
                .iter()
                .map(|exclusion| (exclusion.server_key.as_str(), exclusion.reason))
                .collect::<Vec<_>>(),
            vec![("kitchen", expected)],
            "{label} must be reported as one bounded class"
        );
        assert_eq!(
            voice.upgrade().await,
            WsStatusCode::SWITCHING_PROTOCOLS,
            "{label} must never refuse a Voice Session"
        );
        voice.task.abort();
        server.task.abort();
    }
}

/// Two server keys that normalize alike make each other's tools indistinguishable, so neither is
/// published — and the answer does not depend on bind order.
#[tokio::test]
async fn external_mcp_refuses_server_keys_that_normalize_alike() {
    let server = start_mcp(McpBehaviour::with_tools(vec![tool("Light")])).await;
    let voice = start(ConstantSecrets(None)).await;
    let pool = voice.database().await;
    seed(&pool).await;
    bind_server(&pool, "foo-bar", &server.url(), "{}", ("none", None, None)).await;
    bind_server(&pool, "foo_bar", &server.url(), "{}", ("none", None, None)).await;
    assert!(voice.admit().await.external_mcp().is_empty());
    assert_eq!(voice.upgrade().await, WsStatusCode::SWITCHING_PROTOCOLS);
    voice.task.abort();
    server.task.abort();
}

/// The aggregate cap gets its own counter and no per-server duration, and every server that did
/// resolve still counts as resolved.  Counting it a second time as a server failure would report
/// an outcome and a duration for work that never happened.
#[tokio::test]
async fn external_mcp_aggregate_tool_cap_is_counted_once_and_not_as_a_server_failure() {
    let first = start_mcp(McpBehaviour::with_tools(vec![tool("A0"), tool("A1")])).await;
    let second = start_mcp(McpBehaviour::with_tools(vec![tool("B0"), tool("B1")])).await;
    let recorder = Arc::new(RecordingTelemetry::default());
    let mut app_config = config(database_url());
    app_config.mcp.external.limits = ExternalMcpLimitsConfig {
        max_tools_per_session: 3,
        ..ExternalMcpLimitsConfig::default()
    };
    let voice = start_with_config(app_config, ConstantSecrets(None)).await;
    let manager =
        ExternalMcpManager::new_with_telemetry(&voice.state.config.mcp.external, recorder.clone())
            .expect("the transport builds");
    let pool = voice.database().await;
    seed(&pool).await;
    bind_server(&pool, "alpha", &first.url(), "{}", ("none", None, None)).await;
    bind_server(&pool, "beta", &second.url(), "{}", ("none", None, None)).await;

    let snapshot = manager
        .resolve_snapshot(
            &Database::connect(&voice.state.config.database)
                .await
                .expect("the control plane is reachable")
                .agent_mcp_servers(1)
                .await
                .expect("the bindings are readable"),
            &ConstantSecrets(None),
        )
        .await;
    assert!(snapshot.servers.is_empty());
    assert_eq!(snapshot.exclusions.len(), 2);

    let recorded = recorder.recorded();
    let counted: Vec<(&str, Vec<(&str, String)>)> = recorded
        .iter()
        .map(|event| (event.metric, event.labels.clone()))
        .collect();

    // Counted once, with no label: it is a property of the snapshot, not of a server.
    let cap: Vec<_> = recorded
        .iter()
        .filter(|event| event.metric == EXTERNAL_MCP_SESSION_TOOL_CAP_EXCEEDED_TOTAL)
        .collect();
    assert_eq!(
        cap.len(),
        1,
        "the aggregate cap is counted once: {counted:?}"
    );
    assert_eq!(cap[0].labels, Vec::<(&str, String)>::new());

    // Both servers still counted as resolved, and neither as a failure.
    let successes: Vec<&Vec<(&str, String)>> = recorded
        .iter()
        .filter(|event| event.metric == MCP_RESOLVE_SUCCESS_TOTAL)
        .map(|event| &event.labels)
        .collect();
    assert_eq!(successes.len(), 2, "each resolved server still counts once");
    assert!(
        recorded
            .iter()
            .all(|event| event.metric != MCP_RESOLVE_FAILURE_TOTAL)
    );

    voice.task.abort();
    first.task.abort();
    second.task.abort();
}

/// The aggregate cap drops the whole External MCP snapshot rather than any subset of it.
#[tokio::test]
async fn external_mcp_aggregate_tool_cap_drops_the_whole_snapshot() {
    let first = start_mcp(McpBehaviour::with_tools(vec![tool("A0"), tool("A1")])).await;
    let second = start_mcp(McpBehaviour::with_tools(vec![tool("B0"), tool("B1")])).await;
    let mut app_config = config(database_url());
    app_config.mcp.external.limits = ExternalMcpLimitsConfig {
        max_tools_per_session: 3,
        ..ExternalMcpLimitsConfig::default()
    };
    let voice = start_with_config(app_config, ConstantSecrets(None)).await;
    let pool = voice.database().await;
    seed(&pool).await;
    bind_server(&pool, "alpha", &first.url(), "{}", ("none", None, None)).await;
    bind_server(&pool, "beta", &second.url(), "{}", ("none", None, None)).await;

    assert!(
        voice.admit().await.external_mcp().is_empty(),
        "an aggregate overflow must not leave a partial catalog behind"
    );
    assert_eq!(voice.upgrade().await, WsStatusCode::SWITCHING_PROTOCOLS);
    voice.task.abort();
    first.task.abort();
    second.task.abort();
}

// ---------------------------------------------------------------------------
// Outbound policy
// ---------------------------------------------------------------------------

/// A destination the operator did not allow is never contacted, and a redirect is never followed.
#[tokio::test]
async fn external_mcp_outbound_policy_refuses_unlisted_destinations_and_redirects() {
    // 1. A redirect from `tools/list` is not followed, so the catalog behind it never arrives.
    let redirecting = start_mcp(McpBehaviour {
        redirect_tools_list_to: Some("/elsewhere".into()),
        ..McpBehaviour::with_tools(vec![tool("Light")])
    })
    .await;
    let voice = start(ConstantSecrets(None)).await;
    let pool = voice.database().await;
    seed(&pool).await;
    bind_server(
        &pool,
        "kitchen",
        &redirecting.url(),
        "{}",
        ("none", None, None),
    )
    .await;
    assert!(
        voice.admit().await.external_mcp().is_empty(),
        "a redirect must not be a way to reach a catalog the operator never approved"
    );
    assert_eq!(voice.upgrade().await, WsStatusCode::SWITCHING_PROTOCOLS);
    voice.task.abort();
    redirecting.task.abort();

    // 2. An explicitly configured hostname allowlist rejects an unlisted destination.
    let server = start_mcp(McpBehaviour::with_tools(vec![tool("Light")])).await;
    let mut app_config = config(database_url());
    app_config.mcp.external.network = ExternalMcpNetworkConfig {
        allowed_hosts: vec!["mcp.example.test".into()],
    };
    let voice = start_with_config(app_config, ConstantSecrets(None)).await;
    let pool = voice.database().await;
    seed(&pool).await;
    bind_server(&pool, "kitchen", &server.url(), "{}", ("none", None, None)).await;
    assert!(voice.admit().await.external_mcp().is_empty());
    assert_eq!(
        server.calls(),
        0,
        "an unlisted destination is never contacted"
    );
    voice.task.abort();
    server.task.abort();
}

/// An `https` destination is never dialled without certificate and hostname validation, so a
/// plain-HTTP endpoint cannot be reached by asking for TLS: there is no insecure mode to ask for.
#[tokio::test]
async fn external_mcp_https_never_falls_back_to_an_unvalidated_connection() {
    let server = start_mcp(McpBehaviour::with_tools(vec![tool("Light")])).await;
    let voice = start(ConstantSecrets(None)).await;
    let pool = voice.database().await;
    seed(&pool).await;
    let https = format!("https://{}/mcp", server.address);
    bind_server(&pool, "kitchen", &https, "{}", ("none", None, None)).await;

    assert!(
        voice.admit().await.external_mcp().is_empty(),
        "a certificate the client cannot validate leaves the server with no tools"
    );
    assert_eq!(server.calls(), 0);
    assert_eq!(voice.upgrade().await, WsStatusCode::SWITCHING_PROTOCOLS);
    voice.task.abort();
    server.task.abort();
}

// ---------------------------------------------------------------------------
// Call outcomes and immutability
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// Binding scope
// ---------------------------------------------------------------------------

/// A disabled binding or a disabled server is not an intent this session acts on.
#[tokio::test]
async fn external_mcp_publishes_only_enabled_bindings_of_enabled_servers() {
    let server = start_mcp(McpBehaviour::with_tools(vec![tool("Light")])).await;
    let voice = start(ConstantSecrets(None)).await;
    let pool = voice.database().await;
    seed(&pool).await;
    bind_server(&pool, "enabled", &server.url(), "{}", ("none", None, None)).await;
    bind_server(&pool, "disabled", &server.url(), "{}", ("none", None, None)).await;
    bind_server(
        &pool,
        "soft-deleted",
        &server.url(),
        "{}",
        ("none", None, None),
    )
    .await;
    sqlx::query(
        "UPDATE agent_mcp_bindings SET enabled = 0 WHERE mcp_server_id = \
         (SELECT id FROM mcp_servers WHERE key = 'disabled')",
    )
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query("UPDATE mcp_servers SET enabled = 0 WHERE key = 'soft-deleted'")
        .execute(&pool)
        .await
        .unwrap();

    assert_eq!(
        names(voice.admit().await.external_mcp()),
        Vec::<String>::new()
    );
    assert_eq!(voice.upgrade().await, WsStatusCode::SWITCHING_PROTOCOLS);
    voice.task.abort();
    server.task.abort();
}

/// An Agent with no External MCP binding gets a session with no tools, and nothing else changes.
#[tokio::test]
async fn an_agent_without_mcp_bindings_admits_without_external_tools() {
    let voice = start(ConstantSecrets(None)).await;
    let pool = voice.database().await;
    seed(&pool).await;

    assert!(voice.admit().await.external_mcp().is_empty());
    assert_eq!(voice.upgrade().await, WsStatusCode::SWITCHING_PROTOCOLS);
    voice.task.abort();
}

/// Every exclusion a snapshot reports is a bounded class, and the shared transport builds once
/// with the redirect and TLS policy the guide requires.
#[tokio::test]
async fn external_mcp_diagnostics_stay_bounded_classes() {
    assert_eq!(
        ExternalMcpExclusionReason::ToolsListTimeout.to_string(),
        "mcp_tools_list_timeout"
    );
    assert_eq!(
        ExternalMcpExclusionReason::SessionToolCapExceeded.to_string(),
        "external_mcp_session_tool_cap_exceeded"
    );
    assert_eq!(
        ExternalMcpExclusionReason::SecretResolutionFailed("secret_resolver_unavailable")
            .to_string(),
        "external_mcp_secret_resolution_failed:secret_resolver_unavailable"
    );
    let manager =
        ExternalMcpManager::new(&ExternalMcpConfig::default()).expect("the transport builds");
    assert_eq!(manager.limiter().capacity(), 16);
}

/// The telemetry seam reports the guide's metrics, and the only free-form value it can carry is a
/// server key the Admin API already bounds.  A credential, a destination, a protocol session id, a
/// tool name, a tool argument and a tool result have no way in.

#[tokio::test]
async fn external_agent_reviews_exact_contract_and_drift_requires_new_review() {
    let server = start_mcp(McpBehaviour::with_tools(vec![tool("Light")])).await;
    let mut configuration = config(database_url());
    configuration.api.enabled = true;
    configuration.api.admin_token = "review-token".into();
    let voice = start_with_config(configuration, ConstantSecrets(None)).await;
    let pool = voice.database().await;
    seed(&pool).await;
    bind_server(&pool, "home", &server.url(), "{}", ("none", None, None)).await;
    assert!(names(voice.admit().await.external_mcp()).is_empty());
    let client = reqwest::Client::new();
    let url = format!("{}/api/admin/agents/agent/tool-allowlist", voice.base);
    let items = client
        .get(&url)
        .bearer_auth("review-token")
        .send()
        .await
        .unwrap()
        .json::<serde_json::Value>()
        .await
        .unwrap();
    let observation = &items["items"][0];
    assert_eq!(observation["allowed"], false);
    let mut review = serde_json::json!({"server_key":"home","original_name":"Light","observed_revision":observation["observed_revision"],"fingerprint":observation["fingerprint"],"allowed":true,"sensitive":false});
    let response = client
        .put(&url)
        .bearer_auth("review-token")
        .header("If-Match", "\"1\"")
        .json(&review)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::OK);
    assert_eq!(
        names(voice.admit().await.external_mcp()),
        vec!["external.home.light"]
    );
    use futures_util::{SinkExt, StreamExt};
    use tokio_tungstenite::tungstenite::Message;
    let mut request = format!("{}/voice/v1/", voice.base.replacen("http", "ws", 1))
        .into_client_request()
        .unwrap();
    request
        .headers_mut()
        .insert("Protocol-Version", "1".parse().unwrap());
    request
        .headers_mut()
        .insert("Device-Id", "device".parse().unwrap());
    request
        .headers_mut()
        .insert("Client-Id", "review-test".parse().unwrap());
    let (mut socket, _) = connect_async(request).await.unwrap();
    socket.send(Message::Text(serde_json::json!({"type":"hello","version":1,"transport":"websocket","audio_params":{"format":"opus","sample_rate":16000,"channels":1,"frame_duration":60}}).to_string().into())).await.unwrap();
    assert!(matches!(
        socket.next().await.unwrap().unwrap(),
        Message::Text(_)
    ));
    review["sensitive"] = serde_json::json!(true);
    assert_eq!(
        client
            .put(&url)
            .bearer_auth("review-token")
            .header("If-Match", "\"1\"")
            .json(&review)
            .send()
            .await
            .unwrap()
            .status(),
        reqwest::StatusCode::CONFLICT
    );
    assert_eq!(
        client
            .put(&url)
            .bearer_auth("review-token")
            .header("If-Match", "\"2\"")
            .json(&review)
            .send()
            .await
            .unwrap()
            .status(),
        reqwest::StatusCode::OK
    );
    let close = timeout(Duration::from_secs(2), async {
        loop {
            if let Some(Ok(Message::Close(Some(frame)))) = socket.next().await {
                break frame.code;
            }
        }
    })
    .await
    .unwrap();
    assert_eq!(u16::from(close), 1008);
    assert!(names(voice.admit().await.external_mcp()).is_empty());
    server.behaviour.lock().unwrap().tools[0]["description"] =
        serde_json::json!("changed contract");
    assert!(names(voice.admit().await.external_mcp()).is_empty());
    assert_eq!(
        client
            .put(&url)
            .bearer_auth("review-token")
            .header("If-Match", "\"3\"")
            .json(&review)
            .send()
            .await
            .unwrap()
            .status(),
        reqwest::StatusCode::CONFLICT
    );
    voice.task.abort();
    server.task.abort();
}
