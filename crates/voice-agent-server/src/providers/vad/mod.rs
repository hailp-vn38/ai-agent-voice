mod silero_onnx;

pub(crate) use silero_onnx::initialize_ort;
mod traits;

pub use silero_onnx::verify_onnx_runtime;
pub(crate) use silero_onnx::{LoadedSileroVad, UnavailableVad};
pub use traits::{VadInput, VadProbability, VadProvider, VadSession};
