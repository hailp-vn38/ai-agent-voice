use crate::providers::{
    capabilities::{CapabilityDiscoveryMode, DiscoverySource, ProviderCapabilities},
    descriptor::{ProviderConfigSchema, ProviderDescriptor, ProviderType},
    registry::ProviderAdapterRegistration,
};
use crate::{
    audio::PcmF32Mono,
    providers::{TtsError, TtsProvider},
};
pub static DESCRIPTOR: ProviderDescriptor = ProviderDescriptor {
    adapter: "qualification_tts",
    provider_type: ProviderType::Tts,
    display_name: "Qualification TTS",
    description: "Deterministic TTS for Mandatory Qualification.",
    config_schema: ProviderConfigSchema { fields: &[] },
    capabilities: ProviderCapabilities {
        models: None,
        voices: None,
        languages: None,
        streaming: Some(true),
        offline: Some(true),
        tool_calling: None,
        vision: None,
        input_sample_rates: None,
        channels: None,
        provider_output_sample_rates: Some(&[24_000]),
        voice_delivery_sample_rates: Some(&[24_000]),
    },
    discovery: CapabilityDiscoveryMode {
        models: DiscoverySource::Unsupported,
        voices: DiscoverySource::Unsupported,
        languages: DiscoverySource::Unsupported,
    },
};
pub static REGISTRATION: ProviderAdapterRegistration =
    ProviderAdapterRegistration::remote(&DESCRIPTOR, None);
pub struct QualificationTts;
impl TtsProvider for QualificationTts {
    fn adapter(&self) -> &'static str {
        "qualification_tts"
    }
    fn synthesize(&self, _: &str) -> Result<PcmF32Mono, TtsError> {
        Ok(PcmF32Mono::new(vec![0.0; 240], 24_000))
    }
}
