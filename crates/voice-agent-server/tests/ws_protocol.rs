mod support;

use axum::Router;
use futures_util::{SinkExt, StreamExt};
use std::sync::Arc;
use tokio::{
    net::TcpListener,
    task::JoinHandle,
    time::{Duration, timeout},
};
use tokio_tungstenite::{
    connect_async,
    tungstenite::{Message, client::IntoClientRequest},
};
use url::Url;
use voice_agent_server::{
    app::{AppState, router_with_state},
    config::{
        AppConfig, AudioConfig, AuthConfig, BargeInConfig, DeploymentConfig, LimitsConfig,
        LlmConfig, ProvidersConfig, RuntimeConfig, ServerConfig, SpeechOutputConfig, TtsConfig,
        WebsocketConfig, WorkersConfig,
    },
    providers::ProviderSet,
};

async fn start(max_frame_bytes: usize) -> (String, JoinHandle<()>) {
    start_with_token(max_frame_bytes, String::new()).await
}

async fn start_with_token(max_frame_bytes: usize, token: String) -> (String, JoinHandle<()>) {
    let (base, task, _) = start_with_token_and_lifecycle(max_frame_bytes, token, None).await;
    (base, task)
}

/// `lifecycle` is `None` for a router that is only ever torn down by aborting its task, and
/// `Some` for one whose shutdown is driven through the application lifecycle.
async fn start_with_token_and_lifecycle(
    max_frame_bytes: usize,
    token: String,
    lifecycle: Option<std::sync::Arc<voice_agent_server::lifecycle::RuntimeLifecycle>>,
) -> (
    String,
    JoinHandle<()>,
    Option<std::sync::Arc<voice_agent_server::lifecycle::RuntimeLifecycle>>,
) {
    let (base, task, lifecycle, _) =
        start_with_state(max_frame_bytes, token, lifecycle, |config| config).await;
    (base, task, lifecycle)
}

/// Starts the protocol router and also returns the application state it serves, for tests that
/// must observe or hold application-owned coordination while a connection is admitted.
async fn start_with_state(
    max_frame_bytes: usize,
    token: String,
    lifecycle: Option<std::sync::Arc<voice_agent_server::lifecycle::RuntimeLifecycle>>,
    configure: impl FnOnce(AppConfig) -> AppConfig,
) -> (
    String,
    JoinHandle<()>,
    Option<std::sync::Arc<voice_agent_server::lifecycle::RuntimeLifecycle>>,
    AppState,
) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let config = AppConfig {
        orchestrator: Default::default(),
        wiki: Default::default(),
        speaker_recognition: Default::default(),
        server: ServerConfig {
            bind: address,
            public_ws_url: Url::parse(&format!("ws://{address}/voice/v1/")).unwrap(),
            hello_timeout_ms: 500,
        },
        auth: AuthConfig { token },
        audio: AudioConfig::default(),
        websocket: WebsocketConfig {
            max_frame_bytes,
            ..WebsocketConfig::default()
        },
        limits: LimitsConfig::default(),
        provider_defaults: voice_agent_server::config::ProviderDefaultsConfig {
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
        mcp: voice_agent_server::config::McpConfig::default(),
        system_mcp: Default::default(),
        vision: voice_agent_server::config::VisionConfig::default(),
        database: voice_agent_server::config::DatabaseConfig::default(),
        api: voice_agent_server::config::AdminApiConfig::default(),
        shutdown: voice_agent_server::config::ShutdownConfig::default(),
        agent: None,
        effective_agent: voice_agent_server::config::EffectiveAgentConfig::default(),
    };
    let config = configure(config);
    let (config, database) = support::provision(config).await;
    let state = AppState::from_provider_set_with_database_and_shutdown(
        config,
        Arc::new(ProviderSet::unavailable()),
        Some(database),
        lifecycle.clone().unwrap_or_else(|| {
            voice_agent_server::lifecycle::RuntimeLifecycle::new(std::time::Duration::from_millis(
                1_024,
            ))
        }),
    );
    let app: Router = router_with_state(state.clone());
    let task = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    (format!("http://{address}"), task, lifecycle, state)
}

