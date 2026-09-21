use axum::Router;
use futures_util::{SinkExt, StreamExt};
use tokio::{
    net::TcpListener,
    task::JoinHandle,
    time::{timeout, Duration},
};
use tokio_tungstenite::{
    connect_async,
    tungstenite::{client::IntoClientRequest, Message},
};
use url::Url;
use voice_agent_server::{
    app::router,
    config::{AppConfig, AudioConfig, AuthConfig, LimitsConfig, ServerConfig},
};

async fn start(max_frame_bytes: usize) -> (String, JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let config = AppConfig {
        server: ServerConfig {
            bind: address,
            public_ws_url: Url::parse(&format!("ws://{address}/voice/v1/")).unwrap(),
            hello_timeout_ms: 500,
        },
        auth: AuthConfig::default(),
        audio: AudioConfig {
            max_ws_frame_bytes: max_frame_bytes,
            ..AudioConfig::default()
        },
        limits: LimitsConfig::default(),
    };
    let app: Router = router(config);
    let task = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    (format!("http://{address}"), task)
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
async fn valid_hello_receives_canonical_server_hello() {
    let (base, task) = start(1_024).await;
    let (mut socket, _) = connect_async(request(&base)).await.unwrap();
    socket.send(Message::Text(hello())).await.unwrap();
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
async fn binary_before_hello_closes_with_protocol_error() {
    let (base, task) = start(1_024).await;
    let (mut socket, _) = connect_async(request(&base)).await.unwrap();
    socket.send(Message::Binary(vec![1, 2, 3])).await.unwrap();
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
    socket.send(Message::Text(hello())).await.unwrap();
    let _ = next_message(&mut socket).await;
    socket.send(Message::Binary(vec![0; 1_025])).await.unwrap();
    match next_message(&mut socket).await {
        Message::Close(Some(frame)) => assert_eq!(u16::from(frame.code), 1009),
        other => panic!("expected too-large close, got {other:?}"),
    }
    task.abort();
}
