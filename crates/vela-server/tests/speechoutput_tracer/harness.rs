pub use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

pub use axum::Router;
pub use futures_util::{SinkExt, StreamExt};
pub use opus2::{Channels, Decoder};
pub use tokio::{
    net::TcpListener,
    sync::{Notify, mpsc},
    task::JoinHandle,
    time::timeout,
};
pub use tokio_tungstenite::{
    connect_async,
    tungstenite::{Message, client::IntoClientRequest},
};
pub use url::Url;
pub use vela_server::config::{
    AppConfig, AudioConfig, AuthConfig, BargeInConfig, DeploymentConfig, LimitsConfig, LlmConfig,
    ProvidersConfig, RuntimeConfig, ServerConfig, SpeechOutputConfig, TtsConfig, WebsocketConfig,
    WorkersConfig,
};
pub use vela_server::{
    audio::{DownlinkOpusEncoder, DownlinkPcmFrame, Pcm16Mono, PcmF32Mono},
    protocol::{ClientMessage, ListenCommand, ListenMode},
    providers::{
        AsrError, AsrEvent, AsrProvider, AsrResult, AsrSession, LlmProvider, ProviderSet,
        TtsProvider, VadProvider,
    },
    session::{OutboundMessage, SessionActor, SessionPhase, TurnId},
};

pub struct FakeVad;
impl VadProvider for FakeVad {
    fn open(
        &self,
    ) -> Result<Box<dyn vela_server::providers::VadSession>, vela_server::providers::VadError> {
        Ok(Box::new(FakeVadSession))
    }
    fn adapter(&self) -> &'static str {
        "fake_vad"
    }
}

pub struct FakeVadSession;

impl vela_server::providers::VadSession for FakeVadSession {
    fn push(
        &mut self,
        input: vela_server::providers::VadInput,
    ) -> Result<vela_server::providers::VadProbability, vela_server::providers::VadError> {
        Ok(vela_server::providers::VadProbability {
            start_sample: input.start_sample,
            end_sample: input.start_sample + input.pcm.len() as u64,
            probability: 0.0,
        })
    }

    fn reset(&mut self) -> Result<(), vela_server::providers::VadError> {
        Ok(())
    }
}

pub struct FakeAsr;
impl AsrProvider for FakeAsr {
    fn open(&self) -> Result<Box<dyn AsrSession>, AsrError> {
        Ok(Box::new(FakeAsrSession))
    }
}
pub struct FakeAsrSession;
impl AsrSession for FakeAsrSession {
    fn push_pcm(&mut self, _: &PcmF32Mono) -> Result<Vec<AsrEvent>, AsrError> {
        Ok(vec![])
    }
    fn finish(&mut self) -> Result<AsrResult, AsrError> {
        Ok(AsrResult::new("xin chao"))
    }
    fn cancel(&mut self) {}
}

pub struct FakeLlm;
impl LlmProvider for FakeLlm {
    fn adapter(&self) -> &'static str {
        "fake_llm"
    }
    fn complete(
        &self,
        _: &vela_server::providers::llm::LlmRequest,
    ) -> Result<String, vela_server::providers::LlmError> {
        Ok("Xin chao ban.".into())
    }
}

