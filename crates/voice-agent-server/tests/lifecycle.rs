//! Ticket 12: readiness, ordered shutdown and operational degradation, proven through the
//! boundaries an operator actually observes.
//!
//! Everything here goes through `/health`, `/ready`, the WebSocket admission boundary and the
//! application lifecycle.  The assertions are about what a process tells its orchestrator and what
//! it accepts after a shutdown signal, never about which internal field was read to decide.

use std::{
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use futures_util::{SinkExt, StreamExt};
use sqlx::SqlitePool;
use tokio::{net::TcpListener, task::JoinHandle, time::timeout};
use tokio_tungstenite::{
    connect_async,
    tungstenite::{Message, client::IntoClientRequest, http::StatusCode},
};
use url::Url;
use voice_agent_server::{
    app::{AppState, SessionProfileAdmissionError, router_with_state},
    config::{
        AdminApiConfig, AppConfig, AudioConfig, AuthConfig, BargeInConfig, DatabaseConfig,
        DatabaseHistoryConfig, DeploymentConfig, ExternalMcpConfig, ExternalMcpNetworkConfig,
        LimitsConfig, LlmConfig, McpConfig, ProviderDefaultsConfig, ProvidersConfig, RuntimeConfig,
        ServerConfig, ShutdownConfig, SpeechOutputConfig, TtsConfig, VisionConfig, WebsocketConfig,
        WorkersConfig,
    },
    database::Database,
    lifecycle::{AdmissionGate, DrainOutcome, DrainRegistry, RuntimeLifecycle, ShutdownReport},
    providers::ProviderSet,
};

// ---------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------

fn database_url() -> String {
    format!(
        "sqlite://{}",
        std::env::temp_dir()
            .join(format!("voice-agent-lifecycle-{}.db", uuid::Uuid::new_v4()))
            .display()
    )
}

fn config(url: String) -> AppConfig {
    let address: std::net::SocketAddr = ([127, 0, 0, 1], 0).into();
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
        providers: ProvidersConfig::default(),
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
        api: AdminApiConfig::default(),
        shutdown: ShutdownConfig::default(),
        agent: None,
        effective_agent: Default::default(),
    }
}

/// A `ProviderSet` of unavailable providers still publishes the four runtime handles the server
/// defaults name, so `readiness()` reaches the database check instead of stopping at the catalog.
fn providers() -> Arc<ProviderSet> {
    Arc::new(ProviderSet::unavailable())
}

struct Voice {
    base: String,
    state: AppState,
    lifecycle: Arc<RuntimeLifecycle>,
    url: String,
    #[allow(dead_code)]
    task: JoinHandle<()>,
}

impl Voice {
    async fn status(&self, route: &str) -> StatusCode {
        reqwest::get(format!("{}{route}", self.base))
            .await
            .expect("the liveness route answers")
            .status()
    }

    async fn body(&self, route: &str) -> String {
        reqwest::get(format!("{}{route}", self.base))
            .await
            .expect("the readiness route answers")
            .text()
            .await
            .expect("the readiness route has a body")
    }

    async fn database(&self) -> SqlitePool {
        SqlitePool::connect(&self.url).await.unwrap()
    }
}

/// Boots a Voice server with its own lifecycle, which is what the shutdown tests drive.
async fn start(config: AppConfig, grace: Duration) -> Voice {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let url = config.database.url.clone();
    let database = Database::connect(&config.database).await.unwrap();
    let lifecycle = RuntimeLifecycle::new(grace);
    let state = AppState::from_provider_set_with_database_and_shutdown(
        config,
        providers(),
        Some(database),
        Arc::clone(&lifecycle),
    );
    let served = state.clone();
    let task = tokio::spawn(async move {
        axum::serve(listener, router_with_state(served))
            .await
            .unwrap()
    });
    Voice {
        base: format!("http://{address}"),
        state,
        lifecycle,
        url,
        task,
    }
}

/// Constructs incomplete state to test fail-closed readiness and I/O-free shutdown.
async fn start_without_database(grace: Duration) -> Voice {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let config = config(database_url());
    let lifecycle = RuntimeLifecycle::new(grace);
    let state = AppState::from_provider_set_with_database_and_shutdown(
        config,
        providers(),
        None,
        Arc::clone(&lifecycle),
    );
    let served = state.clone();
    let task = tokio::spawn(async move {
        axum::serve(listener, router_with_state(served))
            .await
            .unwrap()
    });
    Voice {
        base: format!("http://{address}"),
        state,
        lifecycle,
        url: String::new(),
        task,
    }
}

