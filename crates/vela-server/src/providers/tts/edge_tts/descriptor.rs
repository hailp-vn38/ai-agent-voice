use crate::providers::{
    capabilities::{
        CapabilityDiscoveryMode, CapabilitySource, DiscoverySource, ProviderCapabilities,
    },
    descriptor::{
        ConfigFieldType, ProviderConfigField, ProviderConfigSchema, ProviderDescriptor,
        ProviderType, field,
    },
    registry::ProviderAdapterRegistration,
};

use super::config::{MAX_PITCH_HZ, MAX_RATE_PERCENT, MIN_PITCH_HZ, MIN_RATE_PERCENT};
use super::voices::{LANGUAGES, VOICES};

const FIELDS: &[ProviderConfigField] = &[
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
        "rate_percent",
        "Speech Rate (%)",
        ConfigFieldType::Integer,
        false,
        None,
        Some(MIN_RATE_PERCENT as i64),
        Some(MAX_RATE_PERCENT as i64),
        None,
    ),
    field(
        "pitch_hz",
        "Voice Pitch (Hz)",
        ConfigFieldType::Integer,
        false,
        None,
        Some(MIN_PITCH_HZ as i64),
        Some(MAX_PITCH_HZ as i64),
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
    adapter: "edge_tts",
    provider_type: ProviderType::Tts,
    display_name: "Microsoft Edge TTS",
    description: "Remote Microsoft Edge Read Aloud neural text-to-speech over WebSocket supporting 142 languages/countries, 322 voices, and rate/pitch controls.",
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
        provider_output_sample_rates: Some(&[24_000]),
        voice_delivery_sample_rates: Some(&[24_000]),
    },
    discovery: CapabilityDiscoveryMode {
        models: DiscoverySource::Unsupported,
        voices: DiscoverySource::Static,
        languages: DiscoverySource::Static,
    },
};

pub static REGISTRATION: ProviderAdapterRegistration =
    ProviderAdapterRegistration::remote(&DESCRIPTOR, None);
