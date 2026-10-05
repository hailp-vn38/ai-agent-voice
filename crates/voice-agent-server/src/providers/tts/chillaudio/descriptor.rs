use crate::providers::{
    capabilities::{
        CapabilityDiscoveryMode, CapabilitySource, DiscoverySource, LanguageOption,
        ProviderCapabilities, VoiceOption,
    },
    descriptor::{
        ConfigFieldType, ProviderConfigField, ProviderConfigSchema, ProviderDescriptor,
        ProviderType, field,
    },
    registry::ProviderAdapterRegistration,
};
pub(crate) const VOICES: &[VoiceOption] = &[
    VoiceOption {
        id: "BV421_vivn_streaming",
        name: "Nu nhe nhang",
        languages: &["vi"],
        model: None,
    },
    VoiceOption {
        id: "vi_female_huong",
        name: "Nu tram am",
        languages: &["vi"],
        model: None,
    },
    VoiceOption {
        id: "BV074_streaming",
        name: "Nu ca tinh",
        languages: &["vi"],
        model: None,
    },
    VoiceOption {
        id: "BV075_streaming",
        name: "Nam am ap",
        languages: &["vi"],
        model: None,
    },
];
const LANGUAGES: &[LanguageOption] = &[LanguageOption {
    id: "vi",
    name: "Vietnamese",
}];
const FIELDS: &[ProviderConfigField] = &[
    field(
        "voice",
        "Voice",
        ConfigFieldType::Select,
        true,
        Some(CapabilitySource::Voices),
        None,
        None,
        Some(128),
    ),
    field(
        "language",
        "Language",
        ConfigFieldType::Select,
        false,
        Some(CapabilitySource::Languages),
        None,
        None,
        Some(32),
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
        voices: Some(VOICES),
        languages: Some(LANGUAGES),
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
        voices: DiscoverySource::Static,
        languages: DiscoverySource::Static,
    },
};
pub static REGISTRATION: ProviderAdapterRegistration =
    ProviderAdapterRegistration::remote(&DESCRIPTOR, None);
