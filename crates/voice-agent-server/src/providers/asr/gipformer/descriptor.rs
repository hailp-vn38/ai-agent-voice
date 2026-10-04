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
    id: "gipformer15_vi_int8",
    name: "Gipformer 1.5 Vietnamese INT8",
    description: None,
}];
const VIETNAMESE: &[LanguageOption] = &[LanguageOption {
    id: "vi-VN",
    name: "Vietnamese",
}];
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
    crate::providers::descriptor::select_field(
        "decoding_method",
        "Decoding method",
        &["greedy_search", "modified_beam_search"],
    ),
    field(
        "max_active_paths",
        "Maximum active paths",
        ConfigFieldType::Integer,
        true,
        None,
        Some(1),
        Some(10_000),
        None,
    ),
];
pub static DESCRIPTOR: ProviderDescriptor = ProviderDescriptor {
    adapter: "gipformer_sherpa_offline",
    provider_type: ProviderType::Asr,
    display_name: "Gipformer Sherpa Offline",
    description: "Local offline Vietnamese speech recognition.",
    config_schema: ProviderConfigSchema { fields: FIELDS },
    capabilities: ProviderCapabilities {
        models: Some(MODELS),
        voices: None,
        languages: Some(VIETNAMESE),
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
        validate_model_selection(selection, "gipformer15_vi_int8")?;
        Ok(DiscoveredCapabilities {
            models: MODELS,
            voices: &[],
            languages: VIETNAMESE,
        })
    }
}
static INSPECTOR: Inspector = Inspector;
pub static REGISTRATION: ProviderAdapterRegistration =
    ProviderAdapterRegistration::local(&DESCRIPTOR, Some(&INSPECTOR), None);
