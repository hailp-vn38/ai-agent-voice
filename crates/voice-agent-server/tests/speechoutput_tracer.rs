use std::{sync::Arc, time::Duration};

use axum::Router;
use futures_util::{SinkExt, StreamExt};
use opus2::{Channels, Decoder};
use tokio::{net::TcpListener, sync::mpsc, task::JoinHandle, time::timeout};
use tokio_tungstenite::{
    connect_async,
    tungstenite::{Message, client::IntoClientRequest},
};
use url::Url;
use voice_agent_server::config::{
    AppConfig, AudioConfig, AuthConfig, DeploymentConfig, LimitsConfig, LlmConfig, ProvidersConfig,
    RuntimeConfig, ServerConfig, SpeechOutputConfig, TtsConfig, WebsocketConfig, WorkersConfig,
};
use voice_agent_server::{
    app::router_with_providers,
    audio::{DownlinkOpusEncoder, DownlinkPcmFrame, Pcm16Mono, PcmF32Mono},
    protocol::{ClientMessage, ListenCommand, ListenMode},
    providers::{
        AsrError, AsrEvent, AsrProvider, AsrResult, AsrSession, LlmProvider, ProviderSet,
        TtsProvider, VadProvider,
    },
    session::{OutboundMessage, SessionActor, SessionPhase},
};

struct FakeVad;
impl VadProvider for FakeVad {
    fn open(
        &self,
    ) -> Result<
        Box<dyn voice_agent_server::providers::VadSession>,
        voice_agent_server::providers::VadError,
    > {
        Err(voice_agent_server::providers::VadError::Failed(
            "manual only".into(),
        ))
    }
    fn adapter(&self) -> &'static str {
        "fake_vad"
    }
}

struct FakeAsr;
impl AsrProvider for FakeAsr {
    fn open(&self) -> Result<Box<dyn AsrSession>, AsrError> {
        Ok(Box::new(FakeAsrSession))
    }
}
struct FakeAsrSession;
impl AsrSession for FakeAsrSession {
    fn push_pcm(&mut self, _: &PcmF32Mono) -> Result<Vec<AsrEvent>, AsrError> {
        Ok(vec![])
    }
    fn finish(&mut self) -> Result<AsrResult, AsrError> {
        Ok(AsrResult::new("xin chao"))
    }
    fn cancel(&mut self) {}
}

struct FakeLlm;
impl LlmProvider for FakeLlm {
    fn adapter(&self) -> &'static str {
        "fake_llm"
    }
    fn complete(&self, _: &str) -> Result<String, voice_agent_server::providers::LlmError> {
        Ok("Xin chao ban.".into())
    }
}

struct StreamingLlm;
#[async_trait::async_trait]
impl LlmProvider for StreamingLlm {
    fn adapter(&self) -> &'static str {
        "streaming_llm"
    }

    async fn stream(
        &self,
        _: String,
    ) -> Result<
        voice_agent_server::providers::llm::LlmEventStream,
        voice_agent_server::providers::LlmError,
    > {
        Ok(Box::pin(
            futures_util::stream::iter([
                Ok(voice_agent_server::providers::LlmEvent::TextDelta(
                    "Cau dau tien.".into(),
                )),
                Ok(voice_agent_server::providers::LlmEvent::TextDelta(
                    " Cau thu hai.".into(),
                )),
                Ok(voice_agent_server::providers::LlmEvent::Finished),
            ])
            .enumerate()
            .then(|(index, event)| async move {
                if index == 1 {
                    tokio::time::sleep(Duration::from_millis(120)).await;
                }
                event
            }),
        ))
    }
}

struct BackpressuredLlm;
#[async_trait::async_trait]
impl LlmProvider for BackpressuredLlm {
    fn adapter(&self) -> &'static str {
        "backpressured_llm"
    }

    async fn stream(
        &self,
        _: String,
    ) -> Result<
        voice_agent_server::providers::llm::LlmEventStream,
        voice_agent_server::providers::LlmError,
    > {
        let mut events = (0..10)
            .map(|_| {
                Ok(voice_agent_server::providers::LlmEvent::TextDelta(
                    "Mot cau dai du de tach thanh speech segment ngay. ".into(),
                ))
            })
            .collect::<Vec<_>>();
        events.push(Ok(voice_agent_server::providers::LlmEvent::Finished));
        Ok(Box::pin(futures_util::stream::iter(events)))
    }
}

