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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exposes_only_optional_window_controls_with_server_defaults() {
        let keys: Vec<_> = DESCRIPTOR
            .config_schema
            .fields
            .iter()
            .map(|field| field.key)
            .collect();
        assert_eq!(keys, ["min_speech_ms", "target_speech_ms", "max_window_ms"]);
        let config: crate::config::CampPlusConfig = serde_json::from_str("{}").unwrap();
        assert_eq!(
            (
                config.min_speech_ms,
                config.target_speech_ms,
                config.max_window_ms
            ),
            (2_000, 4_000, 6_000)
        );
    }
}

/// Deterministic, model-free speaker adapter compiled only into the qualification
/// build (ADR 0068). It is remote: there are no assets for the runtime manager to
/// prepare, and it is intentionally absent from the local runtime adapter registry.
#[cfg(feature = "qualification-providers")]
pub static QUALIFICATION_DESCRIPTOR: ProviderDescriptor = ProviderDescriptor {
    adapter: "qualification_speaker",
    provider_type: ProviderType::Speaker,
    display_name: "Qualification Speaker",
    description: "Deterministic speaker embeddings for Mandatory Qualification; never production.",
    config_schema: ProviderConfigSchema { fields: &[] },
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
#[cfg(feature = "qualification-providers")]
pub static QUALIFICATION_REGISTRATION: ProviderAdapterRegistration =
    ProviderAdapterRegistration::remote(&QUALIFICATION_DESCRIPTOR, None);
