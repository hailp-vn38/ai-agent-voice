//! Local inference seams and adapters. Provider code never owns workers or Voice Sessions.

mod catalog;
mod database_loader;
mod error;
mod loader;
mod registry;
mod runtime_catalog;
mod set;

pub mod asr;
pub mod llm;
pub mod tts;
pub mod vad;
pub mod vision;

pub use asr::{AsrEvent, AsrProvider, AsrResult, AsrSession};
pub use catalog::{ProviderCatalog, ProviderLookupError};
pub use database_loader::{
    DatabaseMaterialization, DatabaseRuntimeFailure, DatabaseRuntimeSnapshot, DatabaseRuntimeState,
    DatabaseRuntimeStatus, RequiredProviderUnavailable, materialize_database_providers,
};
pub use error::{AsrError, ProviderLoadError, VadError};
pub use llm::{LlmError, LlmEvent, LlmProvider};
pub(crate) use loader::{LoadedProviders, load_local, vad_timing};

pub use registry::{
    AsrFactory, LlmFactory, ProviderRegistry, TtsFactory, VadFactory, compiled_provider_registry,
};
pub use runtime_catalog::{LoadedVad, ResolvedAgentRuntimes, RuntimeCatalog, RuntimeResolveError};
pub use set::ProviderSet;
pub use tts::{TtsError, TtsProvider, TtsStream, TtsWorker};
pub use vad::{VadInput, VadProbability, VadProvider, VadSession};
pub use vision::{
    OpenAiVisionProvider, VisionError, VisionProvider, VisionRequest, VisionResponse,
};
