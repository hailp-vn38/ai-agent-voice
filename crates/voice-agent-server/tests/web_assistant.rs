use futures_util::{SinkExt, StreamExt};
use reqwest::{Client, StatusCode};
use std::{fs, sync::Arc, time::Duration};
use voice_agent_server::{
    app::{AppState, router_with_state},
    audio::PcmF32Mono,
    config::AppConfig,
    database::Database,
    providers::{
        AsrError, AsrEvent, AsrProvider, AsrResult, AsrSession, LlmError, LlmProvider, ProviderSet,
        TtsError, TtsProvider,
    },
};

async fn server(_: bool) -> (String, tokio::task::JoinHandle<()>) {
    start(Arc::new(ProviderSet::unavailable()), "").await
}
async fn start(providers: Arc<ProviderSet>, token: &str) -> (String, tokio::task::JoinHandle<()>) {
    start_with_system_mcp(providers, token, false).await
}
async fn start_with_system_mcp(
    providers: Arc<ProviderSet>,
    token: &str,
    enabled: bool,
) -> (String, tokio::task::JoinHandle<()>) {
    let path = std::env::temp_dir().join(format!("assistant-{}.toml", uuid::Uuid::new_v4()));
    let database = std::env::temp_dir().join(format!("assistant-{}.db", uuid::Uuid::new_v4()));
    fs::write(
        &path,
        format!(
            r#"
[server]
bind="127.0.0.1:0"
public_ws_url="ws://127.0.0.1:0/voice/v1/"
[system_mcp]
enabled={enabled}
[auth]
token="{token}"
[provider_defaults]
vad="test"
asr="test"
llm="test"
tts="test"
[database]
url="sqlite://{}"
[api]
enabled=true
admin_token="admin-test-token"
"#,
            database.display()
        ),
    )
    .unwrap();
    let config = AppConfig::parse_and_resolve(&path).unwrap();
    fs::remove_file(path).unwrap();
    let db = Database::connect(&config.database).await.unwrap();
    let state = AppState::from_provider_set_with_database(config, providers, Some(db));
    if enabled {
        let stop = tokio_util::sync::CancellationToken::new();
        let sampler = state.system_metrics.clone().start(stop.clone());
        tokio::time::timeout(Duration::from_secs(5), async {
            while state.system_metrics.latest().is_none() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        stop.cancel();
        sampler.await.unwrap();
    }
    let router = router_with_state(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let task = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    (format!("http://{address}"), task)
}

#[tokio::test]
async fn assistant_settings_require_admin_and_do_not_provision_on_read() {
    let (base, task) = server(true).await;
    let client = Client::new();
    let url = format!("{base}/api/admin/system/assistant-device");
    assert_eq!(
        client.get(&url).send().await.unwrap().status(),
        StatusCode::UNAUTHORIZED
    );
    let response = client
        .get(&url)
        .bearer_auth("admin-test-token")
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body: serde_json::Value = response.json().await.unwrap();
    assert_eq!(body["configured"], false);
    assert_eq!(body["device_id"], serde_json::Value::Null);
    task.abort();
}

#[tokio::test]
async fn assistant_device_is_managed_and_tickets_cannot_bypass_voice_auth() {
    let (base, task) = server(true).await;
    let client = Client::new();
    let auth = "admin-test-token";
    let settings = format!("{base}/api/admin/system/assistant-device");
    let tickets = format!("{base}/api/admin/assistant/ws-ticket");
    assert_eq!(
        client
            .post(&tickets)
            .header("Origin", "http://localhost")
            .bearer_auth(auth)
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::CONFLICT
    );
    assert_eq!(
        client
            .post(format!("{base}/api/admin/agents"))
            .bearer_auth(auth)
            .json(&serde_json::json!({"key":"studio","name":"Studio"}))
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::CREATED
    );
    let input = serde_json::json!({"agent_key":"studio","template_key":null,"enabled":true});
    let response = client
        .put(&settings)
        .bearer_auth(auth)
        .json(&input)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let device: serde_json::Value = response.json().await.unwrap();
    let identity = device["device_id"].as_str().unwrap();
    assert_eq!(
        client
            .put(&settings)
            .bearer_auth(auth)
            .json(&input)
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::CONFLICT
    );
    assert_eq!(
        client
            .patch(format!("{base}/api/admin/devices/{identity}"))
            .bearer_auth(auth)
            .header("If-Match", "\"1\"")
            .json(&serde_json::json!({"enabled":false}))
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::CONFLICT
    );
    assert_eq!(
        client
            .delete(format!("{base}/api/admin/devices/{identity}"))
            .bearer_auth(auth)
            .header("If-Match", "\"1\"")
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::CONFLICT
    );
    assert_eq!(client.post(format!("{base}/api/admin/devices")).bearer_auth(auth).json(&serde_json::json!({"device_id":"spoof","agent_key":"studio","kind":"web_assistant"})).send().await.unwrap().status(), StatusCode::BAD_REQUEST);
    assert_eq!(
        client
            .post(&tickets)
            .bearer_auth(auth)
            .header("Origin", "https://evil.example")
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::FORBIDDEN
    );
    let ticket_response = client
        .post(&tickets)
        .bearer_auth(auth)
        .header("Origin", "http://localhost")
        .send()
        .await
        .unwrap();
    assert_eq!(ticket_response.status(), StatusCode::OK);
    assert_eq!(ticket_response.headers()["cache-control"], "no-store");
    let lease: serde_json::Value = ticket_response.json().await.unwrap();
    let ws_base = base.replacen("http", "ws", 1);
    // This fixture has Voice auth disabled: identity alone must still never grant web access.
    let url = format!("{ws_base}/voice/v1/?device-id={identity}&client-id=web_client");
    let denied = tokio_tungstenite::connect_async(&url).await.unwrap_err();
    assert!(matches!(denied, tokio_tungstenite::tungstenite::Error::Http(r) if r.status()==401));
    let token = lease["ticket"].as_str().unwrap();
    let cid = lease["client_id"].as_str().unwrap();
    let url = format!("{ws_base}/voice/v1/?device-id=unknown&client-id={cid}&ticket={token}");
    let denied = tokio_tungstenite::connect_async(&url).await.unwrap_err();
    assert!(matches!(denied, tokio_tungstenite::tungstenite::Error::Http(r) if r.status()==401));
    use tokio_tungstenite::tungstenite::client::IntoClientRequest;
    let url = format!("{ws_base}/voice/v1/?device-id={identity}&client-id={cid}&ticket={token}");
    let mut request = url.into_client_request().unwrap();
    request
        .headers_mut()
        .insert("Origin", "http://localhost".parse().unwrap());
    // The fixture exposes unavailable test providers through the existing runtime catalog.
    let (mut socket, response) = tokio_tungstenite::connect_async(request.clone())
        .await
        .unwrap();
    assert_eq!(response.status(), 101);
    use futures_util::{SinkExt, StreamExt};
    socket
        .send(tokio_tungstenite::tungstenite::Message::Text(
            serde_json::json!({"type":"hello","features":{"turn_status":true,"mcp":false}})
                .to_string()
                .into(),
        ))
        .await
        .unwrap();
    let hello = tokio::time::timeout(Duration::from_secs(2), socket.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let hello: serde_json::Value = serde_json::from_str(hello.to_text().unwrap()).unwrap();
    assert_eq!(hello["type"], "hello");
    socket
        .send(tokio_tungstenite::tungstenite::Message::Text(
            serde_json::json!({"type":"listen","state":"start","mode":"manual"})
                .to_string()
                .into(),
        ))
        .await
        .unwrap();
    let status = tokio::time::timeout(Duration::from_secs(2), socket.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let status: serde_json::Value = serde_json::from_str(status.to_text().unwrap()).unwrap();
    assert_eq!(status["type"], "turn_status");
    assert_eq!(status["state"], "listening_ready");
    socket
        .send(tokio_tungstenite::tungstenite::Message::Text(
            serde_json::json!({"type":"listen","state":"detect","text":"Xin chào"})
                .to_string()
                .into(),
        ))
        .await
        .unwrap();
    let mut stt_count = 0;
    loop {
        let message = tokio::time::timeout(Duration::from_secs(2), socket.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        let event: serde_json::Value = serde_json::from_str(message.to_text().unwrap()).unwrap();
        if event["type"] == "stt" {
            stt_count += 1;
            assert_eq!(event["text"], "Xin chào");
        }
        if event["type"] == "turn_status" {
            assert_eq!(event["state"], "failed");
            break;
        }
    }
    assert_eq!(stt_count, 1);
    let replay = tokio_tungstenite::connect_async(request).await.unwrap_err();
    assert!(matches!(replay, tokio_tungstenite::tungstenite::Error::Http(r) if r.status()==401));
    assert_eq!(
        client
            .patch(format!("{base}/api/admin/agents/studio"))
            .bearer_auth(auth)
            .header("If-Match", "\"1\"")
            .json(&serde_json::json!({"enabled":false}))
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::OK
    );
    let disabled = serde_json::json!({"agent_key":"studio","template_key":null,"enabled":false});
    assert_eq!(
        client
            .put(&settings)
            .bearer_auth(auth)
            .header("If-Match", "\"1\"")
            .json(&disabled)
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::OK
    );
    assert_eq!(
        client
            .post(&tickets)
            .bearer_auth(auth)
            .header("Origin", "http://localhost")
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::CONFLICT
    );
    let close = tokio::time::timeout(Duration::from_secs(2), socket.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert!(
        matches!(close, tokio_tungstenite::tungstenite::Message::Close(Some(frame)) if u16::from(frame.code)==1008)
    );
    task.abort();
}

struct VoiceAsr;
struct VoiceAsrSession;
impl AsrProvider for VoiceAsr {
    fn open(&self) -> Result<Box<dyn AsrSession>, AsrError> {
        Ok(Box::new(VoiceAsrSession))
    }
}
impl AsrSession for VoiceAsrSession {
    fn push_pcm(&mut self, _: &PcmF32Mono) -> Result<Vec<AsrEvent>, AsrError> {
        Ok(vec![])
    }
    fn finish(&mut self) -> Result<AsrResult, AsrError> {
        Ok(AsrResult::new("Mic input"))
    }
    fn cancel(&mut self) {}
}
struct VoiceLlm;
impl LlmProvider for VoiceLlm {
    fn adapter(&self) -> &'static str {
        "test-llm"
    }
    fn complete(
        &self,
        _: &voice_agent_server::providers::llm::LlmRequest,
    ) -> Result<String, LlmError> {
        Ok("Xin chào. Tôi có thể hỗ trợ bạn.".into())
    }
}
struct VoiceTts;
impl TtsProvider for VoiceTts {
    fn adapter(&self) -> &'static str {
        "test-tts"
    }
    fn synthesize(&self, _: &str) -> Result<PcmF32Mono, TtsError> {
        Ok(PcmF32Mono::new(vec![0.1; 5760], 48000))
    }
}

#[tokio::test]
async fn web_assistant_shared_pipeline_completes_text_and_mic_and_caps_connections() {
    let providers = Arc::new(ProviderSet::with_all(
        Arc::new(VoiceVad),
        Arc::new(VoiceAsr),
        Arc::new(VoiceLlm),
        Arc::new(VoiceTts),
    ));
    let (base, task) = start(providers, "physical-token").await;
    let client = Client::new();
    let auth = "admin-test-token";
    client
        .post(format!("{base}/api/admin/agents"))
        .bearer_auth(auth)
        .json(&serde_json::json!({"key":"studio","name":"Studio"}))
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap();
    let settings = client
        .put(format!("{base}/api/admin/system/assistant-device"))
        .bearer_auth(auth)
        .json(&serde_json::json!({"agent_key":"studio","enabled":true}))
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json::<serde_json::Value>()
        .await
        .unwrap();
    let identity = settings["device_id"].as_str().unwrap();
    let ws = base.replacen("http", "ws", 1);
    let forbidden = tokio_tungstenite::connect_async(format!(
        "{ws}/voice/v1/?device-id={identity}&client-id=client&authorization=Bearer%20physical-token"
    ))
    .await
    .unwrap_err();
    assert!(
        matches!(forbidden, tokio_tungstenite::tungstenite::Error::Http(r) if r.status()==400 || r.status()==401)
    );
    use tokio_tungstenite::tungstenite::client::IntoClientRequest;
    let mut sockets = Vec::new();
    for index in 0..3 {
        let ticket = client
            .post(format!("{base}/api/admin/assistant/ws-ticket"))
            .bearer_auth(auth)
            .header("Origin", "http://localhost")
            .send()
            .await
            .unwrap()
            .error_for_status()
            .unwrap()
            .json::<serde_json::Value>()
            .await
            .unwrap();
        let url = format!(
            "{ws}/voice/v1/?device-id={identity}&client-id={}&ticket={}",
            ticket["client_id"].as_str().unwrap(),
            ticket["ticket"].as_str().unwrap()
        );
        let mut request = url.into_client_request().unwrap();
        request
            .headers_mut()
            .insert("Origin", "http://localhost".parse().unwrap());
        let connected = tokio_tungstenite::connect_async(request).await;
        if index == 2 {
            assert!(
                matches!(connected.unwrap_err(), tokio_tungstenite::tungstenite::Error::Http(r) if r.status()==429)
            );
        } else {
            sockets.push(connected.unwrap().0);
        }
    }
    let socket = &mut sockets[0];
    send(
        socket,
        serde_json::json!({"type":"hello","features":{"turn_status":true,"mcp":false}}),
    )
    .await;
    assert_eq!(next_json(socket).await["type"], "hello");
    for microphone in [false, true] {
        send(
            socket,
            serde_json::json!({"type":"listen","state":"start","mode":"manual"}),
        )
        .await;
        assert_eq!(next_json(socket).await["state"], "listening_ready");
        if microphone {
            // A canonical 16 kHz, 60 ms raw Opus packet; admission and ASR remain shared.
            let mut encoder =
                opus2::Encoder::new(16000, opus2::Channels::Mono, opus2::Application::Voip)
                    .unwrap();
            let mut packet = vec![0; 4096];
            let count = encoder.encode(&vec![0i16; 960], &mut packet).unwrap();
            packet.truncate(count);
            socket
                .send(tokio_tungstenite::tungstenite::Message::Binary(
                    packet.into(),
                ))
                .await
                .unwrap();
            send(socket, serde_json::json!({"type":"listen","state":"stop"})).await;
        } else {
            send(
                socket,
                serde_json::json!({"type":"listen","state":"detect","text":"Xin chào"}),
            )
            .await;
        }
        let mut accepted = 0;
        let mut llm = String::new();
        let mut audio = 0;
        let mut stopped = false;
        loop {
            let message = tokio::time::timeout(Duration::from_secs(5), socket.next())
                .await
                .unwrap()
                .unwrap()
                .unwrap();
            if message.is_binary() {
                audio += 1;
                continue;
            }
            let event: serde_json::Value =
                serde_json::from_str(message.to_text().unwrap()).unwrap();
            match event["type"].as_str().unwrap() {
                "stt" => {
                    accepted += 1;
                    assert_eq!(
                        event["text"],
                        if microphone { "Mic input" } else { "Xin chào" }
                    );
                }
                "llm" => llm.push_str(event["text"].as_str().unwrap()),
                "tts" if event["state"] == "stop" => stopped = true,
                "turn_status" => {
                    assert_eq!(event["state"], "completed");
                    assert!(stopped, "terminal follows writer stop");
                    break;
                }
                _ => {}
            }
        }
        assert_eq!(accepted, 1);
        assert!(llm.contains("Xin chào"));
        assert!(audio > 0);
    }
    // Rebind closes even a second socket that is still waiting for its hello.
    client
        .put(format!("{base}/api/admin/system/assistant-device"))
        .bearer_auth(auth)
        .header("If-Match", "\"1\"")
        .json(&serde_json::json!({"agent_key":"studio","enabled":false}))
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap();
    let close = tokio::time::timeout(Duration::from_secs(2), sockets[0].next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert!(
        matches!(close, tokio_tungstenite::tungstenite::Message::Close(Some(frame)) if u16::from(frame.code)==1008)
    );
    task.abort();
}

type Socket =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;
async fn send(socket: &mut Socket, message: serde_json::Value) {
    socket
        .send(tokio_tungstenite::tungstenite::Message::Text(
            message.to_string().into(),
        ))
        .await
        .unwrap();
}
async fn next_json(socket: &mut Socket) -> serde_json::Value {
    let message = tokio::time::timeout(Duration::from_secs(2), socket.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    serde_json::from_str(message.to_text().unwrap()).unwrap()
}

struct VoiceVad;
impl voice_agent_server::providers::VadProvider for VoiceVad {
    fn adapter(&self) -> &'static str {
        "test-vad"
    }
    fn open(
        &self,
    ) -> Result<
        Box<dyn voice_agent_server::providers::VadSession>,
        voice_agent_server::providers::VadError,
    > {
        Err(voice_agent_server::providers::VadError::Failed(
            "Manual does not open VAD".into(),
        ))
    }
}

#[path = "web_assistant/system_mcp.rs"]
mod system_mcp;

/// A capacity refusal is not a use of the ticket: the same one-time ticket admits the connection
/// once a slot frees, while every other refusal still burns it.
#[tokio::test]
async fn capacity_refusal_does_not_consume_the_web_assistant_ticket() {
    use tokio_tungstenite::tungstenite::client::IntoClientRequest;
    let providers = Arc::new(ProviderSet::with_all(
        Arc::new(VoiceVad),
        Arc::new(VoiceAsr),
        Arc::new(VoiceLlm),
        Arc::new(VoiceTts),
    ));
    let (base, task) = start(providers, "physical-token").await;
    let client = Client::new();
    let auth = "admin-test-token";
    client
        .post(format!("{base}/api/admin/agents"))
        .bearer_auth(auth)
        .json(&serde_json::json!({"key":"studio","name":"Studio"}))
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap();
    let settings = client
        .put(format!("{base}/api/admin/system/assistant-device"))
        .bearer_auth(auth)
        .json(&serde_json::json!({"agent_key":"studio","enabled":true}))
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json::<serde_json::Value>()
        .await
        .unwrap();
    let identity = settings["device_id"].as_str().unwrap().to_owned();
    let ws = base.replacen("http", "ws", 1);
    let ticket_request = || async {
        let ticket = client
            .post(format!("{base}/api/admin/assistant/ws-ticket"))
            .bearer_auth(auth)
            .header("Origin", "http://localhost")
            .send()
            .await
            .unwrap()
            .error_for_status()
            .unwrap()
            .json::<serde_json::Value>()
            .await
            .unwrap();
        let mut request = format!(
            "{ws}/voice/v1/?device-id={identity}&client-id={}&ticket={}",
            ticket["client_id"].as_str().unwrap(),
            ticket["ticket"].as_str().unwrap()
        )
        .into_client_request()
        .unwrap();
        request
            .headers_mut()
            .insert("Origin", "http://localhost".parse().unwrap());
        request
    };
    let mut sockets = Vec::new();
    for _ in 0..2 {
        sockets.push(
            tokio_tungstenite::connect_async(ticket_request().await)
                .await
                .unwrap()
                .0,
        );
    }
    let waiting = ticket_request().await;
    let refused = tokio_tungstenite::connect_async(waiting.clone()).await;
    assert!(
        matches!(refused.unwrap_err(), tokio_tungstenite::tungstenite::Error::Http(r) if r.status() == 429)
    );
    let mut freed = sockets.pop().unwrap();
    let _ = freed.close(None).await;
    drop(freed);
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(2);
    loop {
        match tokio_tungstenite::connect_async(waiting.clone()).await {
            Ok(_) => break,
            Err(tokio_tungstenite::tungstenite::Error::Http(r))
                if r.status() == 429 && tokio::time::Instant::now() < deadline =>
            {
                tokio::time::sleep(std::time::Duration::from_millis(25)).await;
            }
            Err(error) => panic!("the ticket refused for capacity no longer admits: {error:?}"),
        }
    }
    task.abort();
}
