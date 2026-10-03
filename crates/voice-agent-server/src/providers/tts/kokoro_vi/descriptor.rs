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
    id: "kokoro_vi_contextbox",
    name: "Kokoro Vietnamese (ContextBox)",
    description: None,
}];
const VIETNAMESE: &[LanguageOption] = &[LanguageOption {
    id: "vi-VN",
    name: "Vietnamese",
}];
const VOICES: &[VoiceOption] = &[
    VoiceOption {
        id: "diem_trinh",
        name: "Diem Trinh",
        languages: &["vi-VN"],
        model: Some("kokoro_vi_contextbox"),
    },
    VoiceOption {
        id: "hung_thinh",
        name: "Hung Thinh",
        languages: &["vi-VN"],
        model: Some("kokoro_vi_contextbox"),
    },
    VoiceOption {
        id: "mai_linh",
        name: "Mai Linh",
        languages: &["vi-VN"],
        model: Some("kokoro_vi_contextbox"),
    },
    VoiceOption {
        id: "mai_loan",
        name: "Mai Loan",
        languages: &["vi-VN"],
        model: Some("kokoro_vi_contextbox"),
    },
    VoiceOption {
        id: "manh_dung",
        name: "Manh Dung",
        languages: &["vi-VN"],
        model: Some("kokoro_vi_contextbox"),
    },
    VoiceOption {
        id: "my_yen",
        name: "My Yen",
        languages: &["vi-VN"],
        model: Some("kokoro_vi_contextbox"),
    },
    VoiceOption {
        id: "ngoc_huyen",
        name: "Ngoc Huyen",
        languages: &["vi-VN"],
        model: Some("kokoro_vi_contextbox"),
    },
    VoiceOption {
        id: "phat_tai",
        name: "Phat Tai",
        languages: &["vi-VN"],
        model: Some("kokoro_vi_contextbox"),
    },
    VoiceOption {
        id: "thanh_dat",
        name: "Thanh Dat",
        languages: &["vi-VN"],
        model: Some("kokoro_vi_contextbox"),
    },
    VoiceOption {
        id: "thuc_trinh",
        name: "Thuc Trinh",
        languages: &["vi-VN"],
        model: Some("kokoro_vi_contextbox"),
    },
    VoiceOption {
        id: "tuan_ngoc",
        name: "Tuan Ngoc",
        languages: &["vi-VN"],
        model: Some("kokoro_vi_contextbox"),
    },
    VoiceOption {
        id: "storyvert",
        name: "Storyvert",
        languages: &["vi-VN"],
        model: Some("kokoro_vi_contextbox"),
    },
    VoiceOption {
        id: "duc_an",
        name: "Duc An",
        languages: &["vi-VN"],
        model: Some("kokoro_vi_contextbox"),
    },
    VoiceOption {
        id: "duc_duy",
        name: "Duc Duy",
        languages: &["vi-VN"],
        model: Some("kokoro_vi_contextbox"),
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
        "speed_percent",
        "Speed (%)",
        ConfigFieldType::Integer,
        false,
        None,
        Some(50),
        Some(200),
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
    adapter: "kokoro_vi_onnx",
    provider_type: ProviderType::Tts,
    display_name: "Kokoro Vietnamese ONNX",
    description: "Local Vietnamese Kokoro ONNX text to speech.",
    config_schema: ProviderConfigSchema { fields: FIELDS },
    capabilities: ProviderCapabilities {
        models: Some(MODELS),
        voices: Some(VOICES),
        languages: Some(VIETNAMESE),
        streaming: Some(false),
        offline: Some(true),
        tool_calling: None,
        vision: None,
        input_sample_rates: None,
        channels: Some(&[1]),
        provider_output_sample_rates: Some(&[24_000]),
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
        validate_model_selection(selection, "kokoro_vi_contextbox")?;
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
