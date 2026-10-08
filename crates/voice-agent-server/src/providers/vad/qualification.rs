use crate::providers::{VadError, VadInput, VadProbability, VadProvider, VadSession};
use crate::providers::{
    capabilities::{CapabilityDiscoveryMode, DiscoverySource, ProviderCapabilities},
    descriptor::{ProviderConfigSchema, ProviderDescriptor, ProviderType},
    registry::ProviderAdapterRegistration,
};
pub static DESCRIPTOR: ProviderDescriptor = ProviderDescriptor {
    adapter: "qualification_vad",
    provider_type: ProviderType::Vad,
    display_name: "Qualification VAD",
    description: "Deterministic VAD for Mandatory Qualification.",
    config_schema: ProviderConfigSchema { fields: &[] },
    capabilities: ProviderCapabilities {
        models: None,
        voices: None,
        languages: None,
        streaming: Some(true),
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

pub struct QualificationVad;
impl VadProvider for QualificationVad {
    fn open(&self) -> Result<Box<dyn VadSession>, VadError> {
        Ok(Box::new(Session))
    }
    fn adapter(&self) -> &'static str {
        "qualification_vad"
    }
}
struct Session;
impl VadSession for Session {
    fn push(&mut self, input: VadInput) -> Result<VadProbability, VadError> {
        if input.pcm.len() != 512 {
            return Err(VadError::Failed(
                "qualification VAD requires 512 samples".into(),
            ));
        }
        Ok(VadProbability {
            start_sample: input.start_sample,
            end_sample: input.start_sample + 512,
            probability: if input.pcm.iter().any(|sample| *sample != 0.0) {
                1.0
            } else {
                0.0
            },
        })
    }
    fn reset(&mut self) -> Result<(), VadError> {
        Ok(())
    }
}
