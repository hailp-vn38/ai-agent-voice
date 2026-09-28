use voice_agent_server::{config::AppConfig, providers::compiled_provider_registry};

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
    assert_eq!(
        registry
            .asr_factory("gipformer_sherpa_offline")
            .unwrap()
            .adapter(),
        "gipformer_sherpa_offline"
    );
    assert_eq!(registry.llm_factory("openai").unwrap().adapter(), "openai");
    assert_eq!(
        registry.tts_factory("zerotts_onnx").unwrap().adapter(),
        "zerotts_onnx"
    );
    assert_eq!(
        registry.tts_factory("chillaudio_ws").unwrap().adapter(),
        "chillaudio_ws"
    );
    assert!(registry.tts_factory("http_tts").is_err());
}

#[test]
fn catalog_config_keeps_secrets_redacted_and_options_scoped_to_each_instance() {
    let config: AppConfig = toml::from_str(
        r#"
[server]
bind = "127.0.0.1:8000"
public_ws_url = "ws://127.0.0.1:8000/voice/v1/"
[provider_defaults]
vad = "vad"
asr = "asr"
llm = "llm"
tts = "tts"
[providers.vad.instances.vad]
adapter = "silero_onnx"
[providers.asr.instances.asr]
adapter = "zipformer_sherpa"
[providers.llm.instances.llm]
adapter = "openai"
api_key = "secret-do-not-log"
model = "gpt-test"
[providers.tts.instances.tts]
adapter = "chillaudio_ws"
token = "secret-do-not-log"
"#,
    )
    .unwrap();
    assert_eq!(config.providers.llm.instances["llm"].adapter(), "openai");
    assert_eq!(
        config.providers.tts.instances["tts"].adapter(),
        "chillaudio_ws"
    );
    assert!(!format!("{config:?}").contains("secret-do-not-log"));
}