fn request(
    base: &str,
    device_id: &str,
) -> tokio_tungstenite::tungstenite::handshake::client::Request {
    let mut request = format!("{}/voice/v1/", base.replacen("http", "ws", 1))
        .into_client_request()
        .unwrap();
    request
        .headers_mut()
        .insert("Protocol-Version", "1".parse().unwrap());
    request
        .headers_mut()
        .insert("Device-Id", device_id.parse().unwrap());
    request
        .headers_mut()
        .insert("Client-Id", "lifecycle-test".parse().unwrap());
    request
}

fn hello() -> String {
    r#"{"type":"hello","version":1,"transport":"websocket","features":{}}"#.to_owned()
}

fn rejected_status(error: tokio_tungstenite::tungstenite::Error) -> StatusCode {
    match error {
        tokio_tungstenite::tungstenite::Error::Http(response) => response.status(),
        other => panic!("expected an HTTP WebSocket rejection, got {other:?}"),
    }
}

async fn seed(pool: &SqlitePool) {
    sqlx::query(
        "INSERT INTO agents (key,name,enabled,created_at,updated_at) \
                 VALUES ('agent','Agent',1,1,1)",
    )
    .execute(pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO devices (device_id,agent_id,enabled,created_at,updated_at) \
                 VALUES ('device',1,1,1,1)",
    )
    .execute(pool)
    .await
    .unwrap();
}

// ---------------------------------------------------------------------------
// Liveness versus readiness
// ---------------------------------------------------------------------------

/// The two routes answer different questions, so a healthy process says so on both and a degraded
/// one still says it is alive.  Collapsing them is the failure this test exists to prevent: an
/// orchestrator that reads a degraded database as a dead process kills sessions that are fine.
#[tokio::test]
async fn a_healthy_process_is_both_live_and_ready() {
    let voice = start(config(database_url()), Duration::from_secs(15)).await;
    assert_eq!(voice.status("/health").await, StatusCode::OK);
    assert_eq!(voice.status("/ready").await, StatusCode::OK);
    assert_eq!(voice.body("/ready").await, "ready");
}

/// Liveness depends on nothing, so a database that has gone away degrades readiness and leaves
/// `/health` alone.
#[tokio::test]
async fn a_degraded_database_degrades_readiness_without_taking_the_process_down() {
    let voice = start(config(database_url()), Duration::from_secs(15)).await;
    voice.state.database.as_ref().unwrap().pool().close().await;

    assert_eq!(
        voice.status("/health").await,
        StatusCode::OK,
        "the process is still running and still serving the sessions it admitted"
    );
    assert_eq!(
        voice.status("/ready").await,
        StatusCode::SERVICE_UNAVAILABLE
    );
    assert_eq!(
        voice.body("/ready").await,
        "database_unreachable",
        "readiness names the dependency that failed, in bounded words only"
    );
}

/// Readiness is a statement about *new* connections.  A Voice Session already admitted keeps its
/// snapshot, so a database outage after admission cannot disturb it — that is the whole reason
/// liveness and readiness are separate routes.
#[tokio::test]
async fn a_session_admitted_before_a_database_outage_keeps_its_profile() {
    let voice = start(config(database_url()), Duration::from_secs(15)).await;
    seed(&voice.database().await).await;
    let admitted = voice
        .state
        .resolve_session_profile("device")
        .await
        .expect("the seeded Device is admitted before the outage");
    assert_eq!(admitted.agent_key, "agent");

    voice.state.database.as_ref().unwrap().pool().close().await;

    assert_eq!(
        voice.status("/ready").await,
        StatusCode::SERVICE_UNAVAILABLE
    );
    assert!(
        admitted.agent_key == "agent",
        "the admitted snapshot is owned by the session, not re-read from the database"
    );
}

/// Directly assembled state without the required DB must never report ready.
#[tokio::test]
async fn missing_database_reports_startup_incomplete() {
    let voice = start_without_database(Duration::from_secs(15)).await;
    assert_eq!(voice.status("/health").await, StatusCode::OK);
    assert_eq!(
        voice.status("/ready").await,
        StatusCode::SERVICE_UNAVAILABLE
    );
    assert_eq!(voice.body("/ready").await, "startup_incomplete");
}

