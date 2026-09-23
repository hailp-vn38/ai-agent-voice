use std::{sync::Arc, time::Duration};

use axum::Router;
use futures_util::{SinkExt, StreamExt};
use opus2::{Application, Channels, Encoder};
use tokio::{net::TcpListener, task::JoinHandle, time::timeout};
use tokio_tungstenite::{
    connect_async,
    tungstenite::{Message, client::IntoClientRequest},
};
use url::Url;
use voice_agent_server::{
    app::router_with_providers,
    config::{
        AppConfig, AudioConfig, AuthConfig, BargeInConfig, DeploymentConfig, LimitsConfig,
        LlmConfig, ProvidersConfig, RuntimeConfig, ServerConfig, SpeechOutputConfig, TtsConfig,
        WebsocketConfig, WorkersConfig,
    },
    providers::{
        AsrError, AsrEvent, AsrProvider, AsrResult, AsrSession, ProviderSet, VadError, VadInput,
        VadProbability, VadProvider, VadSession,
    },
};

#[derive(Clone, Copy)]
enum AsrOutcome {
    Final,
    Fail,
    SlowFinal,
}

struct DeterministicAsr(AsrOutcome);

impl AsrProvider for DeterministicAsr {
    fn open(&self) -> Result<Box<dyn AsrSession>, AsrError> {
        Ok(Box::new(DeterministicAsrSession(self.0)))
    }
}

struct DeterministicAsrSession(AsrOutcome);

impl AsrSession for DeterministicAsrSession {
    fn push_pcm(
        &mut self,
        _: &voice_agent_server::audio::PcmF32Mono,
    ) -> Result<Vec<AsrEvent>, AsrError> {
        // The protocol test proves this internal partial cannot cross the WebSocket seam.
        Ok(vec![AsrEvent::Partial("internal partial".into())])
    }

    fn finish(&mut self) -> Result<AsrResult, AsrError> {
        match self.0 {
            AsrOutcome::Final => Ok(AsrResult::new("deterministic final")),
            AsrOutcome::Fail => Err(AsrError::Failed("deterministic failure".into())),
            AsrOutcome::SlowFinal => {
                std::thread::sleep(Duration::from_millis(100));
                Ok(AsrResult::new("stale final"))
            }
        }
    }

    fn cancel(&mut self) {}
}

struct SpeechThenSilenceVad;

impl VadProvider for SpeechThenSilenceVad {
    fn open(&self) -> Result<Box<dyn VadSession>, VadError> {
        Ok(Box::new(SpeechThenSilenceVadSession { frames: 0 }))
    }

    fn adapter(&self) -> &'static str {
        "deterministic-vad"
    }
}

struct SpeechThenSilenceVadSession {
    frames: usize,
}

impl VadSession for SpeechThenSilenceVadSession {
    fn push(&mut self, input: VadInput) -> Result<VadProbability, VadError> {
        self.frames += 1;
        Ok(VadProbability {
            start_sample: input.start_sample,
            end_sample: input.start_sample + 512,
            probability: if self.frames == 1 { 1.0 } else { 0.0 },
        })
    }

    fn reset(&mut self) -> Result<(), VadError> {
        self.frames = 0;
        Ok(())
    }
}

async fn start(outcome: AsrOutcome) -> (String, JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let mut providers_config = ProvidersConfig::default();
    let vad = providers_config.vad.silero_onnx.as_mut().unwrap();
    vad.min_speech_ms = 32;
    vad.end_silence_ms = 32;
    let config = AppConfig {
        server: ServerConfig {
            bind: address,
            public_ws_url: Url::parse(&format!("ws://{address}/voice/v1/")).unwrap(),
            hello_timeout_ms: 500,
        },
        auth: AuthConfig::default(),
        audio: AudioConfig::default(),
        websocket: WebsocketConfig::default(),
        limits: LimitsConfig::default(),
        providers: providers_config,
        workers: WorkersConfig::default(),
        deployment: DeploymentConfig::default(),
        runtime: RuntimeConfig::default(),
        llm: LlmConfig::default(),
        tts: TtsConfig::default(),
        speech_output: SpeechOutputConfig::default(),
        barge_in: BargeInConfig::default(),
    };
    let providers = Arc::new(ProviderSet::with_vad(
        Arc::new(SpeechThenSilenceVad),
        Arc::new(DeterministicAsr(outcome)),
    ));
    let app: Router = router_with_providers(config, providers);
    let task = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (format!("http://{address}"), task)
}