pub struct StreamingLlm;
#[async_trait::async_trait]
impl LlmProvider for StreamingLlm {
    fn adapter(&self) -> &'static str {
        "streaming_llm"
    }

    async fn stream(
        &self,
        _: vela_server::providers::llm::LlmRequest,
    ) -> Result<vela_server::providers::llm::LlmEventStream, vela_server::providers::LlmError> {
        Ok(Box::pin(
            futures_util::stream::iter([
                Ok(vela_server::providers::LlmEvent::TextDelta(
                    "Cau dau tien.".into(),
                )),
                Ok(vela_server::providers::LlmEvent::TextDelta(
                    " Cau thu hai.".into(),
                )),
                Ok(vela_server::providers::LlmEvent::Finished),
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

pub struct ControlledSentenceLlm {
    pub release_second: Arc<Notify>,
}

#[async_trait::async_trait]
impl LlmProvider for ControlledSentenceLlm {
    fn adapter(&self) -> &'static str {
        "controlled_sentence_llm"
    }

    async fn stream(
        &self,
        _: vela_server::providers::llm::LlmRequest,
    ) -> Result<vela_server::providers::llm::LlmEventStream, vela_server::providers::LlmError> {
        use vela_server::providers::LlmEvent;
        let release = Arc::clone(&self.release_second);
        Ok(Box::pin(
            futures_util::stream::iter([
                Ok(LlmEvent::TextDelta("Xin".into())),
                Ok(LlmEvent::TextDelta(" ch".into())),
                Ok(LlmEvent::TextDelta("ào".into())),
                Ok(LlmEvent::TextDelta("!".into())),
            ])
            .chain(futures_util::stream::once(async move {
                release.notified().await;
                Ok(LlmEvent::TextDelta(
                    " Mình có thể giúp gì cho bạn hôm nay?".into(),
                ))
            }))
            .chain(futures_util::stream::once(async { Ok(LlmEvent::Finished) })),
        ))
    }
}

pub struct BackpressuredLlm {
    pub release_overflow: Arc<Notify>,
}

pub struct GreetingThenRustLlm(pub Arc<AtomicUsize>);

pub struct WrappedLongLlm;

#[async_trait::async_trait]
impl LlmProvider for WrappedLongLlm {
    fn adapter(&self) -> &'static str {
        "wrapped_long_llm"
    }

    async fn stream(
        &self,
        _: vela_server::providers::llm::LlmRequest,
    ) -> Result<vela_server::providers::llm::LlmEventStream, vela_server::providers::LlmError> {
        use vela_server::providers::LlmEvent;
        let text = format!("[{}]", "Rust là ngôn ngữ lập trình hệ thống. ".repeat(9));
        Ok(Box::pin(futures_util::stream::iter([
            Ok(LlmEvent::TextDelta(text)),
            Ok(LlmEvent::Finished),
        ])))
    }
}

#[async_trait::async_trait]
impl LlmProvider for GreetingThenRustLlm {
    fn adapter(&self) -> &'static str {
        "greeting_then_rust"
    }

    async fn stream(
        &self,
        _: vela_server::providers::llm::LlmRequest,
    ) -> Result<vela_server::providers::llm::LlmEventStream, vela_server::providers::LlmError> {
        use vela_server::providers::LlmEvent;
        let text = if self.0.fetch_add(1, Ordering::SeqCst) == 0 {
            "Xin chào! Mình có thể giúp gì cho bạn hôm nay?".to_owned()
        } else {
            format!(
                "Rust là một ngôn ngữ lập trình hệ thống, nổi bật vì kết hợp hiệu năng cao như C/C++ với cơ chế an toàn bộ nhớ rất mạnh. {}",
                "Rust còn hỗ trợ lập trình đồng thời an toàn. ".repeat(10)
            )
        };
        Ok(Box::pin(futures_util::stream::iter([
            Ok(LlmEvent::TextDelta(text)),
            Ok(LlmEvent::Finished),
        ])))
    }
}

#[async_trait::async_trait]
impl LlmProvider for BackpressuredLlm {
    fn adapter(&self) -> &'static str {
        "backpressured_llm"
    }

    async fn stream(
        &self,
        _: vela_server::providers::llm::LlmRequest,
    ) -> Result<vela_server::providers::llm::LlmEventStream, vela_server::providers::LlmError> {
        // One delta carries more complete sentences than pending capacity, so the
        // backpressure result does not depend on how quickly synthesis runs ahead.
        let overflow = std::iter::once(Ok(vela_server::providers::LlmEvent::TextDelta(
            "Mot cau dai du de tach thanh speech segment ngay. ".repeat(10),
        )))
        .chain(std::iter::once(Ok(
            vela_server::providers::LlmEvent::Finished,
        )));
        let release_overflow = Arc::clone(&self.release_overflow);
        Ok(Box::pin(
            futures_util::stream::once(async {
                Ok(vela_server::providers::LlmEvent::TextDelta(
                    "Mot cau dau tien du de tach thanh speech segment ngay. ".into(),
                ))
            })
            .chain(futures_util::stream::once(async move {
                release_overflow.notified().await;
                Ok(vela_server::providers::LlmEvent::TextDelta(
                    "Mot cau thu hai du de tach thanh speech segment ngay. ".into(),
                ))
            }))
            .chain(futures_util::stream::iter(overflow)),
        ))
    }
}

pub struct FailingLlm;
#[async_trait::async_trait]
impl LlmProvider for FailingLlm {
    fn adapter(&self) -> &'static str {
        "failing_llm"
    }

    async fn stream(
        &self,
        _: vela_server::providers::llm::LlmRequest,
    ) -> Result<vela_server::providers::llm::LlmEventStream, vela_server::providers::LlmError> {
        Err(vela_server::providers::LlmError::Failed)
    }
}

pub struct FakeTts;
impl TtsProvider for FakeTts {
    fn adapter(&self) -> &'static str {
        "fake_tts"
    }
    fn synthesize(&self, _: &str) -> Result<PcmF32Mono, vela_server::providers::TtsError> {
        Ok(PcmF32Mono::new(vec![0.1; 2_880], 48_000))
    }
}

pub struct RecordingTts(pub Arc<Mutex<Vec<String>>>);
impl TtsProvider for RecordingTts {
    fn adapter(&self) -> &'static str {
        "recording_tts"
    }
    fn synthesize(&self, text: &str) -> Result<PcmF32Mono, vela_server::providers::TtsError> {
        self.0.lock().unwrap().push(text.to_owned());
        Ok(PcmF32Mono::new(vec![0.1; 2_880], 48_000))
    }
}

