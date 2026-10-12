//! Local voice metadata. Registration is deferred until real native streaming
//! is implemented and its worker passes the end-to-end model gate.

use super::{assets, config::MODEL_ID};
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
    id: MODEL_ID,
    name: "VieNeu v3 Turbo (CPU FP32)",
    description: None,
}];
const LANGUAGES: &[LanguageOption] = &[LanguageOption {
    id: "vi-VN",
    name: "Vietnamese",
}];

const fn voice_options() -> [VoiceOption; assets::VOICES.len()] {
    let mut options = [VoiceOption {
        id: "",
        name: "",
        languages: &[],
        model: None,
    }; assets::VOICES.len()];
    let mut i = 0;
    while i < assets::VOICES.len() {
        let voice = &assets::VOICES[i];
        options[i] = VoiceOption {
            id: voice.id,
            name: voice.name,
            languages: &["vi-VN"],
            model: Some(MODEL_ID),
        };
        i += 1;
    }
    options
}

const VOICES: &[VoiceOption] = &voice_options();
const FIELDS: &[ProviderConfigField] = &[
    field("voice", "Voice", ConfigFieldType::Select, true,
        Some(CapabilitySource::Voices), None, None, Some(128)),
    field("language", "Language", ConfigFieldType::Select, true,
        Some(CapabilitySource::Languages), None, None, Some(32)),
    field("preload", "Preload", ConfigFieldType::Boolean, false,
        None, None, None, None),
];

pub static DESCRIPTOR: ProviderDescriptor = ProviderDescriptor {
    adapter: "vieneu_native",
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
        validate_model_selection(selection, MODEL_ID)?;
        Ok(DiscoveredCapabilities {
            models: MODELS,
            voices: VOICES,
            languages: LANGUAGES,
        })
    }
}
static INSPECTOR: Inspector = Inspector;
pub static REGISTRATION: ProviderAdapterRegistration =
    ProviderAdapterRegistration::local(&DESCRIPTOR, Some(&INSPECTOR), Some(assets::ASSETS));

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn discovered_voices_exactly_match_asset_catalog() {
        let discovered = INSPECTOR.inspect(&serde_json::json!({
            "model": MODEL_ID,
        })).unwrap();
        let ids: HashSet<_> = discovered.voices.iter().map(|voice| voice.id).collect();
        assert_eq!(ids.len(), assets::VOICES.len());
        for voice in assets::VOICES {
            assert!(ids.contains(voice.id));
        }
    }

    #[test]
    fn refuses_other_model_identity() {
        assert!(INSPECTOR.inspect(&serde_json::json!({
            "model": "vieneu_v4",
        })).is_err());
    }
}
