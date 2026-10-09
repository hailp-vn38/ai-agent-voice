//! Local inference seams and adapters. Provider code never owns workers or Voice Sessions.

pub mod assets;
pub mod capabilities;
mod catalog;
mod database_loader;
mod descriptor;
mod error;
mod factory_registry;
pub mod inspector;
mod loader;
mod local_runtime;
pub mod registry;
mod runtime_catalog;
mod set;

pub mod asr;
pub mod llm;
pub mod speaker;
pub mod tts;
pub mod vad;
pub mod vision;

pub use asr::{AsrEvent, AsrProvider, AsrResult, AsrSession};
pub use assets::{Asset, AssetError, VoiceAsset};
pub use catalog::{ProviderCatalog, ProviderLookupError};
pub use database_loader::{
    DatabaseMaterialization, DatabaseRuntimeFailure, DatabaseRuntimeSnapshot, DatabaseRuntimeState,
    DatabaseRuntimeStatus, RequiredProviderUnavailable, materialize_database_providers,
    materialize_provider, materialize_provider_with_admission,
};
pub use descriptor::{AdapterSummary, ProviderDescriptor, ProviderType};
pub use inspector::{DiscoveredCapabilities, ProviderInspectError};
pub use registry::{ProviderAdapterRegistry, compiled_provider_adapter_registry};

pub fn admin_provider_adapter_matches_kind(kind: &str, adapter: &str) -> bool {
    compiled_provider_adapter_registry().supports(adapter, kind)
}
pub use error::{AsrError, ProviderLoadError, VadError};
pub use llm::{LlmError, LlmEvent, LlmProvider};
pub(crate) use loader::{LoadedProviders, load_local, vad_timing};

pub use factory_registry::{
    AsrFactory, LlmFactory, ProviderRegistry, TtsFactory, VadFactory, compiled_provider_registry,
    local_model_identity,
};
pub use local_runtime::{
    LocalRuntimeAdapter, LocalRuntimeAdapterRegistry, compiled_local_runtime_adapter_registry,
    configured_physical_replicas,
};
pub use runtime_catalog::{
    DiagnosticRuntimeError, DiagnosticRuntimeKind, LoadedVad, ResolvedAgentRuntimes,
    RuntimeCatalog, RuntimeResolveError, TtsDiagnosticValidationError,
};
pub use set::ProviderSet;
pub use tts::{
    TtsBinding, TtsDiagnosticRequest, TtsError, TtsProvider, TtsStream, TtsSynthesisRequest,
    TtsWorker,
};
pub use vad::{VadInput, VadProbability, VadProvider, VadSession};
pub use vision::{
    OpenAiVisionProvider, VisionError, VisionProvider, VisionRequest, VisionResponse,
};

mod deployment_snapshot;
pub use deployment_snapshot::deployment_provider_snapshot;

pub(crate) use database_loader::{ProviderSource, materialize_provider_from_artifacts};

pub(crate) use factory_registry::effective_local_config;