pub struct LongTts;
impl TtsProvider for LongTts {
    fn adapter(&self) -> &'static str {
        "long_tts"
    }
    fn synthesize(&self, _: &str) -> Result<PcmF32Mono, vela_server::providers::TtsError> {
        // 48 kHz -> 24 kHz produces three canonical 60 ms downlink frames.
        Ok(PcmF32Mono::new(vec![0.1; 8_640], 48_000))
    }
}

pub fn uplink_packet() -> vela_server::audio::OpusPacket {
    DownlinkOpusEncoder::new(65_536)
        .unwrap()
        .encode(DownlinkPcmFrame::try_new(Pcm16Mono::new(vec![0; 1_440])).unwrap())
        .unwrap()
}

pub async fn start_router() -> (String, JoinHandle<()>) {
    let providers = Arc::new(ProviderSet::with_all(
        Arc::new(FakeVad),
        Arc::new(FakeAsr),
        Arc::new(FakeLlm),
        Arc::new(FakeTts),
    ));
    start_router_with(providers).await
}

pub async fn start_router_with(providers: Arc<ProviderSet>) -> (String, JoinHandle<()>) {
    start_router_with_limits(providers, LimitsConfig::default()).await
}

pub async fn start_router_with_limits(
    providers: Arc<ProviderSet>,
    limits: LimitsConfig,
) -> (String, JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let config = AppConfig {
        orchestrator: Default::default(),
        wiki: Default::default(),
        memory: Default::default(),
        speaker_recognition: Default::default(),
        server: ServerConfig {
            bind: address,
            public_ws_url: Url::parse(&format!("ws://{address}/voice/v1/")).unwrap(),
            hello_timeout_ms: 500,
            trusted_origins: Vec::new(),
        },
        auth: AuthConfig::default(),
        audio: AudioConfig::default(),
        websocket: WebsocketConfig::default(),
        limits,
        provider_defaults: vela_server::config::ProviderDefaultsConfig {
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
        mcp: vela_server::config::McpConfig::default(),
        system_mcp: Default::default(),
        vision: vela_server::config::VisionConfig::default(),
        database: vela_server::config::DatabaseConfig::default(),
        api: vela_server::config::AdminApiConfig {
            enabled: true,
            admin_token: "direct-admin".into(),
            jwt_secret: vela_server::config::SecretString::new("speechoutput-signing-secret-0123456789"),
            ..Default::default()
        },
        shutdown: vela_server::config::ShutdownConfig::default(),
        agent: None,
        effective_agent: vela_server::config::EffectiveAgentConfig::default(),
    };
    let app: Router = crate::support::router(config, providers).await;
    let task = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (format!("http://{address}"), task)
}

pub fn websocket_request(base: &str) -> tokio_tungstenite::tungstenite::handshake::client::Request {
    let mut request = format!("{}/voice/v1/", base.replacen("http", "ws", 1))
        .into_client_request()
        .unwrap();
    let headers = request.headers_mut();
    headers.insert("Protocol-Version", "1".parse().unwrap());
    headers.insert("Device-Id", "tracer-device".parse().unwrap());
    headers.insert("Client-Id", "tracer-client".parse().unwrap());
    request
}

pub async fn connect_router(
    base: &str,
) -> tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>> {
    connect_router_features(base, false).await
}

pub async fn connect_router_features(
    base: &str,
    direct_tts: bool,
) -> tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>> {
    let (mut socket, _) = connect_async(websocket_request(base)).await.unwrap();
    socket
        .send(Message::Text(
            serde_json::json!({
                "type": "hello", "version": 1, "transport": "websocket",
                "features": {"direct_tts": direct_tts},
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

pub struct ControlledFailureTts {
    pub calls: Arc<AtomicUsize>,
    pub after_audio: Option<Mutex<std::sync::mpsc::Receiver<()>>>,
}

impl TtsProvider for ControlledFailureTts {
    fn adapter(&self) -> &'static str {
        "controlled_failure"
    }

    fn synthesize_stream(
        &self,
        _: &str,
        on_pcm: &mut dyn FnMut(PcmF32Mono) -> Result<(), vela_server::providers::TtsError>,
    ) -> Result<(), vela_server::providers::TtsError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        if let Some(release) = &self.after_audio {
            // Fill the public streaming high-water mark so playback starts before failure.
            on_pcm(PcmF32Mono::new(vec![0.1; 48_000 * 2], 48_000))?;
            let _ = release.lock().unwrap().recv_timeout(Duration::from_secs(5));
        }
        Err(vela_server::providers::TtsError::Failed)
    }
}

pub struct CountedAsr(pub Arc<AtomicUsize>);
impl AsrProvider for CountedAsr {
    fn open(&self) -> Result<Box<dyn AsrSession>, AsrError> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Ok(Box::new(FakeAsrSession))
    }
}
