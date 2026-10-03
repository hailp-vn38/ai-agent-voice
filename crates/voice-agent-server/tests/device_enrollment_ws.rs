use futures_util::{SinkExt, StreamExt};
use std::{path::PathBuf, sync::Arc};
use tokio::{net::TcpListener, task::JoinHandle, time::{Duration, timeout}};
use tokio_tungstenite::{connect_async, tungstenite::{Message, client::IntoClientRequest}};
use voice_agent_server::{
    app::{AppState, bootstrap_with_providers, new_lifecycle, router_with_state},
    config::*,
    database::Database,
    providers::ProviderSet,
    services::device_enrollment::EnrollmentRuntime,
};

type Socket = tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

struct Server {
    base: String,
    database: Database,
    state: AppState,
    task: JoinHandle<()>,
    directory: PathBuf,
}

impl Drop for Server {
    fn drop(&mut self) {
        self.state.lifecycle.stopping().cancel();
        self.task.abort();
        let _ = std::fs::remove_dir_all(&self.directory);
    }
}

async fn start() -> Server {
    let directory = std::env::temp_dir().join(format!("enrollment-ws-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir(&directory).unwrap();
    for name in std::iter::once("intro".to_string()).chain((0..10).map(|n| n.to_string())) {
        let mut writer = hound::WavWriter::create(directory.join(format!("{name}.wav")), hound::WavSpec {
            channels: 1, sample_rate: 24_000, bits_per_sample: 16, sample_format: hound::SampleFormat::Int,
        }).unwrap();
        for _ in 0..2_400 { writer.write_sample(1_000_i16).unwrap(); }
        writer.finalize().unwrap();
    }
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let config = AppConfig {
        server: ServerConfig { bind: address, public_ws_url: url::Url::parse(&format!("ws://{address}/voice/v1/")).unwrap(), hello_timeout_ms: 500 },
        auth: AuthConfig { token: "voice-transport".into() },
        audio: Default::default(), websocket: Default::default(), limits: Default::default(),
        provider_defaults: ProviderDefaultsConfig { vad:"test".into(), asr:"test".into(), llm:"test".into(), tts:"test".into(), vision:None },
        providers: Default::default(), workers: Default::default(), deployment: Default::default(), runtime: Default::default(),
        provider_runtime: None, llm: Default::default(), tts: Default::default(), speech_output: Default::default(),
        barge_in: Default::default(), mcp: Default::default(), vision: Default::default(),
        database: DatabaseConfig {
            url: format!("sqlite://{}", directory.join("database.db").display()),
            devices: DatabaseDevicesConfig {
                enrollment: EnrollmentConfig { enabled:true, ws_max_connections:1, ws_poll_interval_ms:1_000, prompt_assets_dir:directory.clone(), ..Default::default() },
                ..Default::default()
            }, ..Default::default()
        },
        api: AdminApiConfig { enabled:true, admin_token:"enrollment-admin".into(), ..Default::default() },
        shutdown: Default::default(), agent:None, effective_agent:Default::default(),
    };
    let database = Database::connect(&config.database).await.unwrap();
    sqlx::query("INSERT INTO agents (key,name,enabled,created_at,updated_at) VALUES ('agent','Agent',1,1,1)")
        .execute(database.pool()).await.unwrap();
    let lifecycle = new_lifecycle(&config);
    let runtime = EnrollmentRuntime::prepare(&config.database.devices.enrollment).await.unwrap();
    let mut state = AppState::from_provider_set_with_database_and_shutdown(config, Arc::new(ProviderSet::unavailable()), Some(database.clone()), lifecycle);
    state.enrollment_runtime = runtime;
    let router = router_with_state(state.clone());
    let task = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    Server { base:format!("http://{address}"), database, state, task, directory }
}

fn request(server: &Server, device: &str, token: Option<&str>) -> tokio_tungstenite::tungstenite::handshake::client::Request {
    let mut request = format!("{}/voice/v1/", server.base.replace("http", "ws")).into_client_request().unwrap();
    request.headers_mut().insert("Device-Id", device.parse().unwrap());
    request.headers_mut().insert("Client-Id", "enrollment-test".parse().unwrap());
    request.headers_mut().insert("Protocol-Version", "1".parse().unwrap());
    if let Some(token) = token { request.headers_mut().insert("Authorization", format!("Bearer {token}").parse().unwrap()); }
    request
}

async fn next(socket: &mut Socket) -> Message {
    timeout(Duration::from_secs(4), socket.next()).await.unwrap().unwrap().unwrap()
}

async fn connect(server: &Server, device: &str) -> Socket {
    let (mut socket, response) = connect_async(request(server, device, Some("voice-transport"))).await.unwrap();
    assert_eq!(response.status(), 101);
    socket.send(Message::Text(r#"{"type":"hello","version":1,"features":{"mcp":true}}"#.into())).await.unwrap();
    let hello: serde_json::Value = serde_json::from_str(next(&mut socket).await.to_text().unwrap()).unwrap();
    assert_eq!(hello["type"], "hello");
    assert_eq!(hello["audio_params"]["sample_rate"], 24_000);
    socket
}

async fn seed_code(server: &Server, device: &str, code: &str) {
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs() as i64;
    sqlx::query("INSERT INTO device_enrollments(device_id,client_id,code,challenge,metadata_json,created_at,expires_at) VALUES (?, 'test', ?, 'challenge', '{}', ?, ?)")
        .bind(device).bind(code).bind(now).bind(now+600).execute(server.database.pool()).await.unwrap();
}

async fn read_prompt(socket: &mut Socket, code: &str) {
    let mut decoder = opus2::Decoder::new(24_000, opus2::Channels::Mono).unwrap();
    let mut started = false;
    let mut displayed = false;
    let mut count = 0;
    loop {
        match next(socket).await {
            Message::Text(text) => {
                let message: serde_json::Value = serde_json::from_str(&text).unwrap();
                match (message["type"].as_str(), message["state"].as_str()) {
                    (Some("stt"), _) => { assert!(message["text"].as_str().unwrap().contains(code)); displayed=true; }
                    (Some("tts"), Some("start")) => { assert!(displayed); started=true; }
                    (Some("tts"), Some("sentence_start")) => assert!(started),
                    (Some("tts"), Some("stop")) => { assert!(started && count>0); break; }
                    _ => panic!("unexpected onboarding message"),
                }
            }
            Message::Binary(bytes) => {
                assert!(started);
                let mut pcm = [0_i16; 2_880];
                assert_eq!(decoder.decode(&bytes, &mut pcm, false).unwrap(), 1_440);
                count+=1;
            }
            _ => panic!("unexpected onboarding frame"),
        }
    }
    assert!(count<=250);
}

fn rejected(error: tokio_tungstenite::tungstenite::Error) -> u16 {
    match error { tokio_tungstenite::tungstenite::Error::Http(response)=>response.status().as_u16(), other=>panic!("{other:?}") }
}

#[tokio::test]
async fn ota_ws_prompt_claim_and_reconnect_preserve_database_admission() {
    let server = start().await;
    seed_code(&server, "unknown", "042731").await;
    let client = reqwest::Client::new();
    let ota: serde_json::Value = client.get(format!("{}/voice/ota/",server.base)).header("Device-Id","unknown").header("Client-Id","test")
        .send().await.unwrap().json().await.unwrap();
    assert_eq!(ota["websocket"]["token"], "voice-transport");
    assert!(ota.get("activation").is_none());
    let mut socket = connect(&server,"unknown").await;
    read_prompt(&mut socket,"042731").await;
    socket.send(Message::Text(r#"{"type":"listen","state":"start","mode":"manual"}"#.into())).await.unwrap();
    socket.send(Message::Binary(vec![0_u8;128].into())).await.unwrap();
    socket.send(Message::Text(r#"{"type":"mcp","payload":{"jsonrpc":"2.0","id":1,"method":"tools/list"}}"#.into())).await.unwrap();
    let devices: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM devices").fetch_one(server.database.pool()).await.unwrap();
    assert_eq!(devices,0);
    let claimed = client.post(format!("{}/api/admin/device-enrollments/claim",server.base)).bearer_auth("enrollment-admin")
        .json(&serde_json::json!({"code":"042731","agent_key":"agent"})).send().await.unwrap();
    assert_eq!(claimed.status(),201);
    let message:serde_json::Value = serde_json::from_str(next(&mut socket).await.to_text().unwrap()).unwrap();
    assert!(message["text"].as_str().unwrap().contains("Đã liên kết"));
    assert!(matches!(next(&mut socket).await, Message::Close(Some(frame)) if u16::from(frame.code)==1000));
    // An admitted Device follows the actual voice path with a fresh profile, not EnrollmentSession.
    let mut voice = connect(&server,"unknown").await;
    voice.close(None).await.unwrap();
    let sessions:i64 = sqlx::query_scalar("SELECT COUNT(*) FROM sessions").fetch_one(server.database.pool()).await.unwrap();
    assert_eq!(sessions,0,"onboarding has no Persistent Transcript");
}

#[tokio::test]
async fn auth_capacity_disabled_and_expiry_fail_closed() {
    let server=start().await;
    assert_eq!(rejected(connect_async(request(&server,"unknown",None)).await.unwrap_err()),401);
    assert_eq!(rejected(connect_async(request(&server,"unknown",Some("wrong"))).await.unwrap_err()),401);
    let rows:i64=sqlx::query_scalar("SELECT COUNT(*) FROM device_enrollments").fetch_one(server.database.pool()).await.unwrap();
    assert_eq!(rows,0);
    seed_code(&server,"unknown","000001").await;
    let mut socket=connect(&server,"unknown").await;
    read_prompt(&mut socket,"000001").await;
    assert_eq!(rejected(connect_async(request(&server,"other",Some("voice-transport"))).await.unwrap_err()),503);
    sqlx::query("UPDATE device_enrollments SET expires_at=created_at-1,created_at=created_at-10 WHERE device_id='unknown'")
        .execute(server.database.pool()).await.unwrap();
    let message:serde_json::Value=serde_json::from_str(next(&mut socket).await.to_text().unwrap()).unwrap();
    assert!(message["text"].as_str().unwrap().contains("hết hạn"));
    assert!(matches!(next(&mut socket).await,Message::Close(Some(frame)) if u16::from(frame.code)==1000));
    sqlx::query("INSERT INTO devices(device_id,agent_id,enabled,created_at,updated_at) SELECT 'blocked',id,0,1,1 FROM agents WHERE key='agent'")
        .execute(server.database.pool()).await.unwrap();
    assert_eq!(rejected(connect_async(request(&server,"blocked",Some("voice-transport"))).await.unwrap_err()),403);
}

#[tokio::test]
async fn abort_cancels_audio_without_consuming_enrollment_and_replay_has_cooldown() {
    let server=start().await;
    seed_code(&server,"unknown","000001").await;
    let mut socket=connect(&server,"unknown").await;
    let mut started=false;
    while !started {
        if let Message::Text(text)=next(&mut socket).await {
            let value:serde_json::Value=serde_json::from_str(&text).unwrap();
            started=value["state"]=="start";
        }
    }
    socket.send(Message::Text(r#"{"type":"abort"}"#.into())).await.unwrap();
    loop {
        if let Message::Text(text)=next(&mut socket).await {
            let value:serde_json::Value=serde_json::from_str(&text).unwrap();
            if value["state"]=="stop" {break;}
        }
    }
    socket.send(Message::Text(r#"{"type":"listen","state":"start","mode":"manual"}"#.into())).await.unwrap();
    assert!(timeout(Duration::from_millis(150),socket.next()).await.is_err());
    let status:String=sqlx::query_scalar("SELECT status FROM device_enrollments WHERE code='000001'").fetch_one(server.database.pool()).await.unwrap();
    assert_eq!(status,"pending");
}

#[tokio::test]
async fn missing_assets_fail_bootstrap_before_router_publication() {
    let server=start().await;
    let mut config=server.state.config.as_ref().clone();
    config.database.url=format!("sqlite://{}",server.directory.join("second.db").display());
    config.database.devices.enrollment.prompt_assets_dir=server.directory.join("missing-assets");
    let result=bootstrap_with_providers(config,Arc::new(ProviderSet::unavailable())).await;
    assert!(matches!(result,Err(voice_agent_server::app::BootstrapError::Enrollment)));
}
