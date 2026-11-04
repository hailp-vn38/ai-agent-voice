//! Local streaming VAD based on the Silero ONNX graph.

pub mod assets;
pub mod descriptor;
pub(crate) mod provider;

pub use descriptor::{DESCRIPTOR, REGISTRATION};
pub use provider::verify_onnx_runtime;
pub(crate) use provider::{LoadedSileroVad, UnavailableVad, initialize_ort};
