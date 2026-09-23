use std::net::SocketAddr;

use std::sync::Arc;
use url::Url;
use voice_agent_server::config::{
    AppConfig, AudioConfig, AuthConfig, BargeInConfig, DeploymentConfig, EffectiveAgentConfig,
    LimitsConfig, LlmConfig, ProvidersConfig, RuntimeConfig, ServerConfig, SpeechOutputConfig,
    TtsConfig, WebsocketConfig, WorkersConfig,
};
use voice_agent_server::{
    app::AppState,
    providers::{LlmProvider, ProviderSet, TtsProvider},
};

struct FakeLlm;

impl LlmProvider for FakeLlm {
    fn adapter(&self) -> &'static str {
        "fake_llm"
    }
}

struct FakeTts;

impl TtsProvider for FakeTts {
    fn adapter(&self) -> &'static str {
        "fake_tts"
    }
}

fn valid_config() -> AppConfig {
    AppConfig {
        server: ServerConfig {
            bind: "127.0.0.1:8000".parse::<SocketAddr>().unwrap(),
            public_ws_url: Url::parse("ws://127.0.0.1:8000/voice/v1/").unwrap(),
            hello_timeout_ms: 5_000,
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
        barge_in: BargeInConfig::default(),
        mcp: voice_agent_server::config::McpConfig::default(),
        agent: None,
        effective_agent: EffectiveAgentConfig::default(),
    }
}

#[test]
fn audio_limit_is_converted_to_an_exact_frame_capacity() {
    let config = valid_config();
    assert_eq!(config.max_capture_frames(), 500);
    assert!(config.validate().is_ok());
}

#[test]
fn phase_three_defaults_pin_local_silero_vad_and_zipformer_asr() {
    let providers = ProvidersConfig::default();

    assert_eq!(providers.vad.adapter, "silero_onnx");
    assert_eq!(providers.vad.silero_onnx.unwrap().model, "silero_vad_v5");
    assert_eq!(providers.asr.adapter, "zipformer_sherpa");
    assert_eq!(
        providers.asr.zipformer_sherpa.unwrap().model,
        "zipformer_vi_streaming"
    );
}

#[test]
fn invalid_audio_and_transport_limits_fail_fast() {
    let mut config = valid_config();
    config.audio.max_utterance_ms = 1_000;
    assert!(config.validate().is_err());

    let mut config = valid_config();
    config.websocket.max_frame_bytes = 3_999;
    assert!(config.validate().is_err());
}

#[test]
fn worker_runtime_limits_and_timeouts_are_configured_and_validated() {
    let mut config = valid_config();
    config.workers.asr.max_workers = 3;
    config.workers.asr.command_queue_capacity = 5;
    config.workers.asr.final_timeout_ms = 1_200;
    config.workers.asr.cleanup_grace_ms = 300;
    config.workers.vad.max_workers = 2;
    config.workers.vad.command_queue_capacity = 4;
    config.workers.vad.reset_timeout_ms = 800;
    config.workers.vad.cleanup_grace_ms = 250;
    assert!(config.validate().is_ok());

    config.workers.asr.final_timeout_ms = 0;
    assert!(config.validate().is_err());

    config.workers.asr.final_timeout_ms = 1;
    config.workers.vad.max_workers = 0;
    assert!(config.validate().is_err());
}

#[test]
fn phase_four_delivery_capacity_and_speech_bounds_fail_fast() {
    let mut config = valid_config();
    config.limits.llm_concurrency = 2;
    config.limits.tts_concurrency = 2;
    config.workers.tts.max_workers = 2;
    config.workers.tts.command_queue_capacity = 4;
    config.workers.tts.cleanup_grace_ms = 250;
    config.tts.timeout_ms = 1_000;
    config.speech_output.min_chars = 24;
    config.speech_output.soft_break_min_chars = 48;
    config.speech_output.max_chars = 160;
    config.speech_output.pending_segments = 8;
    assert!(config.validate().is_ok());

    config.limits.tts_concurrency = 1;
    assert!(config.validate().is_err());
    config.limits.tts_concurrency = 2;
    config.speech_output.soft_break_min_chars = 23;
    assert!(config.validate().is_err());
}

#[test]
fn local_http_llm_endpoint_is_allowed_but_remote_http_is_rejected() {
    let mut config = valid_config();
    config.providers.llm.openai.as_mut().unwrap().base_url =
        Url::parse("http://localhost:20128/v1").unwrap();
    assert!(config.validate().is_ok());

    config.providers.llm.openai.as_mut().unwrap().base_url =
        Url::parse("http://example.test/v1").unwrap();
    assert!(config.validate().is_err());
}

#[test]
fn application_state_preserves_injected_llm_and_tts_test_providers() {
    let providers = ProviderSet::with_llm_tts(Arc::new(FakeLlm), Arc::new(FakeTts));

    let state = AppState::new(valid_config(), Arc::new(providers));

    assert_eq!(state.providers.llm_adapter(), "fake_llm");
    assert_eq!(state.providers.tts_adapter(), "fake_tts");
}

#[test]
fn application_builds_each_worker_runtime_from_its_own_config() {
    let mut config = valid_config();
    config.workers.asr.max_workers = 3;
    config.workers.asr.command_queue_capacity = 5;
    config.workers.asr.final_timeout_ms = 1_200;
    config.workers.asr.cleanup_grace_ms = 300;
    config.workers.vad.max_workers = 2;
    config.workers.vad.command_queue_capacity = 4;
    config.workers.vad.reset_timeout_ms = 800;
    config.workers.vad.cleanup_grace_ms = 250;

    let state = AppState::new(config, Arc::new(ProviderSet::unavailable()));
    let asr = state.asr_runtime.runtime_config();
    let vad = state.vad_runtime.runtime_config();
    assert_eq!(asr.max_workers, 3);
    assert_eq!(asr.command_capacity, 5);
    assert_eq!(asr.final_timeout.as_millis(), 1_200);
    assert_eq!(asr.cleanup_grace.as_millis(), 300);
    assert_eq!(vad.max_workers, 2);
    assert_eq!(vad.command_capacity, 4);
    assert_eq!(vad.final_timeout.as_millis(), 800);
    assert_eq!(vad.cleanup_grace.as_millis(), 250);
}

#[test]
fn application_rejects_missing_local_model_artifacts_before_binding() {
    assert!(voice_agent_server::app::application(valid_config()).is_err());
}
