//! TTS provider boundary.

use crate::audio::PcmF32Mono;
use thiserror::Error;

pub mod zerotts_onnx;

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
    #[error("ZeroTTS contract is incompatible: {0}")]
    IncompatibleContract(String),
}

pub struct UnavailableTts;

impl TtsProvider for UnavailableTts {
    fn adapter(&self) -> &'static str {
        "unavailable"
    }
}

pub(crate) struct ConfiguredZeroTts {
    #[allow(dead_code)]
    contract: zerotts_onnx::ZeroTtsContract,
}

pub(crate) struct ZeroTtsArtifacts<'a> {
    pub(crate) config: &'a std::path::Path,
    pub(crate) tokenizer: &'a std::path::Path,
    pub(crate) voice: &'a std::path::Path,
    pub(crate) text_encoder: &'a std::path::Path,
    pub(crate) prefix_step: &'a std::path::Path,
    pub(crate) local_frame_decode: &'a std::path::Path,
}

impl ConfiguredZeroTts {
    pub(crate) fn load(
        artifacts: ZeroTtsArtifacts<'_>,
        runtime_library: &std::path::Path,
        num_threads: i32,
    ) -> Result<Self, TtsError> {
        Ok(Self {
            contract: zerotts_onnx::ZeroTtsContract::load_engine(
                artifacts.config,
                artifacts.tokenizer,
                artifacts.voice,
                artifacts.text_encoder,
                artifacts.prefix_step,
                artifacts.local_frame_decode,
                runtime_library,
                num_threads,
            )?,
        })
    }
}

impl TtsProvider for ConfiguredZeroTts {
    fn adapter(&self) -> &'static str {
        "zerotts_onnx"
    }
}
