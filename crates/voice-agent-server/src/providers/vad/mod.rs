mod silero_onnx;
mod traits;

pub(crate) use silero_onnx::{LoadedSileroVad, UnavailableVad};
pub use traits::{VadEvent, VadProvider, VadSession};
