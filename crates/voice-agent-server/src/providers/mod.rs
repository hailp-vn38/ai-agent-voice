//! Local inference seams and adapters. Provider code never owns workers or Voice Sessions.

mod error;
mod loader;
mod registry;
mod set;

pub mod asr;
pub mod vad;

pub use asr::{AsrEvent, AsrProvider, AsrResult, AsrSession};
pub use error::{AsrError, ProviderLoadError, VadError};
pub use registry::{AsrFactory, ProviderRegistry, VadFactory, compiled_provider_registry};
pub use set::ProviderSet;
pub use vad::{VadInput, VadProbability, VadProvider, VadSession};
