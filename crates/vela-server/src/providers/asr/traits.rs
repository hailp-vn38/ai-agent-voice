use crate::audio::PcmF32Mono;
use crate::providers::AsrError;

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct AsrTimedSegment {
    pub start_ms: u64,
    pub end_ms: u64,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AsrResult {
    text: String,
    segments: Vec<AsrTimedSegment>,
}

impl AsrResult {
    pub fn new(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            segments: Vec::new(),
        }
    }

    pub fn with_segments(text: impl Into<String>, segments: Vec<AsrTimedSegment>) -> Self {
        Self {
            text: text.into(),
            segments,
        }
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn segments(&self) -> &[AsrTimedSegment] {
        &self.segments
    }

    pub fn into_parts(self) -> (String, Vec<AsrTimedSegment>) {
        (self.text, self.segments)
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
    /// Resets utterance state while retaining the initialized native engine.
    fn reset(&mut self) -> Result<(), AsrError> {
        Err(AsrError::Failed("ASR session reset is unsupported".into()))
    }
}

pub trait AsrProvider: Send + Sync {
    fn open(&self) -> Result<Box<dyn AsrSession>, AsrError>;

    fn adapter(&self) -> &'static str {
        "zipformer_sherpa"
    }
}