struct FailingLlm;
#[async_trait::async_trait]
impl LlmProvider for FailingLlm {
    fn adapter(&self) -> &'static str {
        "failing_llm"
    }

    async fn stream(
        &self,
        _: String,
    ) -> Result<
        voice_agent_server::providers::llm::LlmEventStream,
        voice_agent_server::providers::LlmError,
    > {
        Err(voice_agent_server::providers::LlmError::Failed)
    }
}

struct FakeTts;
impl TtsProvider for FakeTts {
    fn adapter(&self) -> &'static str {
        "fake_tts"
    }
    fn synthesize(&self, _: &str) -> Result<PcmF32Mono, voice_agent_server::providers::TtsError> {
        Ok(PcmF32Mono::new(vec![0.1; 2_880], 48_000))
    }
}

fn uplink_packet() -> voice_agent_server::audio::OpusPacket {
    DownlinkOpusEncoder::new(65_536)
        .unwrap()
        .encode(DownlinkPcmFrame::try_new(Pcm16Mono::new(vec![0; 1_440])).unwrap())
        .unwrap()
}

async fn start_router() -> (String, JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
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
        providers: ProvidersConfig::default(),
        workers: WorkersConfig::default(),
        deployment: DeploymentConfig::default(),
        runtime: RuntimeConfig::default(),
        llm: LlmConfig::default(),
        tts: TtsConfig::default(),
        speech_output: SpeechOutputConfig::default(),
    };
    let providers = Arc::new(ProviderSet::with_all(
        Arc::new(FakeVad),
        Arc::new(FakeAsr),
        Arc::new(FakeLlm),
        Arc::new(FakeTts),
    ));
    let app: Router = router_with_providers(config, providers);
    let task = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (format!("http://{address}"), task)
}

fn websocket_request(base: &str) -> tokio_tungstenite::tungstenite::handshake::client::Request {
    let mut request = format!("{}/voice/v1/", base.replacen("http", "ws", 1))
        .into_client_request()
        .unwrap();
    let headers = request.headers_mut();
    headers.insert("Protocol-Version", "1".parse().unwrap());
    headers.insert("Device-Id", "tracer-device".parse().unwrap());
    headers.insert("Client-Id", "tracer-client".parse().unwrap());
    request
}

async fn connect_router(
    base: &str,
) -> tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>> {
    let (mut socket, _) = connect_async(websocket_request(base)).await.unwrap();
    socket
        .send(Message::Text(
            serde_json::json!({
                "type": "hello", "version": 1, "transport": "websocket",
                "audio_params": {"format": "opus", "sample_rate": 16000, "channels": 1, "frame_duration": 60}
            })
            .to_string()
            .into(),
        ))
        .await
        .unwrap();
    assert!(matches!(
        socket.next().await.unwrap().unwrap(),
        Message::Text(_)
    ));
    socket
}