fn request(base: &str) -> tokio_tungstenite::tungstenite::handshake::client::Request {
    let mut request = format!("{}/voice/v1/", base.replace("http", "ws"))
        .into_client_request()
        .unwrap();
    let headers = request.headers_mut();
    headers.insert("Protocol-Version", "1".parse().unwrap());
    headers.insert("Device-Id", "reference-client-01".parse().unwrap());
    headers.insert("Client-Id", "test-client".parse().unwrap());
    request
}

fn hello() -> String {
    r#"{"type":"hello","version":1,"transport":"websocket","audio_params":{"format":"opus","sample_rate":16000,"channels":1,"frame_duration":60},"future_field":true}"#.into()
}

async fn next_message<S>(socket: &mut tokio_tungstenite::WebSocketStream<S>) -> Message
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    timeout(Duration::from_secs(1), socket.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap()
}

#[tokio::test]
async fn ota_advertises_ws_url_and_health_is_available() {
    let (base, task) = start(1_024).await;
    assert_eq!(
        reqwest::get(format!("{base}/health"))
            .await
            .unwrap()
            .text()
            .await
            .unwrap(),
        "ok"
    );
    let ota: serde_json::Value = reqwest::Client::new()
        .post(format!("{base}/voice/ota/"))
        .header("Device-Id", "reference-client-01")
        .header("Client-Id", "test-client")
        .json(&serde_json::json!({}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(
        ota["websocket"]["url"],
        format!("ws://{}/voice/v1/", base.trim_start_matches("http://"))
    );
    task.abort();
}

#[tokio::test]
async fn application_shutdown_controlled_closes_an_upgraded_voice_session() {
    let (base, task, lifecycle) = start_with_token_and_lifecycle(
        1_024,
        String::new(),
        Some(voice_agent_server::lifecycle::RuntimeLifecycle::new(
            std::time::Duration::from_millis(50),
        )),
    )
    .await;
    let (mut socket, _) = connect_async(request(&base)).await.unwrap();
    socket.send(Message::Text(hello().into())).await.unwrap();
    assert!(matches!(next_message(&mut socket).await, Message::Text(_)));

    lifecycle
        .expect("this router is driven through its lifecycle")
        .shutdown()
        .await;
    let close = next_message(&mut socket).await;
    assert!(matches!(
        close,
        Message::Close(Some(frame)) if u16::from(frame.code) == 1001
    ));
    task.abort();
}

#[tokio::test]
async fn ota_accepts_post_and_advertises_preflight_methods() {
    let (base, task) = start(1_024).await;
    let client = reqwest::Client::new();
    let ota = client
        .post(format!("{base}/voice/ota/"))
        .header("Device-Id", "reference-client-01")
        .header("Client-Id", "test-client")
        .header(reqwest::header::ORIGIN, "http://localhost:3000")
        .json(&serde_json::json!({}))
        .send()
        .await
        .unwrap();
    assert_eq!(ota.status(), reqwest::StatusCode::OK);
    assert_eq!(
        ota.headers()[reqwest::header::ACCESS_CONTROL_ALLOW_ORIGIN],
        "http://localhost:3000"
    );

    let preflight = client
        .request(reqwest::Method::OPTIONS, format!("{base}/voice/ota/"))
        .header(reqwest::header::ORIGIN, "http://localhost:3000")
        .send()
        .await
        .unwrap();
    assert_eq!(preflight.status(), reqwest::StatusCode::NO_CONTENT);
    assert_eq!(
        preflight.headers()[reqwest::header::ALLOW],
        "GET, POST, OPTIONS"
    );
    assert_eq!(
        preflight.headers()[reqwest::header::ACCESS_CONTROL_ALLOW_METHODS],
        "GET, POST, OPTIONS"
    );
    assert_eq!(
        preflight.headers()[reqwest::header::ACCESS_CONTROL_ALLOW_ORIGIN],
        "http://localhost:3000"
    );
    task.abort();
}

#[tokio::test]
async fn valid_hello_receives_canonical_server_hello() {
    let (base, task) = start(1_024).await;
    let (mut socket, _) = connect_async(request(&base)).await.unwrap();
    socket.send(Message::Text(hello().into())).await.unwrap();
    let message = match next_message(&mut socket).await {
        Message::Text(text) => text,
        other => panic!("expected ServerHello, got {other:?}"),
    };
    let json: serde_json::Value = serde_json::from_str(&message).unwrap();
    assert_eq!(json["type"], "hello");
    assert_eq!(json["audio_params"]["sample_rate"], 24_000);
    task.abort();
}

#[tokio::test]
async fn mcp_initialize_follows_server_hello_when_client_advertises_capability() {
    let (base, task) = start(1_024).await;
    let (mut socket, _) = connect_async(request(&base)).await.unwrap();
    socket.send(Message::Text(r#"{"type":"hello","version":1,"transport":"websocket","features":{"mcp":true},"audio_params":{"format":"opus","sample_rate":16000,"channels":1,"frame_duration":60}}"#.into())).await.unwrap();
    let hello_text = match next_message(&mut socket).await {
        Message::Text(text) => text,
        other => panic!("expected ServerHello, got {other:?}"),
    };
    let hello: serde_json::Value = serde_json::from_str(&hello_text).unwrap();
    assert_eq!(hello["type"], "hello");
    let initialize_text = match next_message(&mut socket).await {
        Message::Text(text) => text,
        other => panic!("expected MCP initialize, got {other:?}"),
    };
    let initialize: serde_json::Value = serde_json::from_str(&initialize_text).unwrap();
    assert_eq!(initialize["type"], "mcp");
    assert_eq!(initialize["payload"]["id"], 1);
    assert_eq!(initialize["payload"]["method"], "initialize");
    task.abort();
}

#[tokio::test]
async fn websocket_accepts_browser_identity_query_when_headers_are_absent() {
    let (base, task) = start(1_024).await;
    let mut browser_request = request(&base);
    browser_request.headers_mut().remove("Device-Id");
    browser_request.headers_mut().remove("Client-Id");
    browser_request.headers_mut().remove("Protocol-Version");
    *browser_request.uri_mut() = format!(
        "{}/voice/v1/?device-id=browser-client&client-id=browser-ui",
        base.replace("http", "ws")
    )
    .parse()
    .unwrap();
    let (mut socket, _) = connect_async(browser_request).await.unwrap();
    socket
        .send(Message::Text(
            r#"{"type":"hello","device_id":"browser-client","features":{"mcp":true}}"#.into(),
        ))
        .await
        .unwrap();
    let message = next_message(&mut socket).await;
    assert!(matches!(message, Message::Text(_)));
    task.abort();
}

#[tokio::test]
async fn websocket_accepts_browser_authorization_query() {
    let token = "browser-test-token";
    let (base, task) = start_with_token(1_024, token.to_owned()).await;
    let mut browser_request = request(&base);
    browser_request.headers_mut().remove("Device-Id");
    browser_request.headers_mut().remove("Client-Id");
    browser_request.headers_mut().remove("Protocol-Version");
    *browser_request.uri_mut() = format!(
        "{}/voice/v1/?device-id=browser-client&client-id=browser-ui&authorization=Bearer%20{token}",
        base.replace("http", "ws")
    )
    .parse()
    .unwrap();
    let (mut socket, _) = connect_async(browser_request).await.unwrap();
    socket
        .send(Message::Text(r#"{"type":"hello"}"#.into()))
        .await
        .unwrap();
    assert!(matches!(next_message(&mut socket).await, Message::Text(_)));
    task.abort();
}

#[tokio::test]
async fn binary_before_hello_closes_with_protocol_error() {
    let (base, task) = start(1_024).await;
    let (mut socket, _) = connect_async(request(&base)).await.unwrap();
    socket
        .send(Message::Binary(vec![1, 2, 3].into()))
        .await
        .unwrap();
    match next_message(&mut socket).await {
        Message::Close(Some(frame)) => assert_eq!(u16::from(frame.code), 1002),
        other => panic!("expected protocol close, got {other:?}"),
    }
    task.abort();
}

/// A message is capped as a whole, not only per frame: continuation frames that each fit the
/// frame cap must not let a peer make the server buffer an unbounded message before the
/// application-level size check runs.
#[tokio::test]
async fn fragmented_message_beyond_transport_cap_ends_the_connection_before_reassembly() {
    use tokio_tungstenite::tungstenite::protocol::frame::{
        Frame,
        coding::{Data, OpCode},
    };
    let (base, task) = start(1_024).await;
    let (mut socket, _) = connect_async(request(&base)).await.unwrap();
    socket.send(Message::Text(hello().into())).await.unwrap();
    let _ = next_message(&mut socket).await;
    // Each fragment fits the per-frame cap; together they are far beyond it. No final fragment
    // is ever sent, so a server that keeps reassembling never reaches its own size check.
    let mut opcode = OpCode::Data(Data::Text);
    for _ in 0..16 {
        let frame = Frame::message(vec![b'a'; 1_000], opcode, false);
        opcode = OpCode::Data(Data::Continue);
        if socket.send(Message::Frame(frame)).await.is_err() {
            break;
        }
    }
    let ended = timeout(Duration::from_secs(1), async {
        loop {
            match socket.next().await {
                None | Some(Err(_)) | Some(Ok(Message::Close(_))) => return,
                Some(Ok(_)) => continue,
            }
        }
    })
    .await;
    assert!(
        ended.is_ok(),
        "server kept buffering an over-cap fragmented message"
    );
    task.abort();
}

#[tokio::test]
async fn oversized_post_handshake_frame_closes_with_1009() {
    let (base, task) = start(1_024).await;
    let (mut socket, _) = connect_async(request(&base)).await.unwrap();
    socket.send(Message::Text(hello().into())).await.unwrap();
    let _ = next_message(&mut socket).await;
    socket
        .send(Message::Binary(vec![0; 1_025].into()))
        .await
        .unwrap();
    match next_message(&mut socket).await {
        Message::Close(Some(frame)) => assert_eq!(u16::from(frame.code), 1009),
        other => panic!("expected too-large close, got {other:?}"),
    }
    task.abort();
}

/// Web Assistant credential publication must never become a queue that physical Devices wait in:
/// a System save (or a slow Web Assistant admission ahead of one) only concerns the Web
/// Assistant.
#[tokio::test]
async fn physical_device_admission_does_not_wait_on_web_assistant_publication() {
    let (base, task, _, state) = start_with_state(1_024, String::new(), None, |c| c).await;
    let publication = state.hold_web_assistant_publication_for_test().await;
    let connected = timeout(Duration::from_secs(1), async {
        let (mut socket, _) = connect_async(request(&base)).await.unwrap();
        socket.send(Message::Text(hello().into())).await.unwrap();
        next_message(&mut socket).await
    })
    .await;
    drop(publication);
    assert!(
        matches!(connected, Ok(Message::Text(_))),
        "physical Device admission waited on Web Assistant publication"
    );
    task.abort();
}

fn heartbeat(ping_interval_ms: u64, idle_timeout_ms: u64) -> impl FnOnce(AppConfig) -> AppConfig {
    move |mut config| {
        config.websocket.ping_interval_ms = ping_interval_ms;
        config.websocket.idle_timeout_ms = idle_timeout_ms;
        config
    }
}

/// Keepalive Pings are transport control, not the handshake's first application message.
#[tokio::test]
async fn ping_before_client_hello_is_not_a_protocol_error() {
    let (base, task) = start(1_024).await;
    let (mut socket, _) = connect_async(request(&base)).await.unwrap();
    socket
        .send(Message::Ping(b"keepalive".to_vec().into()))
        .await
        .unwrap();
    socket.send(Message::Text(hello().into())).await.unwrap();
    let server_hello = timeout(Duration::from_secs(1), async {
        loop {
            match socket.next().await {
                Some(Ok(Message::Pong(_))) => continue,
                other => return other,
            }
        }
    })
    .await
    .unwrap();
    assert!(
        matches!(server_hello, Some(Ok(Message::Text(_)))),
        "expected ServerHello, got {server_hello:?}"
    );
    task.abort();
}

/// ADR 0020: with no WebSocket frame in either direction for the transport idle window, the
/// Voice Session closes itself (Idle Close) instead of waiting for the operating system to notice
/// a peer that is gone. The server adds no heartbeat by default.
#[tokio::test]
async fn connection_without_transport_activity_is_idle_closed() {
    let (base, task, _, _) = start_with_state(1_024, String::new(), None, heartbeat(0, 300)).await;
    let (mut socket, _) = connect_async(request(&base)).await.unwrap();
    socket.send(Message::Text(hello().into())).await.unwrap();
    let _ = next_message(&mut socket).await;
    tokio::time::sleep(Duration::from_millis(600)).await;
    // The connection ending is the contract; a Close frame, when observed, must carry 1001.
    let closed = timeout(Duration::from_secs(1), async {
        loop {
            match socket.next().await {
                Some(Ok(Message::Close(frame))) => return frame.map(|f| u16::from(f.code)),
                Some(Ok(_)) => continue,
                None | Some(Err(_)) => return None,
            }
        }
    })
    .await
    .expect("silent peer was never closed");
    assert!(
        closed.is_none_or(|code| code == 1001),
        "unexpected close code {closed:?}"
    );
    task.abort();
}

/// Opt-in server Pings, answered by a responsive peer, keep a connection without application
/// traffic open past the idle window.
#[tokio::test]
async fn opt_in_pings_keep_a_responsive_peer_connected() {
    let (base, task, _, _) =
        start_with_state(1_024, String::new(), None, heartbeat(100, 300)).await;
    let (mut socket, _) = connect_async(request(&base)).await.unwrap();
    socket.send(Message::Text(hello().into())).await.unwrap();
    let _ = next_message(&mut socket).await;
    let mut pings = 0;
    let deadline = tokio::time::Instant::now() + Duration::from_millis(1_000);
    while let Ok(message) = tokio::time::timeout_at(deadline, socket.next()).await {
        match message {
            Some(Ok(Message::Ping(_))) => pings += 1,
            Some(Ok(Message::Close(frame))) => panic!("responsive peer was closed: {frame:?}"),
            None | Some(Err(_)) => panic!("responsive peer connection ended"),
            Some(Ok(_)) => {}
        }
    }
    assert!(pings >= 2, "expected periodic server Pings, got {pings}");
    task.abort();
}

/// ADR 0020: transport idle counts frames sent as well as received, so server traffic alone keeps
/// a Voice Session open even while the peer sends nothing.
#[tokio::test]
async fn server_traffic_counts_as_transport_activity() {
    let (base, task, _, _) =
        start_with_state(1_024, String::new(), None, heartbeat(100, 300)).await;
    let (mut socket, _) = connect_async(request(&base)).await.unwrap();
    socket.send(Message::Text(hello().into())).await.unwrap();
    let _ = next_message(&mut socket).await;
    // Not reading: the client sends nothing back, not even Pongs, for twice the idle window.
    tokio::time::sleep(Duration::from_millis(700)).await;
    let mut pings = 0;
    let deadline = tokio::time::Instant::now() + Duration::from_millis(200);
    while let Ok(message) = tokio::time::timeout_at(deadline, socket.next()).await {
        match message {
            Some(Ok(Message::Ping(_))) => pings += 1,
            Some(Ok(Message::Close(frame))) => panic!("server traffic did not count: {frame:?}"),
            None | Some(Err(_)) => panic!("connection ended despite server traffic"),
            Some(Ok(_)) => {}
        }
    }
    assert!(
        pings >= 3,
        "expected the server's Pings while idle-silent, got {pings}"
    );
    task.abort();
}
