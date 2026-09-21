use crate::audio::PcmF32Mono;
use crate::providers::AsrError;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AsrResult {
    text: String,
}

impl AsrResult {
    pub fn new(text: impl Into<String>) -> Self {
        Self { text: text.into() }
    }
    pub fn text(&self) -> &str {
        &self.text
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AsrEvent {
    Partial(String),
}

pub trait AsrSession: Send {
    fn push_pcm(&mut self, pcm: &PcmF32Mono) -> Result<Vec<AsrEvent>, AsrError>;
    fn finish(&mut self) -> Result<AsrResult, AsrError>;
    fn cancel(&mut self);
}

pub trait AsrProvider: Send + Sync {
    fn open(&self) -> Result<Box<dyn AsrSession>, AsrError>;
}
