//! Local inference seams and adapters. Provider code never owns workers or Voice Sessions.

mod capabilities;
mod error;
mod registry;
mod set;

pub mod asr;
pub mod vad;

pub use asr::{AsrEvent, AsrProvider, AsrResult, AsrSession};
pub use capabilities::ProviderCapabilities;
pub use error::{AsrError, ProviderLoadError, VadError};
pub use set::ProviderSet;
pub use vad::{VadEvent, VadProvider, VadSession};
