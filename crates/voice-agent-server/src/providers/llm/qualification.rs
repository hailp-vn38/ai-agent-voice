use crate::providers::{LlmError, LlmProvider, llm::LlmRequest};
use crate::providers::{
    capabilities::{CapabilityDiscoveryMode, DiscoverySource, ProviderCapabilities},
    descriptor::{ProviderConfigSchema, ProviderDescriptor, ProviderType},
    registry::ProviderAdapterRegistration,
};
pub static DESCRIPTOR: ProviderDescriptor = ProviderDescriptor {
    adapter: "qualification_llm",
    provider_type: ProviderType::Llm,
    display_name: "Qualification LLM",
    description: "Deterministic LLM for Mandatory Qualification.",
    config_schema: ProviderConfigSchema { fields: &[] },
    capabilities: ProviderCapabilities {
        models: None,
        voices: None,
        languages: None,
        streaming: Some(true),
        offline: Some(true),
        tool_calling: Some(false),
        vision: Some(false),
        input_sample_rates: None,
        channels: None,
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
pub struct QualificationLlm;
impl LlmProvider for QualificationLlm {
    fn adapter(&self) -> &'static str {
        "qualification_llm"
    }
    fn complete(&self, _: &LlmRequest) -> Result<String, LlmError> {
        Ok("qualification response".into())
    }
}
