use crate::providers::{
    capabilities::{CapabilityDiscoveryMode, DiscoverySource, ProviderCapabilities},
    descriptor::{ProviderConfigSchema, ProviderDescriptor, ProviderType},
    registry::ProviderAdapterRegistration,
};
use crate::{
    audio::PcmF32Mono,
    providers::{AsrError, AsrEvent, AsrProvider, AsrResult, AsrSession},
};
pub static DESCRIPTOR: ProviderDescriptor = ProviderDescriptor {
    adapter: "qualification_asr",
    provider_type: ProviderType::Asr,
    display_name: "Qualification ASR",
    description: "Deterministic ASR for Mandatory Qualification.",
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
pub static REGISTRATION: ProviderAdapterRegistration =
    ProviderAdapterRegistration::remote(&DESCRIPTOR, None);
pub struct QualificationAsr;
impl AsrProvider for QualificationAsr {
    fn open(&self) -> Result<Box<dyn AsrSession>, AsrError> {
        Ok(Box::new(Session(false)))
    }
}
struct Session(bool);
impl AsrSession for Session {
    fn push_pcm(&mut self, pcm: &PcmF32Mono) -> Result<Vec<AsrEvent>, AsrError> {
        self.0 |= !pcm.samples().is_empty();
        Ok(Vec::new())
    }
    fn finish(&mut self) -> Result<AsrResult, AsrError> {
        Ok(AsrResult::new(if self.0 {
            "qualification transcript"
        } else {
            ""
        }))
    }
    fn cancel(&mut self) {
        self.0 = false;
    }
    fn reset(&mut self) -> Result<(), AsrError> {
        self.0 = false;
        Ok(())
    }
}