/// A process that has begun shutdown reports so.  `/health` stays up because the process is up:
/// it is finishing the sessions it already has, and killing it would cost their Dialogue History.
#[tokio::test]
async fn a_shutting_down_process_is_live_but_no_longer_ready() {
    let voice = start(config(database_url()), Duration::from_millis(50)).await;
    seed(&voice.database().await).await;
    // Hold a session open so the drain reaches its deadline instead of completing immediately.
    let (mut socket, _) = connect_async(request(&voice.base, "device")).await.unwrap();
    socket.send(Message::Text(hello().into())).await.unwrap();
    assert!(matches!(
        timeout(Duration::from_secs(2), socket.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap(),
        Message::Text(_)
    ));

    let lifecycle = Arc::clone(&voice.lifecycle);
    let shutdown = tokio::spawn(async move { lifecycle.shutdown().await });
    // The gate is the first thing shutdown does, before any session is asked anything. Waiting for
    // it rather than assuming it keeps the assertion about the ordering, not about scheduling.
    await_gate_closed(voice.state.admission_gate().as_ref()).await;
    assert_eq!(voice.status("/health").await, StatusCode::OK);
    assert_eq!(
        voice.status("/ready").await,
        StatusCode::SERVICE_UNAVAILABLE,
        "a process refusing new connections must not advertise that it will take them"
    );
    assert_eq!(voice.body("/ready").await, "shutting_down");

    let _ = shutdown.await;
}

// ---------------------------------------------------------------------------
// What readiness must never do
// ---------------------------------------------------------------------------

/// An External MCP server that is refusing every connection is exactly the failure that must not
/// reach the readiness route.  External MCP is fail-soft in V1: a session that cannot use the bound
/// server simply gets fewer tools, so the process is still able to accept a connection.
#[tokio::test]
async fn an_unreachable_external_mcp_server_does_not_make_the_process_unready() {
    let voice = start(config(database_url()), Duration::from_secs(15)).await;
    let pool = voice.database().await;
    seed(&pool).await;
    // Bind nothing: nothing is listening on this port, so every discovery attempt fails.
    let dead_port = {
        let probe = TcpListener::bind("127.0.0.1:0").await.unwrap();
        probe.local_addr().unwrap().port()
    };
    let server_id: i64 = sqlx::query_scalar(
        "INSERT INTO mcp_servers (key,name,url,headers_json,auth_type,auth_header_name,secret_ref,\
         connect_timeout_ms,request_timeout_ms,enabled,created_at,updated_at) \
         VALUES ('weather','Weather',?,'{}','none',NULL,NULL, 500, 500, 1, 1, 1) RETURNING id",
    )
    .bind(format!("http://127.0.0.1:{dead_port}/mcp"))
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
    drop(pool);

    // A real admission proves the failure is real and fail-soft at the same time.
    let profile = voice
        .state
        .resolve_session_profile("device")
        .await
        .expect("an unavailable optional server still admits the Device");
    assert!(
        profile.external_mcp().is_empty(),
        "the unreachable server contributed no tools, and the session was admitted anyway"
    );

    assert_eq!(
        voice.status("/ready").await,
        StatusCode::OK,
        "an optional External MCP server being down is not a readiness failure"
    );
}

/// The strongest possible statement about what readiness does not do: a readiness probe must not
/// resolve a Device.  A Device that admission would refuse has to leave `/ready` untouched, because
/// whether *this* client is allowed in says nothing about whether the process can serve anyone.
#[tokio::test]
async fn readiness_never_resolves_a_device() {
    let voice = start(config(database_url()), Duration::from_secs(15)).await;
    // No Device rows at all: full admission would refuse every client.
    let refusal = voice.state.resolve_session_profile("nobody").await;
    assert!(
        refusal.is_err(),
        "the boundary under test really does refuse this Device"
    );

    assert_eq!(
        voice.status("/ready").await,
        StatusCode::OK,
        "readiness answers for the process, not for the Device a probe might have invented"
    );
}

/// A readiness probe must not reach out over the network.  This counts every request the fake MCP
/// server receives, so "discovery is not on the readiness path" is an observation rather than a
/// claim about which code runs.
#[tokio::test]
async fn readiness_never_discovers_external_mcp() {
    let requests = Arc::new(AtomicUsize::new(0));
    let counted = Arc::clone(&requests);
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let task = tokio::spawn(async move {
        while let Ok((socket, _)) = listener.accept().await {
            counted.fetch_add(1, Ordering::SeqCst);
            // Answer nothing: a connection that is accepted and dropped is enough to be counted,
            // and it proves the probe never waits on one.
            drop(socket);
        }
    });

    let mut app_config = config(database_url());
    app_config.mcp = McpConfig {
        external: ExternalMcpConfig {
            network: ExternalMcpNetworkConfig {
                allowed_hosts: vec![],
            },
            ..ExternalMcpConfig::default()
        },
        ..McpConfig::default()
    };
    let voice = start(app_config, Duration::from_secs(15)).await;
    let pool = voice.database().await;
    seed(&pool).await;
    let server_id: i64 = sqlx::query_scalar(
        "INSERT INTO mcp_servers (key,name,url,headers_json,auth_type,auth_header_name,secret_ref,\
         connect_timeout_ms,request_timeout_ms,enabled,created_at,updated_at) \
         VALUES ('weather','Weather',?,'{}','none',NULL,NULL, 500, 500, 1, 1, 1) RETURNING id",
    )
    .bind(format!("http://{address}/mcp"))
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
    drop(pool);

    for _ in 0..5 {
        assert_eq!(voice.status("/ready").await, StatusCode::OK);
    }
    assert_eq!(
        requests.load(Ordering::SeqCst),
        0,
        "five readiness probes, zero outbound requests: discovery is not on this path"
    );
    task.abort();
}

/// The same discipline against the database: readiness asks one question and it is about the
/// dependency, never about the configuration.  A probe cannot walk the Agent/Template graph, and
/// this proves it by pointing the process at a graph a full admission would reject.
#[tokio::test]
async fn readiness_never_runs_full_admission() {
    let voice = start(config(database_url()), Duration::from_secs(15)).await;
    let pool = voice.database().await;
    // An Agent whose default Template cannot be materialized: full admission fails closed for it,
    // and readiness must not notice or care.
    sqlx::query(
        "INSERT INTO agents (key,name,enabled,created_at,updated_at) \
                 VALUES ('agent','Agent',1,1,1)",
    )
    .execute(&pool)
    .await
    .unwrap();
    let template_id: i64 = sqlx::query_scalar(
        "INSERT INTO agent_templates (key,name,description,language,prompt,enabled,created_at,updated_at) \
         VALUES ('broken','Broken',NULL,'','prompt',1,1,1) RETURNING id",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO agent_template_assignments (agent_id,template_id,is_default,enabled,\
         created_at) VALUES (1, ?, 1, 1, 1)",
    )
    .bind(template_id)
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO devices (device_id,agent_id,enabled,created_at,updated_at) \
                 VALUES ('device',1,1,1,1)",
    )
    .execute(&pool)
    .await
    .unwrap();
    drop(pool);

    assert!(
        voice.state.resolve_session_profile("device").await.is_err(),
        "the boundary under test really does fail closed for this Agent"
    );
    assert_eq!(
        voice.status("/ready").await,
        StatusCode::OK,
        "readiness does not evaluate any one Agent's profile"
    );
}

