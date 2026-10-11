use crate::providers::{
    capabilities::{
        CapabilityDiscoveryMode, DiscoverySource, LanguageOption, ProviderCapabilities,
    },
    descriptor::{
        ConfigFieldType, ProviderConfigField, ProviderConfigSchema, ProviderDescriptor,
        ProviderType, field,
    },
    registry::ProviderAdapterRegistration,
};

const FIELDS: &[ProviderConfigField] = &[
    field(
        "base_url",
        "Base URL (including /v1)",
        ConfigFieldType::String,
        true,
        None,
        None,
        None,
        Some(2048),
    ),
    field(
        "voice",
        "VieNeu voice name",
        ConfigFieldType::String,
        true,
        None,
        None,
        None,
        Some(128),
    ),
    field(
        "timeout_ms",
        "Timeout (ms)",
        ConfigFieldType::Integer,
        false,
        None,
        Some(1),
        Some(120_000),
        None,
    ),
    field(
        "preload",
        "Preload",
        ConfigFieldType::Boolean,
        false,
        None,
        None,
        None,
        None,
    ),
];

pub static DESCRIPTOR: ProviderDescriptor = ProviderDescriptor {
    adapter: "vieneu_http",
    provider_type: ProviderType::Tts,
    display_name: "VieNeu HTTP Streaming",
    description: "Vietnamese speech from a VieNeu v3 Turbo streaming HTTP service.",
    config_schema: ProviderConfigSchema { fields: FIELDS },
    capabilities: ProviderCapabilities {
        models: None,
        voices: None,
        languages: Some(&[LanguageOption {
            id: "vi-VN",
            name: "Vietnamese",
        }]),
        streaming: Some(true),
        offline: Some(false),
        tool_calling: None,
        vision: None,
        input_sample_rates: None,
        channels: Some(&[1]),
        provider_output_sample_rates: Some(&[24_000]),
        voice_delivery_sample_rates: Some(&[24_000]),
    },
    discovery: CapabilityDiscoveryMode {
        models: DiscoverySource::Unsupported,
        voices: DiscoverySource::Unsupported,
        languages: DiscoverySource::Static,
    },
};

pub static REGISTRATION: ProviderAdapterRegistration =
    ProviderAdapterRegistration::remote(&DESCRIPTOR, None);
