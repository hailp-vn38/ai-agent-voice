use std::net::SocketAddr;

use std::sync::Arc;
use url::Url;
use voice_agent_server::config::{
    AppConfig, AudioConfig, AuthConfig, DeploymentConfig, LimitsConfig, LlmConfig, ProvidersConfig,
    RuntimeConfig, ServerConfig, WebsocketConfig, WorkersConfig,
};
use voice_agent_server::{app::AppState, providers::ProviderSet};

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
    assert_eq!(providers.vad.model, "silero_vad_v5");
    assert_eq!(providers.asr.adapter, "zipformer_sherpa");
    assert_eq!(providers.asr.model, "zipformer_vi_streaming");
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
