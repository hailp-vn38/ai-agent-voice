use crate::providers::{
    capabilities::{
        CapabilityDiscoveryMode, CapabilitySource, DiscoverySource, LanguageOption, ModelOption,
        ProviderCapabilities, VoiceOption,
    },
    descriptor::{
        ConfigFieldType, ProviderConfigField, ProviderConfigSchema, ProviderDescriptor,
        ProviderType, field,
    },
    inspector::{
        BootstrapCapabilityInspector, DiscoveredCapabilities, ProviderInspectError,
        validate_model_selection,
    },
    registry::ProviderAdapterRegistration,
};
use serde_json::Value;
const MODELS: &[ModelOption] = &[ModelOption {
    id: "zerotts_default",
    name: "ZeroTTS Default",
    description: None,
}];
const VIETNAMESE: &[LanguageOption] = &[LanguageOption {
    id: "vi-VN",
    name: "Vietnamese",
}];
const VOICES: &[VoiceOption] = &[
    VoiceOption {
        id: "baotrang",
        name: "Bao Trang",
        languages: &["vi-VN"],
        model: Some("zerotts_default"),
    },
    VoiceOption {
        id: "giahuy",
        name: "Gia Huy",
        languages: &["vi-VN"],
        model: Some("zerotts_default"),
    },
    VoiceOption {
        id: "hamy",
        name: "Ha My",
        languages: &["vi-VN"],
        model: Some("zerotts_default"),
    },
    VoiceOption {
        id: "huuduc",
        name: "Huu Duc",
        languages: &["vi-VN"],
        model: Some("zerotts_default"),
    },
    VoiceOption {
        id: "kimoanh",
        name: "Kim Oanh",
        languages: &["vi-VN"],
        model: Some("zerotts_default"),
    },
    VoiceOption {
        id: "maichi",
        name: "Mai Chi",
        languages: &["vi-VN"],
        model: Some("zerotts_default"),
    },
    VoiceOption {
        id: "quangminh",
        name: "Quang Minh",
        languages: &["vi-VN"],
        model: Some("zerotts_default"),
    },
    VoiceOption {
        id: "tiendat",
        name: "Tien Dat",
        languages: &["vi-VN"],
        model: Some("zerotts_default"),
    },
];
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
        true,
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
    ProviderConfigField {
        key: "delivery_mode",
        label: "Delivery mode",
        field_type: ConfigFieldType::Select,
        required: false,
        nullable: false,
        enum_source: None,
        enum_values: Some(&["stream", "file"]),
        minimum: None,
        maximum: None,
        max_length: None,
        description: None,
    },
];
pub static DESCRIPTOR: ProviderDescriptor = ProviderDescriptor {
    adapter: "zerotts_onnx",
    provider_type: ProviderType::Tts,
    display_name: "ZeroTTS",
    description: "Local streaming Vietnamese text to speech.",
    config_schema: ProviderConfigSchema { fields: FIELDS },
    capabilities: ProviderCapabilities {
        models: Some(MODELS),
        voices: Some(VOICES),
        languages: Some(VIETNAMESE),
        streaming: Some(true),
        offline: Some(true),
        tool_calling: None,
        vision: None,
        input_sample_rates: None,
        channels: Some(&[1]),
        provider_output_sample_rates: Some(&[48_000]),
        voice_delivery_sample_rates: Some(&[24_000]),
    },
    discovery: CapabilityDiscoveryMode {
        models: DiscoverySource::Static,
        voices: DiscoverySource::Static,
        languages: DiscoverySource::Static,
    },
};
struct Inspector;
impl BootstrapCapabilityInspector for Inspector {
    fn inspect(&self, selection: &Value) -> Result<DiscoveredCapabilities, ProviderInspectError> {
        validate_model_selection(selection, "zerotts_default")?;
        Ok(DiscoveredCapabilities {
            models: MODELS,
            voices: VOICES,
            languages: VIETNAMESE,
        })
    }
}
static INSPECTOR: Inspector = Inspector;
pub static REGISTRATION: ProviderAdapterRegistration = ProviderAdapterRegistration {
    descriptor: &DESCRIPTOR,
    bootstrap_inspector: Some(&INSPECTOR),
};