#[tokio::test]
async fn non_empty_asr_final_delivers_started_canonical_opus_then_one_stop() {
    let (control_tx, mut control_rx) = mpsc::channel(8);
    let (audio_tx, mut audio_rx) = mpsc::channel(8);
    let providers = Arc::new(ProviderSet::with_all(
        Arc::new(FakeVad),
        Arc::new(FakeAsr),
        Arc::new(FakeLlm),
        Arc::new(FakeTts),
    ));
    let mut actor =
        SessionActor::new("session".into(), control_tx, audio_tx, 2, providers).unwrap();

    actor.on_client_message(ClientMessage::Listen(ListenCommand::Start {
        mode: ListenMode::Manual,
    }));
    assert!(actor.on_binary(uplink_packet().as_bytes().to_vec()));
    actor.on_client_message(ClientMessage::Listen(ListenCommand::Stop));

    for _ in 0..200 {
        actor.pump_workers();
        if actor.phase() == SessionPhase::Ready {
            break;
        }
        tokio::time::sleep(Duration::from_millis(1)).await;
    }
    assert_eq!(actor.phase(), SessionPhase::Ready);

    let controls = std::iter::from_fn(|| control_rx.try_recv().ok())
        .map(|message| message.as_text().unwrap().to_owned())
        .collect::<Vec<_>>();
    assert_eq!(controls.len(), 3);
    let controls = controls
        .iter()
        .map(|control| serde_json::from_str::<serde_json::Value>(control).unwrap())
        .collect::<Vec<_>>();
    assert_eq!(controls[0]["type"], "stt");
    assert_eq!(controls[1]["type"], "tts");
    assert_eq!(controls[1]["state"], "start");
    assert_eq!(controls[2]["type"], "tts");
    assert_eq!(controls[2]["state"], "stop");

    let packet = match audio_rx.try_recv().unwrap() {
        OutboundMessage::Binary { packet, .. } => packet,
        unexpected => panic!("expected downlink audio, got {unexpected:?}"),
    };
    assert!(audio_rx.try_recv().is_err());
    let mut decoder = Decoder::new(24_000, Channels::Mono).unwrap();
    let mut pcm = [0_i16; 1_440];
    assert_eq!(decoder.decode(&packet, &mut pcm, false).unwrap(), 1_440);
}

