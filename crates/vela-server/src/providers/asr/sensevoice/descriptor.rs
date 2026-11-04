use crate::providers::{
    capabilities::{
        CapabilityDiscoveryMode, CapabilitySource, DiscoverySource, LanguageOption, ModelOption,
        ProviderCapabilities,
    },
    descriptor::{ConfigFieldType, ProviderConfigSchema, ProviderDescriptor, ProviderType, field},
    inspector::{
        BootstrapCapabilityInspector, DiscoveredCapabilities, ProviderInspectError,
        validate_model_selection,
    },
    registry::ProviderAdapterRegistration,
};
use serde_json::Value;

const MODELS: &[ModelOption] = &[ModelOption {
    id: "sensevoice_small_int8",
    name: "SenseVoiceSmall Multilingual INT8",
    description: None,
}];

const LANGUAGES: &[LanguageOption] = &[
    LanguageOption {
        id: "auto",
        name: "Auto Detect",
    },
    LanguageOption {
        id: "en",
        name: "English",
    },
    LanguageOption {
        id: "zh",
        name: "Chinese (Mandarin)",
    },
    LanguageOption {
        id: "ja",
        name: "Japanese",
    },
    LanguageOption {
        id: "ko",
        name: "Korean",
    },
    LanguageOption {
        id: "yue",
        name: "Cantonese",
    },
];

const FIELDS: &[crate::providers::descriptor::ProviderConfigField] = &[
    field(
        "language",
        "Language",
        ConfigFieldType::Select,
        true,
        Some(CapabilitySource::Languages),
        None,
        None,
        Some(32),
    ),
    field(
        "use_itn",
        "Inverse text normalization (ITN)",
        ConfigFieldType::Boolean,
        false,
        None,
        None,
        None,
        None,
    ),
];

pub static DESCRIPTOR: ProviderDescriptor = ProviderDescriptor {
    adapter: "sensevoice_sherpa_offline",
    provider_type: ProviderType::Asr,
    display_name: "SenseVoice Sherpa Offline",
    description: "Local offline multilingual speech recognition (English, Chinese, Japanese, Korean, Cantonese).",
    config_schema: ProviderConfigSchema { fields: FIELDS },
    capabilities: ProviderCapabilities {
        models: Some(MODELS),
        voices: None,
        languages: Some(LANGUAGES),
        streaming: Some(false),
        offline: Some(true),
        tool_calling: None,
        vision: None,
        input_sample_rates: Some(&[16_000]),
        channels: Some(&[1]),
        provider_output_sample_rates: None,
        voice_delivery_sample_rates: None,
    },
    discovery: CapabilityDiscoveryMode {
        models: DiscoverySource::Static,
        voices: DiscoverySource::Unsupported,
        languages: DiscoverySource::Bootstrap,
    },
};

struct Inspector;

impl BootstrapCapabilityInspector for Inspector {
    fn inspect(&self, selection: &Value) -> Result<DiscoveredCapabilities, ProviderInspectError> {
        validate_model_selection(selection, "sensevoice_small_int8")?;
        Ok(DiscoveredCapabilities {
            models: MODELS,
            voices: &[],
            languages: LANGUAGES,
        })
    }
}

static INSPECTOR: Inspector = Inspector;

pub static REGISTRATION: ProviderAdapterRegistration =
    ProviderAdapterRegistration::local(&DESCRIPTOR, Some(&INSPECTOR), Some(super::assets::ASSETS));
