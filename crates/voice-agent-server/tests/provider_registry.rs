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
            ProviderType::Speaker => assert!(registry.speaker_factory(descriptor.adapter).is_ok()),
        }
    }
}

#[test]
fn descriptors_expose_only_user_configuration_and_typed_decoding_choices() {
    let registry = compiled_provider_adapter_registry();
    for descriptor in registry.list(None) {
        let json = serde_json::to_value(descriptor).unwrap();
        let fields = json["config_schema"]["fields"].as_array().unwrap();
        assert!(fields.iter().all(|field| !matches!(
            field["key"].as_str(),
            Some("threads" | "num_threads" | "ws_url")
        )));
        if matches!(
            descriptor.adapter,
            "silero_onnx"
                | "zerotts_onnx"
                | "kokoro_vi_onnx"
                | "zipformer_sherpa"
                | "gipformer_sherpa_offline"
        ) {
            assert!(fields.iter().all(|field| field["key"] != "model"));
        }
        if descriptor.adapter == "silero_onnx" {
            assert!(fields.is_empty());
        }
        if matches!(
            descriptor.adapter,
            "zipformer_sherpa" | "gipformer_sherpa_offline"
        ) {
            let decoding = fields
                .iter()
                .find(|field| field["key"] == "decoding_method")
                .unwrap();
            assert_eq!(decoding["type"], "select");
            assert_eq!(
                decoding["enum_values"],
                serde_json::json!(["greedy_search", "modified_beam_search"])
            );
        }
    }
    let chill = registry.get("chillaudio_ws").unwrap();
    assert_eq!(chill.capabilities.voices.unwrap().len(), 4);
    assert_eq!(chill.capabilities.languages.unwrap()[0].id, "vi");
}

/// Every voice a local provider advertises is one it can actually load, because both come from the
/// same asset catalog.
#[test]
fn every_advertised_local_voice_is_an_installed_voice_asset() {
    for (adapter, voices) in [
        (
            "zerotts_onnx",
            voice_agent_server::providers::tts::zerotts::assets::VOICES
                .iter()
                .map(|voice| voice.id)
                .collect::<Vec<_>>(),
        ),
        (
            "kokoro_vi_onnx",
            voice_agent_server::providers::tts::kokoro_vi::assets::VOICES
                .iter()
                .map(|voice| voice.id)
                .collect::<Vec<_>>(),
        ),
    ] {
        let descriptor = compiled_provider_adapter_registry().get(adapter).unwrap();
        let advertised: HashSet<_> = descriptor
            .capabilities
            .voices
            .expect("local TTS advertises voices")
            .iter()
            .map(|voice| voice.id)
            .collect();
        assert_eq!(advertised, voices.iter().copied().collect(), "{adapter}");
    }
}

/// Every local adapter owns its model files, and every remote adapter owns none.
#[test]
fn asset_ownership_follows_whether_a_provider_loads_local_files() {
    for adapter in compiled_provider_adapter_registry()
        .list(None)
        .map(|descriptor| descriptor.adapter)
    {
        let owns_assets = compiled_provider_adapter_registry()
            .assets(adapter)
            .is_some();
        assert_eq!(
            owns_assets,
            !matches!(adapter, "openai" | "openai_vision" | "chillaudio_ws"),
            "{adapter}"
        );
    }
}

/// Each local adapter pins exactly one upstream revision, and it is what resource identity uses.
#[test]
fn every_local_adapter_pins_an_immutable_upstream_revision() {
    for adapter in [
        "silero_onnx",
        "zipformer_sherpa",
        "gipformer_sherpa_offline",
        "zerotts_onnx",
        "kokoro_vi_onnx",
    ] {
        let revision = compiled_provider_adapter_registry()
            .assets(adapter)
            .unwrap_or_else(|| panic!("{adapter} owns model files"))
            .revision();
        assert!(!revision.is_empty(), "{adapter}");
        assert!(
            revision.bytes().all(|byte| byte.is_ascii_hexdigit()),
            "{adapter} pins a content-addressed revision"
        );
    }
}

#[test]
#[ignore = "requires acknowledged installed ASR models; set PROVIDER_QUALIFICATION_CONFIG"]
fn native_transducers_accept_every_advertised_decoding_method() {
    use voice_agent_server::{
        audio::PcmF32Mono,
        config::{AsrInstanceConfig, TransducerDecodingMethod},
    };
    let path = std::env::var("PROVIDER_QUALIFICATION_CONFIG").expect("qualification config");
    let mut config = AppConfig::parse_and_resolve(&path).expect("valid deployment configuration");
    // The model root is an internal constant relative to the working directory, so the gate runs
    // from the deployment directory that holds `models/`.
    let deployment_root = std::path::Path::new(&path).parent().unwrap();
    if config.runtime.onnx.library.is_relative() {
        config.runtime.onnx.library = deployment_root.join(&config.runtime.onnx.library);
    }
    std::env::set_current_dir(deployment_root).expect("enter the deployment directory");
    for adapter in ["zipformer_sherpa", "gipformer_sherpa_offline"] {
        let instance = config
            .providers
            .asr
            .instances
            .values()
            .find(|instance| instance.adapter() == adapter)
            .expect("configured adapter");
        let factory = compiled_provider_registry().asr_factory(adapter).unwrap();
        for method in [
            TransducerDecodingMethod::GreedySearch,
            TransducerDecodingMethod::ModifiedBeamSearch,
        ] {
            let mut selected = instance.clone();
            match &mut selected {
                AsrInstanceConfig::ZipformerSherpa(options) => options.decoding_method = method,
                AsrInstanceConfig::GipformerSherpaOffline(options) => {
                    options.decoding_method = method
                }
            }
            let provider = factory
                .build(&selected, &config.runtime, 480_000)
                .expect("native recognizer accepts decoding mode");
            let mut session = provider.open().unwrap();
            session
                .push_pcm(&PcmF32Mono::new(vec![0.0; 16_000], 16_000))
                .unwrap();
            session.finish().unwrap();
        }
    }
}
