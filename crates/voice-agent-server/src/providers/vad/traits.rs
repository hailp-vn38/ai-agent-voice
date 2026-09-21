use crate::{audio::PcmF32Mono, providers::VadError};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VadEvent {
    SpeechStart,
    SpeechEnd,
}

pub trait VadProvider: Send + Sync {
    fn open(&self) -> Result<Box<dyn VadSession>, VadError>;
    fn adapter(&self) -> &'static str;
}

/// Mutable VAD state belongs to a worker for one Auto listening cycle.
pub trait VadSession: Send {
    fn push_pcm(&mut self, pcm: &PcmF32Mono) -> Result<Vec<VadEvent>, VadError>;
    fn reset(&mut self) -> Result<(), VadError>;
    fn close(&mut self) -> Result<(), VadError>;
}