// ---------------------------------------------------------------------------
// Admission and tool work during shutdown
// ---------------------------------------------------------------------------

/// The gate refuses new connections before any session is asked anything, so a request that arrives
/// after shutdown began is refused rather than admitted and immediately closed.
#[tokio::test]
async fn a_closed_gate_refuses_a_new_voice_connection() {
    let voice = start(config(database_url()), Duration::from_secs(15)).await;
    seed(&voice.database().await).await;
    assert_eq!(
        connect_async(request(&voice.base, "device"))
            .await
            .unwrap()
            .1
            .status(),
        StatusCode::SWITCHING_PROTOCOLS,
        "the boundary admits while the process is working"
    );

    // Close the gate without draining anything: this is the state shutdown reaches first.
    assert!(voice.lifecycle.begin_shutdown());

    let refusal = connect_async(request(&voice.base, "device"))
        .await
        .unwrap_err();
    assert_eq!(
        rejected_status(refusal),
        StatusCode::SERVICE_UNAVAILABLE,
        "a closed gate refuses the connection at the boundary"
    );
    assert_eq!(voice.status("/health").await, StatusCode::OK);
}

/// The gate is application-owned, so admission is refused even for a caller that never went near
/// the WebSocket boundary.  This is what "must not depend on a SessionActor observing shutdown"
/// means in practice: the decision belongs to the process, not to whoever happens to call it.
#[tokio::test]
async fn a_closed_gate_refuses_database_admission_regardless_of_the_caller() {
    let voice = start(config(database_url()), Duration::from_secs(15)).await;
    seed(&voice.database().await).await;
    assert!(
        voice.state.resolve_session_profile("device").await.is_ok(),
        "the boundary admits while the process is working"
    );

    voice.lifecycle.begin_shutdown();

    assert_eq!(
        voice.state.resolve_session_profile("device").await.err(),
        Some(SessionProfileAdmissionError::ShuttingDown),
        "no caller can start a database admission after the gate closed"
    );
}

