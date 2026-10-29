//! Local model and voice metadata for the native VieNeu adapter.

use super::{
    assets,
    config::{NANO_MODEL_ID, TURBO_MODEL_ID, VieneuModel},
};
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
        BootstrapCapabilityInspector, DiscoveredCapabilities, ModelSelection, ProviderInspectError,
    },
    registry::ProviderAdapterRegistration,
};
use serde_json::Value;

const MODELS: &[ModelOption] = &[
    ModelOption {
        id: TURBO_MODEL_ID,
        name: "VieNeu v3 Turbo (CPU FP32, 48kHz Streaming)",
        description: Some(
            "Autoregressive LLM + MOSS Codec, 25 preset voices, 48 kHz frame streaming.",
        ),
    },
    ModelOption {
        id: NANO_MODEL_ID,
        name: "VieNeu v3 Nano (CPU FP32, 24kHz Flow-Matching)",
        description: Some(
            "Ultra-lightweight 48M OT-CFM model, 11 preset voices, 24 kHz native output.",
        ),
    },
];
const LANGUAGES: &[LanguageOption] = &[LanguageOption {
    id: "vi-VN",
    name: "Vietnamese",
}];

const fn turbo_voice_options() -> [VoiceOption; assets::TURBO_VOICES.len()] {
    let mut options = [VoiceOption {
        id: "",
        name: "",
        languages: &[],
        model: None,
    }; assets::TURBO_VOICES.len()];
    let mut i = 0;
    while i < assets::TURBO_VOICES.len() {
        let voice = &assets::TURBO_VOICES[i];
        options[i] = VoiceOption {
            id: voice.id,
            name: voice.name,
            languages: &["vi-VN"],
            model: Some(TURBO_MODEL_ID),
        };
        i += 1;
    }
    options
}

const fn nano_voice_options() -> [VoiceOption; assets::NANO_VOICES.len()] {
    let mut options = [VoiceOption {
        id: "",
        name: "",
        languages: &[],
        model: None,
    }; assets::NANO_VOICES.len()];
    let mut i = 0;
    while i < assets::NANO_VOICES.len() {
        let voice = &assets::NANO_VOICES[i];
        options[i] = VoiceOption {
            id: voice.id,
            name: voice.name,
            languages: &["vi-VN"],
            model: Some(NANO_MODEL_ID),
        };
        i += 1;
    }
    options
}

const fn all_voice_options() -> [VoiceOption; assets::TURBO_VOICES.len() + assets::NANO_VOICES.len()]
{
    let mut options = [VoiceOption {
        id: "",
        name: "",
        languages: &[],
        model: None,
    }; assets::TURBO_VOICES.len() + assets::NANO_VOICES.len()];
    let turbo = turbo_voice_options();
    let nano = nano_voice_options();
    let mut i = 0;
    while i < turbo.len() {
        options[i] = turbo[i];
        i += 1;
    }
    let mut j = 0;
    while j < nano.len() {
        options[turbo.len() + j] = nano[j];
        j += 1;
    }
    options
}

const TURBO_VOICES: &[VoiceOption] = &turbo_voice_options();
const NANO_VOICES: &[VoiceOption] = &nano_voice_options();
const VOICES: &[VoiceOption] = &all_voice_options();
const FIELDS: &[ProviderConfigField] = &[
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
        "Speed (%) — Nano only",
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
    adapter: "vieneu",
    provider_type: ProviderType::Tts,
    display_name: "VieNeu v3 Turbo Native",
    description: "Offline Vietnamese TTS via ONNX Runtime (CPU FP32).",
    config_schema: ProviderConfigSchema { fields: FIELDS },
    capabilities: ProviderCapabilities {
        models: Some(MODELS),
        voices: Some(VOICES),
        languages: Some(LANGUAGES),
        streaming: Some(true),
        offline: Some(true),
        tool_calling: None,
        vision: None,
        input_sample_rates: None,
        channels: Some(&[1]),
        provider_output_sample_rates: Some(&[24_000, 48_000]),
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
        let parsed: ModelSelection = serde_json::from_value(selection.clone())
            .map_err(|_| ProviderInspectError::InvalidSelection)?;
        let model = match parsed.model.as_deref() {
            None => VieneuModel::V3TurboFp32,
            Some(id) if id.len() <= 128 => {
                VieneuModel::parse(id).ok_or(ProviderInspectError::InvalidSelection)?
            }
            Some(_) => return Err(ProviderInspectError::InvalidSelection),
        };
        let voices = match model {
            VieneuModel::V3TurboFp32 => TURBO_VOICES,
            VieneuModel::V3NanoFp32 => NANO_VOICES,
        };
        Ok(DiscoveredCapabilities {
            models: MODELS,
            voices,
            languages: LANGUAGES,
        })
    }
}
static INSPECTOR: Inspector = Inspector;
pub static REGISTRATION: ProviderAdapterRegistration =
    ProviderAdapterRegistration::local(&DESCRIPTOR, Some(&INSPECTOR), Some(assets::ASSETS));

#[cfg(test)]
#[path = "descriptor_tests.rs"]
mod tests;
