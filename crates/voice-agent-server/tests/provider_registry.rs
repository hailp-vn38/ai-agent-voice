use std::net::SocketAddr;

use url::Url;
use voice_agent_server::{
    config::{
        AppConfig, AudioConfig, AuthConfig, BargeInConfig, DeploymentConfig, LimitsConfig,
        LlmConfig, ProvidersConfig, RuntimeConfig, ServerConfig, SpeechOutputConfig, TtsConfig,
        WebsocketConfig, WorkersConfig,
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
        tts: TtsConfig::default(),
        speech_output: SpeechOutputConfig::default(),
        barge_in: BargeInConfig::default(),
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
    assert_eq!(registry.llm_factory("openai").unwrap().adapter(), "openai");
    assert_eq!(
        registry.tts_factory("zerotts_onnx").unwrap().adapter(),
        "zerotts_onnx"
    );
    assert!(registry.vad_factory("http_vad").is_err());
    assert!(registry.asr_factory("python_sidecar").is_err());
    assert!(registry.llm_factory("python_llm").is_err());
    assert!(registry.tts_factory("http_tts").is_err());
}

#[test]
fn startup_validation_rejects_an_adapter_not_compiled_into_the_binary() {
    let mut config = valid_config();
    config.providers.vad.adapter = "http_vad".into();

    let error = config.validate().unwrap_err().to_string();

    assert!(error.contains("not compiled into this binary"));
}

#[test]
fn startup_validation_rejects_uncompiled_llm_and_tts_adapters_before_bind() {
    let mut config = valid_config();
    config.providers.llm.adapter = "python_llm".into();
    assert!(
        config
            .validate()
            .unwrap_err()
            .to_string()
            .contains("LLM adapter `python_llm` is not compiled")
    );

    let mut config = valid_config();
    config.providers.tts.adapter = "http_tts".into();
    assert!(
        config
            .validate()
            .unwrap_err()
            .to_string()
            .contains("TTS adapter `http_tts` is not compiled")
    );
}

#[test]
fn typed_llm_and_tts_tables_redact_api_keys_and_keep_options_scoped() {
    let config: AppConfig = toml::from_str(
        r#"
[server]
bind = "127.0.0.1:8000"
public_ws_url = "ws://127.0.0.1:8000/voice/v1/"

[providers.vad]
adapter = "silero_onnx"

[providers.asr]
adapter = "zipformer_sherpa"

[providers.llm]
type = "openai"

[providers.llm.openai]
api_key = "secret-do-not-log"
base_url = "https://api.openai.com/v1"
model = "gpt-test"

[providers.tts]
adapter = "zerotts_onnx"

[providers.tts.zerotts_onnx]
model = "zerotts_default"
num_threads = 2
voice = "maichi"
"#,
    )
    .unwrap();

    assert_eq!(config.providers.llm.adapter, "openai");
    assert_eq!(config.providers.tts.adapter, "zerotts_onnx");
    assert!(!format!("{config:?}").contains("secret-do-not-log"));
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