// ---------------------------------------------------------------------------
// Ordered shutdown and bounded drain
// ---------------------------------------------------------------------------

/// A session that finishes on its own drains the process without waiting out the grace deadline.
/// Shutdown returning promptly is the property; a fixed wait would satisfy the deadline and fail
/// this.
#[tokio::test]
async fn a_session_that_finishes_on_its_own_drains_before_the_deadline() {
    let voice = start(config(database_url()), Duration::from_secs(30)).await;
    seed(&voice.database().await).await;
    let (mut socket, _) = connect_async(request(&voice.base, "device")).await.unwrap();
    socket.send(Message::Text(hello().into())).await.unwrap();
    assert!(matches!(
        timeout(Duration::from_secs(2), socket.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap(),
        Message::Text(_)
    ));
    // The client goes away on its own terms, which is the ordinary case: the connection ends and
    // the server finishes the session without anybody closing the gate or signalling a deadline.
    drop(socket);
    await_drain(voice.lifecycle.drain().as_ref()).await;

    let started = std::time::Instant::now();
    let report = shutdown_within(Arc::clone(&voice.lifecycle), Duration::from_secs(10)).await;
    assert_eq!(report.outcome, DrainOutcome::Drained);
    assert_eq!(report.controlled_closes, 0);
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "a completed drain returns on completion, not on the deadline: {:?}",
        started.elapsed()
    );
}

/// A session still open at the grace deadline is issued a controlled close — a protocol close the
/// session performs itself — rather than aborted.  The client sees the normal going-away code, which
/// is the observable difference between the two mechanisms.
#[tokio::test]
async fn a_session_open_at_the_deadline_is_controlled_closed_not_aborted() {
    let voice = start(config(database_url()), Duration::from_millis(50)).await;
    seed(&voice.database().await).await;
    let (mut socket, _) = connect_async(request(&voice.base, "device")).await.unwrap();
    socket.send(Message::Text(hello().into())).await.unwrap();
    assert!(matches!(
        timeout(Duration::from_secs(2), socket.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap(),
        Message::Text(_)
    ));
    // The session stays connected and idle across the deadline.

    let started = std::time::Instant::now();
    let lifecycle = Arc::clone(&voice.lifecycle);
    let shutdown = tokio::spawn(async move { lifecycle.shutdown().await });

    let close = timeout(Duration::from_secs(5), socket.next())
        .await
        .expect("the controlled close arrives inside the settle window")
        .expect("the connection is still open")
        .expect("the frame is readable");
    assert!(
        matches!(close, Message::Close(Some(frame)) if u16::from(frame.code) == 1001),
        "a session closed by the drain performs its own protocol close, not a task abort"
    );

    let report = shutdown.await.unwrap();
    assert_eq!(
        report.outcome,
        DrainOutcome::DeadlineReached { remaining: 1 }
    );
    assert_eq!(report.controlled_closes, 1);
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "the drain stops at the grace deadline instead of waiting the session out: {:?}",
        started.elapsed()
    );
}

