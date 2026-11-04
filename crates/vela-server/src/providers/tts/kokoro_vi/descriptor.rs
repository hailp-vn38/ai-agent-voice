use super::assets;
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

const MODEL_ID: &str = "kokoro_vi_contextbox";
const MODELS: &[ModelOption] = &[ModelOption {
    id: MODEL_ID,
    name: "Kokoro Vietnamese (ContextBox)",
    description: None,
}];
const VIETNAMESE: &[LanguageOption] = &[LanguageOption {
    id: "vi-VN",
    name: "Vietnamese",
}];
/// Voice metadata is projected from the asset catalog, so a voice the Admin API advertises is
/// exactly a voice whose voicepack the provider installs. Built at compile time to keep the
/// descriptor a plain static; there is no second list to keep in step.
const fn voice_options() -> [VoiceOption; assets::VOICES.len()] {
    let mut options = [VoiceOption {
        id: "",
        name: "",
        languages: &[],
        model: None,
    }; assets::VOICES.len()];
    let mut index = 0;
    while index < assets::VOICES.len() {
        let voice = &assets::VOICES[index];
        options[index] = VoiceOption {
            id: voice.id,
            name: voice.name,
            languages: &["vi-VN"],
            model: Some(MODEL_ID),
        };
        index += 1;
    }
    options
}

const VOICES: &[VoiceOption] = &voice_options();

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
        validate_model_selection(selection, MODEL_ID)?;
        Ok(DiscoveredCapabilities {
            models: MODELS,
            voices: VOICES,
            languages: VIETNAMESE,
        })
    }
}
static INSPECTOR: Inspector = Inspector;
pub static REGISTRATION: ProviderAdapterRegistration =
    ProviderAdapterRegistration::local(&DESCRIPTOR, Some(&INSPECTOR), Some(super::assets::ASSETS));
