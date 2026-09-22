//! TTS provider boundary.

use crate::audio::PcmF32Mono;
use thiserror::Error;

/// Terminal output produced by startup-only ZeroTTS warmup.
#[derive(Debug, Clone, PartialEq)]
pub struct WarmupPcm {
    sample_rate: u32,
    channels: u8,
    samples: Vec<f32>,
}

impl WarmupPcm {
    pub fn new(sample_rate: u32, channels: u8, samples: Vec<f32>) -> Self {
        Self {
            sample_rate,
            channels,
            samples,
        }
    }
}

/// Keeps the startup boundary explicit until the native ZeroTTS runtime is added.
pub fn validate_warmup_pcm(pcm: WarmupPcm) -> Result<(), TtsError> {
    if pcm.sample_rate != 48_000 || pcm.channels != 1 {
        return Err(TtsError::InvalidWarmupPcm);
    }
    if pcm.samples.is_empty() || pcm.samples.iter().any(|sample| !sample.is_finite()) {
        return Err(TtsError::InvalidWarmupPcm);
    }
    Ok(())
}

pub trait TtsProvider: Send + Sync {
    fn adapter(&self) -> &'static str;
    fn synthesize(&self, _: &str) -> Result<PcmF32Mono, TtsError> {
        Err(TtsError::Failed)
    }
}

#[derive(Debug, Error)]
pub enum TtsError {
    #[error("TTS synthesis failed")]
    Failed,
    #[error("ZeroTTS warmup must produce finite, non-empty 48 kHz mono PCM")]
    InvalidWarmupPcm,
}

pub struct UnavailableTts;

impl TtsProvider for UnavailableTts {
    fn adapter(&self) -> &'static str {
        "unavailable"
    }
}

pub(crate) struct ConfiguredZeroTts;

impl TtsProvider for ConfiguredZeroTts {
    fn adapter(&self) -> &'static str {
        "zerotts_onnx"
    }
}
