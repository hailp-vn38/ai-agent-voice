use crate::providers::{
    capabilities::{
        CapabilityDiscoveryMode, CapabilitySource, DiscoverySource, LanguageOption, ModelOption,
        ProviderCapabilities,
    },
    descriptor::{ConfigFieldType, ProviderConfigSchema, ProviderDescriptor, ProviderType, field},
    registry::ProviderAdapterRegistration,
};
const MODELS: &[ModelOption] = &[ModelOption {
    id: "zipformer_vi_streaming",
    name: "Zipformer Vietnamese Streaming",
    description: None,
}];
const VIETNAMESE: &[LanguageOption] = &[LanguageOption {
    id: "vi-VN",
    name: "Vietnamese",
}];
const FIELDS: &[crate::providers::descriptor::ProviderConfigField] = &[
    field(
        "model",
        "Model",
        ConfigFieldType::Select,
        true,
        Some(CapabilitySource::Models),
        None,
        None,
        Some(128),
    ),
    field(
        "num_threads",
        "Threads",
        ConfigFieldType::Integer,
        true,
        None,
        Some(1),
        Some(128),
        None,
    ),
    field(
        "decoding_method",
        "Decoding method",
        ConfigFieldType::String,
        true,
        None,
        None,
        None,
        Some(64),
    ),
];
pub static DESCRIPTOR: ProviderDescriptor = ProviderDescriptor {
    adapter: "zipformer_sherpa",
    provider_type: ProviderType::Asr,
    display_name: "Zipformer Sherpa",
    description: "Local streaming Vietnamese speech recognition.",
    config_schema: ProviderConfigSchema { fields: FIELDS },
    capabilities: ProviderCapabilities {
        models: Some(MODELS),
        voices: None,
        languages: Some(VIETNAMESE),
        streaming: Some(true),
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
        languages: DiscoverySource::Static,
    },
};
pub static REGISTRATION: ProviderAdapterRegistration = ProviderAdapterRegistration {
    descriptor: &DESCRIPTOR,
    bootstrap_inspector: None,
};
