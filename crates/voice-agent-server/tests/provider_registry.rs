use std::collections::HashSet;

use voice_agent_server::{
    config::AppConfig,
    providers::{ProviderType, compiled_provider_adapter_registry, compiled_provider_registry},
};

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
        registry.vision_factory("openai_vision").unwrap().adapter(),
        "openai_vision"
    );
    assert_eq!(
        registry.tts_factory("zerotts_onnx").unwrap().adapter(),
        "zerotts_onnx"
    );
    assert_eq!(
        registry.tts_factory("chillaudio_ws").unwrap().adapter(),
        "chillaudio_ws"
    );
    assert_eq!(
        registry.tts_factory("kokoro_vi_onnx").unwrap().adapter(),
        "kokoro_vi_onnx"
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

#[test]
fn admin_adapter_descriptors_are_bounded_unique_and_cover_the_active_tts_adapters() {
    let descriptors: Vec<_> = compiled_provider_adapter_registry().list(None).collect();
    let adapters: HashSet<_> = descriptors
        .iter()
        .map(|descriptor| descriptor.adapter)
        .collect();
    assert_eq!(adapters.len(), descriptors.len());
    assert!(adapters.contains("zerotts_onnx"));
    assert!(adapters.contains("chillaudio_ws"));
    assert!(adapters.contains("kokoro_vi_onnx"));
    let factory_adapters: HashSet<_> = compiled_provider_registry()
        .admin_adapters()
        .map(|(_, adapter)| adapter)
        .collect();
    assert_eq!(adapters, factory_adapters);

    for descriptor in descriptors {
        assert!(descriptor.adapter.len() <= 64);
        assert!(descriptor.display_name.len() <= 128);
        assert!(descriptor.description.len() <= 2_048);
        assert!(descriptor.config_schema.fields.len() <= 64);
        let keys: HashSet<_> = descriptor
            .config_schema
            .fields
            .iter()
            .map(|field| field.key)
            .collect();
        assert_eq!(keys.len(), descriptor.config_schema.fields.len());
        let models = descriptor.capabilities.models.unwrap_or(&[]);
        assert!(models.len() <= 128);
        for model in models {
            assert!(model.id.len() <= 128);
            assert!(model.name.len() <= 128);
            assert!(
                model
                    .description
                    .is_none_or(|description| description.len() <= 2_048)
            );
        }
        let languages = descriptor.capabilities.languages.unwrap_or(&[]);
        assert!(languages.len() <= 64);
        for language in languages {
            assert!(language.id.len() <= 32);
            assert!(language.name.len() <= 128);
        }
        let voices = descriptor.capabilities.voices.unwrap_or(&[]);
        assert!(voices.len() <= 256);
        for voice in voices {
            assert!(voice.id.len() <= 128);
            assert!(voice.name.len() <= 128);
            assert!(
                voice
                    .model
                    .is_none_or(|model| models.iter().any(|item| item.id == model))
            );
            assert!(
                voice
                    .languages
                    .iter()
                    .all(|language| languages.iter().any(|item| item.id == *language))
            );
        }
        let registry = compiled_provider_registry();
        match descriptor.provider_type {
            ProviderType::Vad => assert!(registry.vad_factory(descriptor.adapter).is_ok()),
            ProviderType::Asr => assert!(registry.asr_factory(descriptor.adapter).is_ok()),
            ProviderType::Llm => assert!(registry.llm_factory(descriptor.adapter).is_ok()),
            ProviderType::Tts => assert!(registry.tts_factory(descriptor.adapter).is_ok()),
        }
    }
}