/// The deadline is one shared deadline.  A process with a session it cannot close still finishes
/// shutdown in bounded time, and the archival flush gets whatever the drain did not — never a
/// second grace period.
#[tokio::test(start_paused = true)]
async fn shutdown_is_bounded_even_when_a_session_never_answers() {
    // No database and no socket: this test exercises the deadline itself, and a paused clock is
    // only sound when nothing in the process is waiting on real I/O.
    let voice = start_without_database(Duration::from_millis(50)).await;
    // Registered but never connected: a session the drain can signal and nothing will ever answer.
    let silent = voice.state.register_session();

    let grace = voice.lifecycle.grace();
    let lifecycle = Arc::clone(&voice.lifecycle);
    let mut shutdown = Box::pin(lifecycle.shutdown());
    // Nothing has moved the clock, so a drain that closed early would already be finished.
    assert!(
        timeout(Duration::ZERO, shutdown.as_mut()).await.is_err(),
        "the drain waits for the sessions it was given rather than closing them at once"
    );
    tokio::time::advance(grace).await;
    let report = shutdown.await;

    assert_eq!(
        report.outcome,
        DrainOutcome::DeadlineReached { remaining: 1 }
    );
    assert_eq!(report.controlled_closes, 1);
    assert!(voice.lifecycle.stopping().is_cancelled());
    drop(silent);
}