#[tokio::test]
async fn streaming_llm_delivers_first_audio_before_eof_and_commits_only_after_drained() {
    let (control_tx, mut control_rx) = mpsc::channel(16);
    let (audio_tx, mut audio_rx) = mpsc::channel(16);
    let providers = Arc::new(ProviderSet::with_all(
        Arc::new(FakeVad),
        Arc::new(FakeAsr),
        Arc::new(StreamingLlm),
        Arc::new(FakeTts),
    ));
    let mut actor =
        SessionActor::new("session".into(), control_tx, audio_tx, 2, providers).unwrap();
    actor.on_client_message(ClientMessage::Listen(ListenCommand::Start {
        mode: ListenMode::Manual,
    }));
    assert!(actor.on_binary(uplink_packet().as_bytes().to_vec()));
    actor.on_client_message(ClientMessage::Listen(ListenCommand::Stop));

    for _ in 0..80 {
        actor.pump_workers();
        if audio_rx.try_recv().is_ok() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(2)).await;
    }
    assert_eq!(
        actor.phase(),
        SessionPhase::Processing,
        "first audio arrives while LLM still streams"
    );
    assert_eq!(
        actor.dialogue_history(),
        ["xin chao"],
        "assistant is not delivered yet"
    );
    let controls = std::iter::from_fn(|| control_rx.try_recv().ok()).collect::<Vec<_>>();
    assert!(controls.iter().any(|message| {
        message
            .as_text()
            .is_some_and(|text| text.contains(r#""state":"start""#))
    }));
    assert!(!controls.iter().any(|message| {
        message
            .as_text()
            .is_some_and(|text| text.contains(r#""state":"stop""#))
    }));

    for _ in 0..300 {
        actor.pump_workers();
        if actor.phase() == SessionPhase::Ready {
            break;
        }
        tokio::time::sleep(Duration::from_millis(2)).await;
    }
    assert_eq!(actor.phase(), SessionPhase::Ready);
    assert_eq!(
        actor.dialogue_history(),
        ["xin chao", "Cau dau tien. Cau thu hai."]
    );
}

#[tokio::test]
async fn full_segment_capacity_stops_started_playback_without_committing_assistant_history() {
    let (control_tx, mut control_rx) = mpsc::channel(32);
    let (audio_tx, _audio_rx) = mpsc::channel(32);
    let providers = Arc::new(ProviderSet::with_all(
        Arc::new(FakeVad),
        Arc::new(FakeAsr),
        Arc::new(BackpressuredLlm),
        Arc::new(FakeTts),
    ));
    let mut actor =
        SessionActor::new("session".into(), control_tx, audio_tx, 2, providers).unwrap();
    actor.on_client_message(ClientMessage::Listen(ListenCommand::Start {
        mode: ListenMode::Manual,
    }));
    assert!(actor.on_binary(uplink_packet().as_bytes().to_vec()));
    actor.on_client_message(ClientMessage::Listen(ListenCommand::Stop));
    for _ in 0..100 {
        actor.pump_workers();
        if actor.phase() == SessionPhase::Ready {
            break;
        }
        tokio::time::sleep(Duration::from_millis(2)).await;
    }
    assert_eq!(actor.phase(), SessionPhase::Ready);
    assert_eq!(actor.dialogue_history(), ["xin chao"]);
    let controls = std::iter::from_fn(|| control_rx.try_recv().ok())
        .filter_map(|message| message.as_text().map(str::to_owned))
        .collect::<Vec<_>>();
    assert_eq!(
        controls
            .iter()
            .filter(|text| text.contains(r#""state":"start""#))
            .count(),
        1
    );
    assert_eq!(
        controls
            .iter()
            .filter(|text| text.contains(r#""state":"stop""#))
            .count(),
        1
    );
}

#[tokio::test]
async fn llm_failure_before_audio_emits_no_playback_controls_or_assistant_history() {
    let (control_tx, mut control_rx) = mpsc::channel(16);
    let (audio_tx, mut audio_rx) = mpsc::channel(16);
    let providers = Arc::new(ProviderSet::with_all(
        Arc::new(FakeVad),
        Arc::new(FakeAsr),
        Arc::new(FailingLlm),
        Arc::new(FakeTts),
    ));
    let mut actor =
        SessionActor::new("session".into(), control_tx, audio_tx, 2, providers).unwrap();
    actor.on_client_message(ClientMessage::Listen(ListenCommand::Start {
        mode: ListenMode::Manual,
    }));
    assert!(actor.on_binary(uplink_packet().as_bytes().to_vec()));
    actor.on_client_message(ClientMessage::Listen(ListenCommand::Stop));
    for _ in 0..100 {
        actor.pump_workers();
        if actor.phase() == SessionPhase::Ready {
            break;
        }
        tokio::time::sleep(Duration::from_millis(2)).await;
    }
    assert_eq!(actor.phase(), SessionPhase::Ready);
    assert_eq!(actor.dialogue_history(), ["xin chao"]);
    assert!(audio_rx.try_recv().is_err());
    let controls = std::iter::from_fn(|| control_rx.try_recv().ok())
        .filter_map(|message| message.as_text().map(str::to_owned))
        .collect::<Vec<_>>();
    assert!(!controls.iter().any(|text| text.contains(r#""type":"tts""#)));
}

#[tokio::test]
async fn router_writes_tts_start_then_canonical_audio_then_one_stop() {
    let (base, task) = start_router().await;
    let mut socket = connect_router(&base).await;
    socket
        .send(Message::Text(
            serde_json::json!({"type": "listen", "state": "start", "mode": "manual"})
                .to_string()
                .into(),
        ))
        .await
        .unwrap();
    socket
        .send(Message::Binary(uplink_packet().as_bytes().to_vec().into()))
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

    let mut observed = Vec::new();
    while observed.len() < 4 {
        observed.push(
            timeout(Duration::from_secs(1), socket.next())
                .await
                .unwrap()
                .unwrap()
                .unwrap(),
        );
    }
    task.abort();

    assert!(
        matches!(&observed[0], Message::Text(text) if serde_json::from_str::<serde_json::Value>(text).unwrap()["type"] == "stt")
    );
    assert!(matches!(&observed[1], Message::Text(text) if {
        let value: serde_json::Value = serde_json::from_str(text).unwrap();
        value["type"] == "tts" && value["state"] == "start"
    }));
    let Message::Binary(packet) = &observed[2] else {
        panic!("expected paced audio")
    };
    let mut decoder = Decoder::new(24_000, Channels::Mono).unwrap();
    let mut pcm = [0_i16; 1_440];
    assert_eq!(decoder.decode(packet, &mut pcm, false).unwrap(), 1_440);
    assert!(matches!(&observed[3], Message::Text(text) if {
        let value: serde_json::Value = serde_json::from_str(text).unwrap();
        value["type"] == "tts" && value["state"] == "stop"
    }));
}
