use crate::providers::{
    capabilities::{CapabilityDiscoveryMode, DiscoverySource, ProviderCapabilities},
    descriptor::{
        ConfigFieldType, ProviderConfigField, ProviderConfigSchema, ProviderDescriptor,
        ProviderType, field,
    },
    registry::ProviderAdapterRegistration,
};
const FIELDS: &[ProviderConfigField] = &[
    field(
        "ws_url",
        "WebSocket URL",
        ConfigFieldType::String,
        true,
        None,
        None,
        None,
        Some(2048),
    ),
    field(
        "voice",
        "Voice",
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
    adapter: "chillaudio_ws",
    provider_type: ProviderType::Tts,
    display_name: "ChillAudio WebSocket",
    description: "Remote ChillAudio text to speech over WebSocket.",
    config_schema: ProviderConfigSchema { fields: FIELDS },
    capabilities: ProviderCapabilities {
        models: None,
        voices: None,
        languages: None,
        streaming: Some(false),
        offline: Some(false),
        tool_calling: None,
        vision: None,
        input_sample_rates: None,
        channels: None,
        provider_output_sample_rates: None,
        voice_delivery_sample_rates: None,
    },
    discovery: CapabilityDiscoveryMode {
        models: DiscoverySource::Unsupported,
        voices: DiscoverySource::Unsupported,
        languages: DiscoverySource::Unsupported,
    },
};
pub static REGISTRATION: ProviderAdapterRegistration = ProviderAdapterRegistration {
    descriptor: &DESCRIPTOR,
    bootstrap_inspector: None,
};
