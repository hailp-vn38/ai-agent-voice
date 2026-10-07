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
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let config = AppConfig {
        speaker_recognition: Default::default(),
        server: ServerConfig {
            bind: address,
            public_ws_url: Url::parse(&format!("ws://{address}/voice/v1/")).unwrap(),
            hello_timeout_ms: 500,
        },
        auth: AuthConfig { token },
        audio: AudioConfig::default(),
        websocket: WebsocketConfig { max_frame_bytes },
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
        vision: voice_agent_server::config::VisionConfig::default(),
        database: voice_agent_server::config::DatabaseConfig::default(),
        api: voice_agent_server::config::AdminApiConfig::default(),
        shutdown: voice_agent_server::config::ShutdownConfig::default(),
        agent: None,
        effective_agent: voice_agent_server::config::EffectiveAgentConfig::default(),
    };
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
    let app: Router = router_with_state(state);
    let task = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    (format!("http://{address}"), task, lifecycle)
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
