use serde_json::Value;

use super::{
    assets::ProviderAssetManager,
    descriptor::{ProviderDescriptor, ProviderType},
    inspector::{BootstrapCapabilityInspector, DiscoveredCapabilities, ProviderInspectError},
};

#[derive(Clone, Copy)]
pub struct ProviderAdapterRegistration {
    pub descriptor: &'static ProviderDescriptor,
    pub bootstrap_inspector: Option<&'static dyn BootstrapCapabilityInspector>,
    /// Present only for adapters that load model files from this host. Remote adapters leave it
    /// `None`, which is how the runtime manager knows a provider has nothing to prepare.
    pub assets: Option<&'static dyn ProviderAssetManager>,
}

impl ProviderAdapterRegistration {
    pub const fn local(
        descriptor: &'static ProviderDescriptor,
        bootstrap_inspector: Option<&'static dyn BootstrapCapabilityInspector>,
        assets: Option<&'static dyn ProviderAssetManager>,
    ) -> Self {
        Self {
            descriptor,
            bootstrap_inspector,
            assets,
        }
    }

    pub const fn remote(
        descriptor: &'static ProviderDescriptor,
        bootstrap_inspector: Option<&'static dyn BootstrapCapabilityInspector>,
    ) -> Self {
        Self::local(descriptor, bootstrap_inspector, None)
    }
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
        self.registration(adapter)
            .map(|registration| registration.descriptor)
    }

    /// The full registration for an adapter, including how it obtains its model files.
    pub fn registration(&self, adapter: &str) -> Option<&'static ProviderAdapterRegistration> {
        self.registrations
            .iter()
            .find(|item| item.descriptor.adapter == adapter)
    }

    /// The asset manager an adapter registered, if it loads local model files.
    pub fn assets(&self, adapter: &str) -> Option<&'static dyn ProviderAssetManager> {
        self.registration(adapter).and_then(|item| item.assets)
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
        self.registration(adapter)
            .and_then(|item| item.bootstrap_inspector)
            .ok_or(ProviderInspectError::Unsupported)?
            .inspect(selection)
    }
}

static REGISTRATIONS: &[ProviderAdapterRegistration] = &[
    super::speaker::descriptor::REGISTRATION,
    #[cfg(feature = "qualification-providers")]
    super::speaker::descriptor::QUALIFICATION_REGISTRATION,
    super::vad::silero::descriptor::REGISTRATION,
    super::asr::zipformer::descriptor::REGISTRATION,
    super::asr::gipformer::descriptor::REGISTRATION,
    super::llm::openai::descriptor::REGISTRATION,
    super::tts::zerotts::descriptor::REGISTRATION,
    super::tts::chillaudio::descriptor::REGISTRATION,
    super::tts::kokoro_vi::descriptor::REGISTRATION,
];

pub fn compiled_provider_adapter_registry() -> &'static ProviderAdapterRegistry {
    static REGISTRY: ProviderAdapterRegistry = ProviderAdapterRegistry {
        registrations: REGISTRATIONS,
    };
    &REGISTRY
}
