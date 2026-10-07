use crate::providers::{
    capabilities::{CapabilityDiscoveryMode, DiscoverySource, ProviderCapabilities},
    descriptor::{ConfigFieldType, ProviderConfigSchema, ProviderDescriptor, ProviderType, field},
    registry::ProviderAdapterRegistration,
};
pub static DESCRIPTOR: ProviderDescriptor = ProviderDescriptor {
    adapter: "campplus_sherpa",
    provider_type: ProviderType::Speaker,
    display_name: "CAM++ Speaker",
    description: "Local speaker embeddings; matching does not grant authorization.",
    config_schema: ProviderConfigSchema {
        fields: &[
            crate::providers::descriptor::ProviderConfigField {
                nullable: true,
                ..field(
                    "calibration_profile",
                    "Calibration profile",
                    ConfigFieldType::String,
                    false,
                    None,
                    None,
                    None,
                    Some(128),
                )
            },
            field(
                "min_speech_ms",
                "Minimum speech (ms)",
                ConfigFieldType::Integer,
                false,
                None,
                Some(1),
                Some(30000),
                None,
            ),
            field(
                "target_speech_ms",
                "Target speech (ms)",
                ConfigFieldType::Integer,
                false,
                None,
                Some(1),
                Some(30000),
                None,
            ),
            field(
                "max_window_ms",
                "Maximum window (ms)",
                ConfigFieldType::Integer,
                false,
                None,
                Some(1),
                Some(6000),
                None,
            ),
        ],
    },
    capabilities: ProviderCapabilities {
        models: None,
        voices: None,
        languages: None,
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
        models: DiscoverySource::Unsupported,
        voices: DiscoverySource::Unsupported,
        languages: DiscoverySource::Unsupported,
    },
};
pub static REGISTRATION: ProviderAdapterRegistration =
    ProviderAdapterRegistration::local(&DESCRIPTOR, None, Some(&super::assets::ASSETS));