fn request(base: &str) -> tokio_tungstenite::tungstenite::handshake::client::Request {
    let mut request = format!("{}/voice/v1/", base.replacen("http", "ws", 1))
        .into_client_request()
        .unwrap();
    let headers = request.headers_mut();
    headers.insert("Protocol-Version", "1".parse().unwrap());
    headers.insert("Device-Id", "reference-client-01".parse().unwrap());
    headers.insert("Client-Id", "protocol-e2e".parse().unwrap());
    request
}

async fn connect(
    base: &str,
) -> tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>> {
    let (mut socket, _) = connect_async(request(base)).await.unwrap();
    socket
        .send(Message::Text(serde_json::json!({
            "type": "hello", "version": 1, "transport": "websocket",
            "audio_params": {"format": "opus", "sample_rate": 16000, "channels": 1, "frame_duration": 60}
        }).to_string().into()))
        .await
        .unwrap();
    let hello = timeout(Duration::from_secs(1), socket.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert!(matches!(hello, Message::Text(_)));
    socket
}

fn canonical_opus_packet() -> Vec<u8> {
    let mut encoder = Encoder::new(16_000, Channels::Mono, Application::Voip).unwrap();
    let mut packet = [0; 4_000];
    let bytes = encoder.encode(&[1_000; 960], &mut packet).unwrap();
    packet[..bytes].to_vec()
}

async fn collect_until_quiet(
    socket: &mut tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
) -> Vec<serde_json::Value> {
    let mut messages = Vec::new();
    loop {
        match timeout(Duration::from_millis(250), socket.next()).await {
            Ok(Some(Ok(Message::Text(text)))) => {
                messages.push(serde_json::from_str(&text).unwrap())
            }
            Ok(Some(Ok(Message::Close(frame)))) => panic!("unexpected close: {frame:?}"),
            Ok(Some(Ok(_))) => {}
            Ok(Some(Err(error))) => panic!("WebSocket error: {error}"),
            Ok(None) | Err(_) => return messages,
        }
    }
}

fn assert_one_v1_final_without_internal_events(messages: &[serde_json::Value]) {
    let stt = messages
        .iter()
        .filter(|message| message["type"] == "stt")
        .collect::<Vec<_>>();
    assert_eq!(stt.len(), 1, "messages: {messages:?}");
    assert_eq!(stt[0]["text"], "deterministic final");
    for message in messages {
        let encoded = message.to_string();
        assert!(!encoded.contains("partial"), "messages: {messages:?}");
        assert!(!encoded.contains("vad_"), "messages: {messages:?}");
    }
}

#[tokio::test]
async fn manual_and_auto_canonical_opus_emit_one_existing_stt_without_internal_events() {
    for mode in ["manual", "auto", "realtime"] {
        let (base, task) = start(AsrOutcome::Final).await;
        let mut socket = connect(&base).await;
        socket
            .send(Message::Text(
                serde_json::json!({"type": "listen", "state": "start", "mode": mode})
                    .to_string()
                    .into(),
            ))
            .await
            .unwrap();
        socket
            .send(Message::Binary(canonical_opus_packet().into()))
            .await
            .unwrap();
        if mode == "manual" {
            socket
                .send(Message::Text(
                    serde_json::json!({"type": "listen", "state": "stop"})
                        .to_string()
                        .into(),
                ))
                .await
                .unwrap();
        } else {
            // The first 512-sample VAD chunk starts speech; the next chunk endpoints it.
            socket
                .send(Message::Binary(canonical_opus_packet().into()))
                .await
                .unwrap();
        }
        let messages = collect_until_quiet(&mut socket).await;
        assert_one_v1_final_without_internal_events(&messages);
        task.abort();
    }
}

#[tokio::test]
async fn failed_and_cancelled_recognition_emit_no_stt() {
    for (outcome, cancel) in [(AsrOutcome::Fail, false), (AsrOutcome::SlowFinal, true)] {
        let (base, task) = start(outcome).await;
        let mut socket = connect(&base).await;
        socket
            .send(Message::Text(
                serde_json::json!({"type": "listen", "state": "start", "mode": "manual"})
                    .to_string()
                    .into(),
            ))
            .await
            .unwrap();
        socket
            .send(Message::Binary(canonical_opus_packet().into()))
            .await
            .unwrap();
        socket
            .send(Message::Text(
                serde_json::json!({"type": "listen", "state": "stop"})
                    .to_string()
                    .into(),
            ))
            .await
            .unwrap();
        if cancel {
            socket
                .send(Message::Text(
                    serde_json::json!({"type": "abort"}).to_string().into(),
                ))
                .await
                .unwrap();
        }
        let messages = collect_until_quiet(&mut socket).await;
        assert!(
            messages.iter().all(|message| message["type"] != "stt"),
            "messages: {messages:?}"
        );
        task.abort();
    }
}
