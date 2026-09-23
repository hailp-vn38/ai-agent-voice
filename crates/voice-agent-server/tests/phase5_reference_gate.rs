use std::{
    fs,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::{SystemTime, UNIX_EPOCH},
};

use axum::Router;
use tokio::{net::TcpListener, task::JoinHandle};
use url::Url;
use voice_agent_server::{
    app::router_with_providers,
    audio::PcmF32Mono,
    config::{
        AppConfig, AudioConfig, AuthConfig, BargeInConfig, DeploymentConfig, LimitsConfig,
        LlmConfig, ModelAcknowledgement, ModelStoreConfig, OnnxRuntimeConfig, ProvidersConfig,
        RuntimeConfig, ServerConfig, SpeechOutputConfig, TtsConfig, WebsocketConfig, WorkersConfig,
    },
    models::prepare,
    providers::{
        AsrError, AsrEvent, AsrProvider, AsrResult, AsrSession, LlmError, LlmProvider, ProviderSet,
        TtsProvider, VadError, VadInput, VadProbability, VadProvider, VadSession,
        compiled_provider_registry,
    },
};
use voice_reference_client::{BargeInConfig as ReferenceClientConfig, BargeInMode, BargeInRequest};

const UPLINK_A: [&[u8]; 2] = [
    include_bytes!("../../voice-reference-client/tests/fixtures/phase5-uplink-02-speech-a.opus"),
    include_bytes!("../../voice-reference-client/tests/fixtures/phase5-uplink-03-silence.opus"),
];
const UPLINK_B: [&[u8]; 2] = [
    include_bytes!("../../voice-reference-client/tests/fixtures/phase5-uplink-04-speech-b.opus"),
    include_bytes!("../../voice-reference-client/tests/fixtures/phase5-uplink-05-silence.opus"),
];

struct TwoTurnAsr(Arc<AtomicUsize>);

impl AsrProvider for TwoTurnAsr {
    fn open(&self) -> Result<Box<dyn AsrSession>, AsrError> {
        Ok(Box::new(TwoTurnAsrSession(Arc::clone(&self.0))))
    }
}

struct TwoTurnAsrSession(Arc<AtomicUsize>);

impl AsrSession for TwoTurnAsrSession {
    fn push_pcm(&mut self, _: &PcmF32Mono) -> Result<Vec<AsrEvent>, AsrError> {
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
        "phase5-deterministic-llm"
    }

    fn complete(&self, prompt: &str) -> Result<String, LlmError> {
        Ok(format!("Phase 5 response for {prompt}."))
    }
}

struct SpeechThenSilenceVad;

impl VadProvider for SpeechThenSilenceVad {
    fn open(&self) -> Result<Box<dyn VadSession>, VadError> {
        Ok(Box::new(SpeechThenSilenceVadSession { frames: 0 }))
    }

    fn adapter(&self) -> &'static str {
        "phase5-deterministic-vad"
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

fn runtime_library() -> PathBuf {
    std::env::var_os("VOICE_ONNX_RUNTIME_LIB")
        .map(PathBuf::from)
        .expect("Phase 5 real-model gate unavailable: set VOICE_ONNX_RUNTIME_LIB")
}

fn real_tts() -> Arc<dyn TtsProvider> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("models");
    let runtime = runtime_library();
    assert!(
        runtime.is_file(),
        "Phase 5 real-model gate unavailable: ONNX Runtime is missing"
    );
    let manifest = include_str!("../../../models/manifest.toml")
        .replace("install_path = \"tts/zerotts/", "install_path = \"zerotts/");
    let path = std::env::temp_dir().join(format!(
        "phase5-manifest-{}.toml",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    fs::write(&path, manifest).unwrap();
    let deployment = DeploymentConfig {
        model_manifest: path.clone(),
        models: ModelStoreConfig {
            root,
            offline: true,
        },
        model_acknowledgements: vec![ModelAcknowledgement {
            model: "zerotts_default".into(),
            revision: "c2bfbd67dc648cac455077333f7cf5c18a2e3bb4".into(),
            license: "MIT; bundled-codec=Apache-2.0".into(),
        }],
        ..DeploymentConfig::default()
    };
    let model = prepare(
        &path,
        &deployment.models.root,
        true,
        "zerotts_default",
        "zerotts_onnx",
        &deployment,
    )
    .unwrap();
    let _ = fs::remove_file(path);
    compiled_provider_registry()
        .tts_factory("zerotts_onnx")
        .unwrap()
        .build(
            phase5_zerotts_config(),
            &RuntimeConfig {
                onnx: OnnxRuntimeConfig { library: runtime },
            },
            &model,
        )
        .unwrap()
}

fn phase5_zerotts_config() -> &'static voice_agent_server::config::ZeroTtsOnnxConfig {
    static CONFIG: std::sync::OnceLock<voice_agent_server::config::ZeroTtsOnnxConfig> =
        std::sync::OnceLock::new();
    CONFIG.get_or_init(|| voice_agent_server::config::ZeroTtsOnnxConfig {
        model: "zerotts_default".into(),
        num_threads: 1,
        voice: "maichi".into(),
        delivery_mode: Default::default(),
    })
}

async fn start(tts: Arc<dyn TtsProvider>) -> (String, JoinHandle<()>) {
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
        mcp: voice_agent_server::config::McpConfig::default(),
        agent: None,
        effective_agent: voice_agent_server::config::EffectiveAgentConfig::default(),
    };
    let providers = Arc::new(ProviderSet::with_all(
        Arc::new(SpeechThenSilenceVad),
        Arc::new(TwoTurnAsr(Arc::new(AtomicUsize::new(0)))),
        Arc::new(DeterministicLlm),
        tts,
    ));
    let app: Router = router_with_providers(config, providers);
    let task = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (format!("ws://{address}/voice/v1/"), task)
}

#[tokio::test]
async fn real_zerotts_reference_client_barge_in_gate_delivers_b_without_stale_a() {
    let tts = real_tts();
    for mode in [BargeInMode::Auto, BargeInMode::Realtime] {
        let (websocket_url, server) = start(Arc::clone(&tts)).await;
        let report = voice_reference_client::run_barge_in(BargeInRequest {
            websocket_url,
            device_id: "phase5-gate".into(),
            client_id: "reference-client".into(),
            mode,
            uplink_a: UPLINK_A.into_iter().map(Vec::from).collect(),
            uplink_b: UPLINK_B.into_iter().map(Vec::from).collect(),
            expected_b_stt: "utterance B".into(),
            expected_b_llm: "Phase 5 response for utterance B.".into(),
            config: ReferenceClientConfig::default(),
        })
        .await
        .unwrap();
        assert_eq!(report.interruption_stops, 1);
        assert!(report.b_stt_messages > 0, "report: {report:?}");
        assert!(report.b_llm_messages > 0, "report: {report:?}");
        assert!(report.b_binary_packets > 0, "report: {report:?}");
        server.abort();
    }
}
