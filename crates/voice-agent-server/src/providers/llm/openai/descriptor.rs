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
        "base_url",
        "Base URL",
        ConfigFieldType::String,
        true,
        None,
        None,
        None,
        Some(2048),
    ),
    field(
        "model",
        "Model",
        ConfigFieldType::String,
        true,
        None,
        None,
        None,
        Some(256),
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
];
pub static DESCRIPTOR: ProviderDescriptor = ProviderDescriptor {
    adapter: "openai",
    provider_type: ProviderType::Llm,
    display_name: "OpenAI-compatible LLM",
    description: "Remote OpenAI-compatible language model.",
    config_schema: ProviderConfigSchema { fields: FIELDS },
    capabilities: ProviderCapabilities {
        models: Some(&[]),
        voices: None,
        languages: None,
        streaming: Some(true),
        offline: Some(false),
        tool_calling: Some(true),
        vision: Some(false),
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
