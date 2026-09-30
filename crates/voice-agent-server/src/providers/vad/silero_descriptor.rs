use crate::providers::{
    capabilities::{
        CapabilityDiscoveryMode, CapabilitySource, DiscoverySource, ModelOption,
        ProviderCapabilities,
    },
    descriptor::{ConfigFieldType, ProviderConfigSchema, ProviderDescriptor, ProviderType, field},
    registry::ProviderAdapterRegistration,
};

const MODELS: &[ModelOption] = &[ModelOption {
    id: "silero_vad_v5",
    name: "Silero VAD v5",
    description: None,
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
];
pub static DESCRIPTOR: ProviderDescriptor = ProviderDescriptor {
    adapter: "silero_onnx",
    provider_type: ProviderType::Vad,
    display_name: "Silero ONNX",
    description: "Local Silero voice activity detector.",
    config_schema: ProviderConfigSchema { fields: FIELDS },
    capabilities: ProviderCapabilities {
        models: Some(MODELS),
        voices: None,
        languages: None,
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
        languages: DiscoverySource::Unsupported,
    },
};
pub static REGISTRATION: ProviderAdapterRegistration = ProviderAdapterRegistration {
    descriptor: &DESCRIPTOR,
    bootstrap_inspector: None,
};
