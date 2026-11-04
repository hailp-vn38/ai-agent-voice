use crate::providers::VadError;

#[derive(Debug, Clone, PartialEq)]
pub struct VadInput {
    pub pcm: Vec<f32>,
    pub start_sample: u64,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VadProbability {
    pub start_sample: u64,
    pub end_sample: u64,
    pub probability: f32,
}

pub trait VadProvider: Send + Sync {
    fn open(&self) -> Result<Box<dyn VadSession>, VadError>;
    fn adapter(&self) -> &'static str;
}

/// Mutable VAD state belongs to a worker for one Auto listening cycle.
pub trait VadSession: Send {
    /// Each call is exactly one canonical 512-sample, 16 kHz inference frame.
    fn push(&mut self, input: VadInput) -> Result<VadProbability, VadError>;
    fn reset(&mut self) -> Result<(), VadError>;
    fn close(&mut self) -> Result<(), VadError> {
        Ok(())
    }
}
