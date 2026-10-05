mod support;

use std::{path::PathBuf, sync::Arc, time::Duration};

use axum::Router;
use futures_util::{SinkExt, StreamExt};
use opus2::{Application, Channels, Decoder, Encoder};
use tokio::{net::TcpListener, sync::Notify, task::JoinHandle, time::timeout};
use tokio_tungstenite::{
    connect_async,
    tungstenite::{Message, client::IntoClientRequest},
};
use url::Url;
use voice_agent_server::{
    audio::PcmF32Mono,
    config::{
        AppConfig, AudioConfig, AuthConfig, DeploymentConfig, LimitsConfig, LlmConfig,
        OnnxRuntimeConfig, ProvidersConfig, RuntimeConfig, ServerConfig, SpeechOutputConfig,
        TtsConfig, WebsocketConfig, WorkersConfig,
    },
    providers::{
        AsrError, AsrEvent, AsrProvider, AsrResult, AsrSession, LlmEvent, LlmProvider, ProviderSet,
        TtsProvider, VadError, VadProvider, VadSession, compiled_provider_registry,
    },
};

struct ManualOnlyVad;
impl VadProvider for ManualOnlyVad {
    fn open(&self) -> Result<Box<dyn VadSession>, VadError> {
        Err(VadError::Failed("manual gate".into()))
    }
    fn adapter(&self) -> &'static str {
        "manual_gate"
    }
}

struct FinalAsr;
impl AsrProvider for FinalAsr {
    fn open(&self) -> Result<Box<dyn AsrSession>, AsrError> {
        Ok(Box::new(FinalAsrSession))
    }
}
struct FinalAsrSession;
impl AsrSession for FinalAsrSession {
    fn push_pcm(&mut self, _: &PcmF32Mono) -> Result<Vec<AsrEvent>, AsrError> {
        Ok(vec![])
    }
    fn finish(&mut self) -> Result<AsrResult, AsrError> {
        Ok(AsrResult::new("phase four gate"))
    }
    fn cancel(&mut self) {}
}

struct ControlledLlm {
    release: Arc<Notify>,
}
#[async_trait::async_trait]
impl LlmProvider for ControlledLlm {
    fn adapter(&self) -> &'static str {
        "controlled_gate"
    }
    async fn stream(
        &self,
        _: voice_agent_server::providers::llm::LlmRequest,
    ) -> Result<
        voice_agent_server::providers::llm::LlmEventStream,
        voice_agent_server::providers::LlmError,
    > {
        let release = Arc::clone(&self.release);
        Ok(Box::pin(
            futures_util::stream::once(async {
                Ok(LlmEvent::TextDelta(
                    "Phan hoi audio that cho Reference Client.".into(),
                ))
            })
            .chain(futures_util::stream::once(async move {
                release.notified().await;
                Ok(LlmEvent::Finished)
            })),
        ))
    }
}

fn runtime_library() -> PathBuf {
    std::env::var_os("VOICE_ONNX_RUNTIME_LIB")
        .map(PathBuf::from)
        .expect("Phase 4 real-model gate unavailable: set VOICE_ONNX_RUNTIME_LIB")
}

fn real_tts() -> Arc<dyn TtsProvider> {
    let runtime = runtime_library();
    assert!(
        runtime.is_file(),
        "Phase 4 real-model gate unavailable: ONNX Runtime is missing"
    );
    // The model root is an internal constant relative to the working directory, so the gate runs
    // from the workspace root where `models/` lives. Tests otherwise start in the package
    // directory, which has no models.
    let workspace = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    assert!(
        voice_agent_server::providers::tts::zerotts::assets::model_dir().exists(),
        "Phase 4 real-model gate unavailable: {} does not exist",
        voice_agent_server::providers::tts::zerotts::assets::model_dir().display()
    );
    std::env::set_current_dir(&workspace).expect("enter the workspace root");
    compiled_provider_registry()
        .tts_factory("zerotts_onnx")
        .unwrap()
        .build(
            deployment_placeholder_tts_config(),
            &RuntimeConfig {
                onnx: OnnxRuntimeConfig {
                    library: runtime,
                    ..Default::default()
                },
                ..RuntimeConfig::default()
            },
        )
        .unwrap()
}

fn deployment_placeholder_tts_config() -> &'static voice_agent_server::config::TtsInstanceConfig {
    static CONFIG: std::sync::OnceLock<voice_agent_server::config::TtsInstanceConfig> =
        std::sync::OnceLock::new();
    CONFIG.get_or_init(|| {
        voice_agent_server::config::TtsInstanceConfig::ZeroTtsOnnx(
            voice_agent_server::config::ZeroTtsOnnxConfig {
                model: "zerotts_default".into(),
                num_threads: 1,
                voice: "maichi".into(),
                language: "vi-VN".into(),
                delivery_mode: Default::default(),
                preload: false,
            },
        )
    })
}