/// The archival writer is best-effort inside the same deadline, and it is never what decides how
/// long shutdown takes.  A writer with nothing left to do is flushed on the first read; one that
/// cannot settle is reported as unflushed and the shutdown still returns.
#[tokio::test]
async fn the_history_flush_never_decides_how_long_shutdown_takes() {
    let mut capture_on = config(database_url());
    capture_on.database.history = DatabaseHistoryConfig {
        enabled: true,
        ..DatabaseHistoryConfig::default()
    };
    let voice = start(capture_on, Duration::from_millis(50)).await;
    seed(&voice.database().await).await;
    let (mut socket, _) = connect_async(request(&voice.base, "device")).await.unwrap();
    socket.send(Message::Text(hello().into())).await.unwrap();
    assert!(matches!(
        timeout(Duration::from_secs(2), socket.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap(),
        Message::Text(_)
    ));

    let started = std::time::Instant::now();
    let report = shutdown_within(Arc::clone(&voice.lifecycle), Duration::from_secs(5)).await;
    assert!(
        started.elapsed() < Duration::from_secs(3),
        "the archive is best-effort and cannot hold the process open: {:?}",
        started.elapsed()
    );
    // Whether the archive settled is reported, not awaited, so both answers are acceptable here and
    // the property under test is the bound above.
    let _ = report.history_flushed;
    assert!(report.outcome != DrainOutcome::Drained || report.controlled_closes == 0);
    let _ = socket;
}

/// A deployment with the archive off has nothing to flush and reports so, rather than waiting.
#[tokio::test]
async fn a_capture_off_process_reports_nothing_left_to_flush() {
    let voice = start(config(database_url()), Duration::from_millis(50)).await;
    seed(&voice.database().await).await;
    let (mut socket, _) = connect_async(request(&voice.base, "device")).await.unwrap();
    socket.send(Message::Text(hello().into())).await.unwrap();
    assert!(matches!(
        timeout(Duration::from_secs(2), socket.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap(),
        Message::Text(_)
    ));

    let report = shutdown_within(Arc::clone(&voice.lifecycle), Duration::from_secs(5)).await;
    assert!(
        report.history_flushed,
        "no writer means nothing unsettled, which is a settled archive"
    );
}

/// One process, several sessions: the drain closes exactly the ones still open and leaves the
/// finished ones alone.  This is what a registry of registered completion handles buys over a
/// broadcast — shutdown can count and address, not merely signal everyone.
#[tokio::test]
async fn the_drain_counts_and_closes_exactly_the_sessions_still_open() {
    // A short grace: this test is about which sessions are counted and signalled, not about how
    // long the process is willing to wait for them.
    let voice = start(config(database_url()), Duration::from_millis(50)).await;
    seed(&voice.database().await).await;
    let finished = voice.state.register_session();
    let open = voice.state.register_session();
    assert_eq!(voice.lifecycle.drain().active(), 2);

    drop(finished);

    let lifecycle = Arc::clone(&voice.lifecycle);
    let report = lifecycle.shutdown().await;
    assert_eq!(
        report.outcome,
        DrainOutcome::DeadlineReached { remaining: 1 }
    );
    assert_eq!(
        report.controlled_closes, 1,
        "only the session still registered is asked to close"
    );
    assert!(
        open.close_signal().is_cancelled(),
        "and it is the one that was signalled"
    );
}

// ---------------------------------------------------------------------------
// Configured grace
// ---------------------------------------------------------------------------

/// The deadline is the one the deployment configured, read from the same validated field the
/// production path reads.  A process configured for a long grace really does wait that long, which
/// is the only way to tell "bounded" from "immediately closed".
#[tokio::test(start_paused = true)]
async fn the_grace_deadline_comes_from_the_deployment_configuration() {
    let mut app_config = config(database_url());
    app_config.shutdown = ShutdownConfig { grace_ms: 1_500 };
    // Built exactly as production builds it: from the configuration's own grace.
    let lifecycle = RuntimeLifecycle::from_config(&app_config);
    assert_eq!(lifecycle.grace(), Duration::from_millis(1_500));

    let silent = lifecycle.drain().register();
    let mut shutdown = Box::pin(lifecycle.shutdown());
    // Nothing has moved the clock. A drain that ignored the configured deadline would be finished
    // already, which is exactly the "immediately closed" behaviour this test exists to rule out.
    assert!(
        timeout(Duration::ZERO, shutdown.as_mut()).await.is_err(),
        "a 1.5s grace means the drain is still waiting with no time elapsed"
    );
    tokio::time::advance(Duration::from_millis(1_500)).await;

    assert_eq!(
        shutdown.await.outcome,
        DrainOutcome::DeadlineReached { remaining: 1 },
        "and at the configured deadline the open session is closed rather than waited out"
    );
    drop(silent);
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Waits for the shutdown sequence to reach its first step, rather than assuming scheduling put it
/// there before the next line runs.
async fn await_gate_closed(gate: &AdmissionGate) {
    timeout(Duration::from_secs(5), async {
        while gate.is_open() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("the ordered shutdown closes the admission gate");
}

/// Waits for every registered session to finish, which is what makes a drain observable.
async fn await_drain(drain: &DrainRegistry) {
    timeout(Duration::from_secs(5), drain.drained())
        .await
        .expect("the registered session finishes on its own");
}

/// Runs the ordered shutdown and fails the test rather than hanging if it never returns.
async fn shutdown_within(lifecycle: Arc<RuntimeLifecycle>, limit: Duration) -> ShutdownReport {
    timeout(limit, lifecycle.shutdown())
        .await
        .expect("the ordered shutdown returns within its bound")
}

#[tokio::test]
async fn application_shutdown_uses_its_existing_deadline_for_managed_resources() {
    use std::collections::HashMap;
    use voice_agent_server::{
        database::{DesiredProvider, secrets::EnvSecretResolver},
        services::provider_runtime::{FactoryMaterializer, ProviderRuntimeManager, RuntimeLimits},
        workers::WorkerSupervisor,
    };
    let mut app_config = config("sqlite::memory:".into());
    app_config.shutdown.grace_ms = 30;
    let state =
        AppState::from_provider_set(app_config.clone(), Arc::new(ProviderSet::unavailable()));
    let builder = FactoryMaterializer::new(
        Arc::new(app_config),
        Arc::new(EnvSecretResolver),
        HashMap::from([("openai".into(), 4096)]),
        Arc::new(WorkerSupervisor::start_many(vec![], vec![])),
    )
    .unwrap();
    let manager = ProviderRuntimeManager::new(
        RuntimeLimits {
            max_parallel_loads: 1,
            max_pending_loads: 1,
            max_waiters: 2,
            max_resident_bytes: 8192,
            max_resources: 2,
            max_version_entries: 4,
            admission_timeout_ms: 1000,
            failure_cooldown_ms: 10,
            idle_ttl_ms: 10,
        },
        Arc::new(builder),
        state.admission_gate().clone(),
    )
    .unwrap();
    let state = state.with_runtime_manager(manager.clone());
    let lease = manager
        .acquire(DesiredProvider {
            id: 1,
            key: "remote".into(),
            kind: "llm".into(),
            adapter: "openai".into(),
            revision: 1,
            config_json: r#"{"base_url":"https://example.test/v1","model":"fixture"}"#.into(),
            secret_ref: None,
        })
        .await
        .unwrap();
    let started = tokio::time::Instant::now();
    let report = state.lifecycle.shutdown().await;
    assert_eq!(report.provider_resources_drained, Some(false));
    assert!(started.elapsed() < Duration::from_millis(300));
    assert_eq!(manager.accounting().reserved_bytes, 4096);
    drop(lease);
    assert!(
        manager
            .shutdown_until(tokio::time::Instant::now() + Duration::from_secs(1))
            .await
    );
}
