use crate::providers::{
    capabilities::{CapabilityDiscoveryMode, DiscoverySource, ModelOption, ProviderCapabilities},
    descriptor::{ProviderConfigSchema, ProviderDescriptor, ProviderType},
    registry::ProviderAdapterRegistration,
};

const MODELS: &[ModelOption] = &[ModelOption {
    id: "silero_vad_v5",
    name: "Silero VAD v5",
    description: None,
}];
const FIELDS: &[crate::providers::descriptor::ProviderConfigField] = &[];
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
pub static REGISTRATION: ProviderAdapterRegistration =
    ProviderAdapterRegistration::local(&DESCRIPTOR, None, None);