async fn start(llm: Arc<dyn LlmProvider>, tts: Arc<dyn TtsProvider>) -> (String, JoinHandle<()>) {
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
        barge_in: voice_agent_server::config::BargeInConfig::default(),
        mcp: voice_agent_server::config::McpConfig::default(),
        vision: voice_agent_server::config::VisionConfig::default(),
        database: voice_agent_server::config::DatabaseConfig::default(),
        api: voice_agent_server::config::AdminApiConfig::default(),
        shutdown: voice_agent_server::config::ShutdownConfig::default(),
        agent: None,
        effective_agent: voice_agent_server::config::EffectiveAgentConfig::default(),
    };
    let providers = Arc::new(ProviderSet::with_all(
        Arc::new(ManualOnlyVad),
        Arc::new(FinalAsr),
        llm,
        tts,
    ));
    let app: Router = support::router(config, providers).await;
    let task = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (format!("http://{address}"), task)
}

async fn connect_and_start(
    base: &str,
) -> tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>> {
    let mut request = format!("{}/voice/v1/", base.replacen("http", "ws", 1))
        .into_client_request()
        .unwrap();
    request
        .headers_mut()
        .insert("Protocol-Version", "1".parse().unwrap());
    request
        .headers_mut()
        .insert("Device-Id", "phase4-gate".parse().unwrap());
    request
        .headers_mut()
        .insert("Client-Id", "reference-client".parse().unwrap());
    let (mut socket, _) = connect_async(request).await.unwrap();
    socket.send(Message::Text(serde_json::json!({"type":"hello","version":1,"transport":"websocket","audio_params":{"format":"opus","sample_rate":16000,"channels":1,"frame_duration":60}}).to_string().into())).await.unwrap();
    assert!(matches!(
        socket.next().await.unwrap().unwrap(),
        Message::Text(_)
    ));
    socket
        .send(Message::Text(
            serde_json::json!({"type":"listen","state":"start","mode":"manual"})
                .to_string()
                .into(),
        ))
        .await
        .unwrap();
    let mut encoder = Encoder::new(16_000, Channels::Mono, Application::Voip).unwrap();
    let mut packet = [0; 4_000];
    let length = encoder.encode(&[1_000; 960], &mut packet).unwrap();
    socket
        .send(Message::Binary(packet[..length].to_vec().into()))
        .await
        .unwrap();
    socket
        .send(Message::Text(
            serde_json::json!({"type":"listen","state":"stop"})
                .to_string()
                .into(),
        ))
        .await
        .unwrap();
    socket
}

async fn await_first_audio(
    socket: &mut tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
) -> Vec<Message> {
    let mut observed = Vec::new();
    loop {
        let message = timeout(Duration::from_secs(90), socket.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        if let Message::Binary(packet) = &message {
            let mut decoder = Decoder::new(24_000, Channels::Mono).unwrap();
            let mut pcm = [0_i16; 1_440];
            assert_eq!(decoder.decode(packet, &mut pcm, false).unwrap(), 1_440);
            observed.push(message);
            return observed;
        }
        observed.push(message);
    }
}

fn tts_state(message: &Message, state: &str) -> bool {
    matches!(message, Message::Text(text) if serde_json::from_str::<serde_json::Value>(text).unwrap()["type"] == "tts" && serde_json::from_str::<serde_json::Value>(text).unwrap()["state"] == state)
}

#[tokio::test]
async fn real_zerotts_reference_client_gate_preserves_delivery_and_blocks_invalidated_audio() {
    let tts = real_tts();
    let release = Arc::new(Notify::new());
    let (base, task) = start(
        Arc::new(ControlledLlm {
            release: Arc::clone(&release),
        }),
        Arc::clone(&tts),
    )
    .await;
    let mut socket = connect_and_start(&base).await;
    let observed = await_first_audio(&mut socket).await;
    assert!(
        observed.iter().any(|message| tts_state(message, "start")),
        "tts:start must precede real audio"
    );
    release.notify_one();
    let mut after = Vec::new();
    while after.iter().all(|message| !tts_state(message, "stop")) {
        after.push(
            timeout(Duration::from_secs(10), socket.next())
                .await
                .unwrap()
                .unwrap()
                .unwrap(),
        );
    }
    assert_eq!(
        after
            .iter()
            .filter(|message| tts_state(message, "stop"))
            .count(),
        1
    );
    task.abort();

    let release = Arc::new(Notify::new());
    let (base, task) = start(Arc::new(ControlledLlm { release }), tts).await;
    let mut socket = connect_and_start(&base).await;
    let observed = await_first_audio(&mut socket).await;
    assert!(observed.iter().any(|message| tts_state(message, "start")));
    socket
        .send(Message::Text(
            serde_json::json!({"type":"abort"}).to_string().into(),
        ))
        .await
        .unwrap();
    let mut after_stop = Vec::new();
    while after_stop.iter().all(|message| !tts_state(message, "stop")) {
        after_stop.push(
            timeout(Duration::from_secs(10), socket.next())
                .await
                .unwrap()
                .unwrap()
                .unwrap(),
        );
    }
    assert_eq!(
        after_stop
            .iter()
            .filter(|message| tts_state(message, "stop"))
            .count(),
        1
    );
    assert!(
        timeout(Duration::from_millis(300), socket.next())
            .await
            .is_err(),
        "cancelled generation must not deliver audio after its stop"
    );
    task.abort();
}
