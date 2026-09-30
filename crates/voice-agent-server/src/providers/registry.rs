use serde_json::Value;

use super::{
    descriptor::{ProviderDescriptor, ProviderType},
    inspector::{BootstrapCapabilityInspector, DiscoveredCapabilities, ProviderInspectError},
};

#[derive(Clone, Copy)]
pub struct ProviderAdapterRegistration {
    pub descriptor: &'static ProviderDescriptor,
    pub bootstrap_inspector: Option<&'static dyn BootstrapCapabilityInspector>,
}

pub struct ProviderAdapterRegistry {
    registrations: &'static [ProviderAdapterRegistration],
}

impl ProviderAdapterRegistry {
    pub fn list(
        &self,
        provider_type: Option<ProviderType>,
    ) -> impl Iterator<Item = &'static ProviderDescriptor> {
        self.registrations
            .iter()
            .map(|item| item.descriptor)
            .filter(move |descriptor| {
                provider_type.is_none_or(|kind| descriptor.provider_type == kind)
            })
    }

    pub fn get(&self, adapter: &str) -> Option<&'static ProviderDescriptor> {
        self.registrations
            .iter()
            .find(|item| item.descriptor.adapter == adapter)
            .map(|item| item.descriptor)
    }

    pub fn supports(&self, adapter: &str, provider_type: &str) -> bool {
        self.get(adapter)
            .is_some_and(|descriptor| descriptor.provider_type.as_str() == provider_type)
    }

    pub fn discover(
        &self,
        adapter: &str,
        selection: &Value,
    ) -> Result<DiscoveredCapabilities, ProviderInspectError> {
        self.registrations
            .iter()
            .find(|item| item.descriptor.adapter == adapter)
            .and_then(|item| item.bootstrap_inspector)
            .ok_or(ProviderInspectError::Unsupported)?
            .inspect(selection)
    }
}

static REGISTRATIONS: &[ProviderAdapterRegistration] = &[
    super::vad::silero_descriptor::REGISTRATION,
    super::asr::zipformer::descriptor::REGISTRATION,
    super::asr::gipformer::descriptor::REGISTRATION,
    super::llm::openai::descriptor::REGISTRATION,
    super::tts::zerotts::descriptor::REGISTRATION,
    super::tts::chillaudio::descriptor::REGISTRATION,
];

pub fn compiled_provider_adapter_registry() -> &'static ProviderAdapterRegistry {
    static REGISTRY: ProviderAdapterRegistry = ProviderAdapterRegistry {
        registrations: REGISTRATIONS,
    };
    &REGISTRY
}
