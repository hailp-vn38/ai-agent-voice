use std::{
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

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
    audio::PcmF32Mono,
    config::{
        AppConfig, AudioConfig, AuthConfig, BargeInConfig, DeploymentConfig, LimitsConfig,
        LlmConfig, ProvidersConfig, RuntimeConfig, ServerConfig, SpeechOutputConfig, TtsConfig,
        WebsocketConfig, WorkersConfig,
    },
    providers::{
        AsrError, AsrEvent, AsrProvider, AsrResult, AsrSession, LlmError, LlmProvider, ProviderSet,
        TtsError, TtsProvider, VadError, VadInput, VadProbability, VadProvider, VadSession,
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

struct TwoTurnAsr(Arc<AtomicUsize>);

impl AsrProvider for TwoTurnAsr {
    fn open(&self) -> Result<Box<dyn AsrSession>, AsrError> {
        Ok(Box::new(TwoTurnAsrSession(Arc::clone(&self.0))))
    }
}

struct TwoTurnAsrSession(Arc<AtomicUsize>);

impl AsrSession for TwoTurnAsrSession {
    fn push_pcm(
        &mut self,
        _: &voice_agent_server::audio::PcmF32Mono,
    ) -> Result<Vec<AsrEvent>, AsrError> {
        Ok(vec![])
    }

    fn finish(&mut self) -> Result<AsrResult, AsrError> {
        let turn = self.0.fetch_add(1, Ordering::SeqCst);
        Ok(AsrResult::new(if turn == 0 {
            "utterance A"
        } else {
            "utterance B"
        }))
    }

    fn cancel(&mut self) {}
}

struct DeterministicLlm;

impl LlmProvider for DeterministicLlm {
    fn adapter(&self) -> &'static str {
        "deterministic-llm"
    }

    fn complete(&self, prompt: &str) -> Result<String, LlmError> {
        Ok(format!("answer for {prompt}"))
    }
}

struct LongTts;

impl TtsProvider for LongTts {
    fn adapter(&self) -> &'static str {
        "long-tts"
    }

    fn synthesize(&self, _: &str) -> Result<PcmF32Mono, TtsError> {
        // Keep A speaking long enough for B to cross the public WebSocket boundary.
        Ok(PcmF32Mono::new(vec![0.1; 28_800], 48_000))
    }
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
            // Two Opus packets yield three 512-sample VAD windows. Leave the third
            // window silent, then make B's first two windows a complete utterance.
            probability: if matches!(self.frames, 1 | 4) {
                1.0
            } else {
                0.0
            },
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

async fn start_barge_in() -> (String, JoinHandle<()>) {
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
        barge_in: BargeInConfig {
            enabled: true,
            trust_client_aec_feature: true,
        },
    };
    let providers = Arc::new(ProviderSet::with_all(
        Arc::new(SpeechThenSilenceVad),
        Arc::new(TwoTurnAsr(Arc::new(AtomicUsize::new(0)))),
        Arc::new(DeterministicLlm),
        Arc::new(LongTts),
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

async fn connect_with_aec(
    base: &str,
) -> tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>> {
    let (mut socket, _) = connect_async(request(base)).await.unwrap();
    socket
        .send(Message::Text(serde_json::json!({
            "type": "hello", "version": 1, "transport": "websocket",
            "audio_params": {"format": "opus", "sample_rate": 16000, "channels": 1, "frame_duration": 60},
            "features": {"aec": true}
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

async fn collect_frames_until_quiet(
    socket: &mut tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
) -> Vec<Message> {
    let mut messages = Vec::new();
    loop {
        match timeout(Duration::from_millis(250), socket.next()).await {
            Ok(Some(Ok(message))) => messages.push(message),
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

#[tokio::test]
async fn trusted_aec_auto_barge_in_stops_a_once_and_delivers_b_on_the_same_socket() {
    let (base, task) = start_barge_in().await;
    let mut socket = connect_with_aec(&base).await;
    socket
        .send(Message::Text(
            serde_json::json!({"type": "listen", "state": "start", "mode": "auto"})
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
        .send(Message::Binary(canonical_opus_packet().into()))
        .await
        .unwrap();

    let mut before_b = Vec::new();
    loop {
        let message = timeout(Duration::from_secs(2), socket.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        if let Message::Text(text) = message {
            let value: serde_json::Value = serde_json::from_str(&text).unwrap();
            let started_a = value["type"] == "tts" && value["state"] == "start";
            before_b.push(value);
            if started_a {
                break;
            }
        }
    }

    socket
        .send(Message::Binary(canonical_opus_packet().into()))
        .await
        .unwrap();
    socket
        .send(Message::Binary(canonical_opus_packet().into()))
        .await
        .unwrap();
    let after_b = collect_frames_until_quiet(&mut socket).await;
    let text_after_b = after_b
        .iter()
        .filter_map(|message| match message {
            Message::Text(text) => Some(serde_json::from_str::<serde_json::Value>(text).unwrap()),
            _ => None,
        })
        .collect::<Vec<_>>();
    let messages = before_b
        .iter()
        .cloned()
        .chain(text_after_b.iter().cloned())
        .collect::<Vec<_>>();

    assert!(
        messages
            .iter()
            .any(|message| message["type"] == "stt" && message["text"] == "utterance B"),
        "messages: {messages:?}"
    );
    assert!(
        messages
            .iter()
            .any(|message| message["type"] == "llm" && message["text"] == "answer for utterance B"),
        "messages: {messages:?}"
    );
    let stop_index = after_b
        .iter()
        .position(|message| {
            matches!(message, Message::Text(text) if serde_json::from_str::<serde_json::Value>(text).is_ok_and(|value| value["type"] == "tts" && value["state"] == "stop"))
        })
        .expect("the interruption stop must be emitted after B ingress");
    let b_start_index = after_b
        .iter()
        .position(|message| {
            matches!(message, Message::Text(text) if serde_json::from_str::<serde_json::Value>(text).is_ok_and(|value| value["type"] == "tts" && value["state"] == "start"))
        })
        .unwrap_or_else(|| panic!("B must begin protocol-visible TTS delivery: {after_b:?}"));
    assert!(stop_index < b_start_index, "frames: {after_b:?}");
    assert_eq!(
        after_b[..b_start_index]
            .iter()
            .filter(|message| {
                matches!(message, Message::Text(text) if serde_json::from_str::<serde_json::Value>(text).is_ok_and(|value| value["type"] == "tts" && value["state"] == "stop"))
            })
            .count(),
        1,
        "A must receive exactly one interruption stop: {after_b:?}"
    );
    assert!(
        after_b[stop_index + 1..b_start_index]
            .iter()
            .all(|message| !matches!(message, Message::Binary(_))),
        "stale A audio crossed the stop boundary: {after_b:?}"
    );
    assert!(
        after_b[b_start_index + 1..]
            .iter()
            .any(|message| matches!(message, Message::Binary(_))),
        "B must deliver at least one Opus frame: {after_b:?}"
    );
    assert!(
        after_b[stop_index + 1..].iter().all(|message| {
            !matches!(message, Message::Text(text) if text.contains("answer for utterance A"))
        }),
        "old A control payload revived after the interruption boundary: {after_b:?}"
    );
    task.abort();
}
