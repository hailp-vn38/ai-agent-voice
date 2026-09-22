use std::net::SocketAddr;

use url::Url;
use voice_agent_server::{
    config::{
        AppConfig, AudioConfig, AuthConfig, DeploymentConfig, LimitsConfig, LlmConfig,
        ProvidersConfig, RuntimeConfig, ServerConfig, WebsocketConfig, WorkersConfig,
    },
    providers::compiled_provider_registry,
};

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
fn registry_exposes_only_adapters_compiled_into_the_binary() {
    let registry = compiled_provider_registry();

    assert_eq!(
        registry.vad_factory("silero_onnx").unwrap().adapter(),
        "silero_onnx"
    );
    assert_eq!(
        registry.asr_factory("zipformer_sherpa").unwrap().adapter(),
        "zipformer_sherpa"
    );
    assert!(registry.vad_factory("http_vad").is_err());
    assert!(registry.asr_factory("python_sidecar").is_err());
}

#[test]
fn startup_validation_rejects_an_adapter_not_compiled_into_the_binary() {
    let mut config = valid_config();
    config.providers.vad.adapter = "http_vad".into();

    let error = config.validate().unwrap_err().to_string();

    assert!(error.contains("not compiled into this binary"));
}

#[test]
fn typed_provider_tables_select_compiled_adapters_and_keep_runtime_options_scoped() {
    let config: AppConfig = toml::from_str(
        r#"
[server]
bind = "127.0.0.1:8000"
public_ws_url = "ws://127.0.0.1:8000/voice/v1/"

[providers.vad]
adapter = "silero_onnx"

[providers.vad.silero_onnx]
model = "silero_vad_v5"
num_threads = 3
min_speech_ms = 240
end_silence_ms = 720
pre_roll_ms = 360
speech_threshold = 0.6
exit_threshold = 0.4

[providers.asr]
adapter = "zipformer_sherpa"

[providers.asr.zipformer_sherpa]
model = "zipformer_vi_streaming"
num_threads = 4
decoding_method = "modified_beam_search"
"#,
    )
    .unwrap();

    config.validate().unwrap();
    let vad = config.providers.vad.silero_onnx.unwrap();
    assert_eq!(vad.model, "silero_vad_v5");
    assert_eq!(vad.num_threads, 3);
    let asr = config.providers.asr.zipformer_sherpa.unwrap();
    assert_eq!(asr.model, "zipformer_vi_streaming");
    assert_eq!(asr.num_threads, 4);
    assert_eq!(asr.decoding_method, "modified_beam_search");
}

#[test]
fn logical_model_identity_must_be_scoped_to_the_selected_adapter_table() {
    let result: Result<AppConfig, _> = toml::from_str(
        r#"
[server]
bind = "127.0.0.1:8000"
public_ws_url = "ws://127.0.0.1:8000/voice/v1/"

[providers.vad]
adapter = "silero_onnx"
model = "silero_vad_v5"
"#,
    );

    assert!(result.is_err());
}
