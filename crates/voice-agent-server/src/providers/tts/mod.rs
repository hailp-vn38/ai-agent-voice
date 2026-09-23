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

    /// Delivers provider-boundary PCM while synthesis is still in progress.  The default keeps
    /// older adapters source-compatible, but production adapters should override it rather than
    /// buffering a complete utterance before delivery.
    fn synthesize_stream(
        &self,
        text: &str,
        on_pcm: &mut dyn FnMut(PcmF32Mono) -> Result<(), TtsError>,
    ) -> Result<(), TtsError> {
        on_pcm(self.synthesize(text)?)
    }

    /// Opens state that is private to one SpeechOutput delivery. Providers that have no
    /// cross-segment state use the runtime's stateless compatibility stream instead.
    fn open_stream(&self) -> Option<Box<dyn TtsStream>> {
        None
    }

    /// Production providers override this to build native sessions once per runtime worker.
    fn open_worker(&self) -> Result<Box<dyn TtsWorker>, TtsError> {
        Err(TtsError::Failed)
    }
}

pub trait TtsStream: Send {
    fn synthesize(
        &mut self,
        text: &str,
        on_pcm: &mut dyn FnMut(PcmF32Mono) -> Result<(), TtsError>,
    ) -> Result<(), TtsError>;
}

/// Mutable native state owned by one long-lived worker thread.
pub trait TtsWorker: Send {
    fn synthesize(
        &mut self,
        text: &str,
        cancelled: &std::sync::atomic::AtomicBool,
        on_pcm: &mut dyn FnMut(PcmF32Mono) -> Result<(), TtsError>,
    ) -> Result<(), TtsError>;
    fn reset(&mut self) -> Result<(), TtsError>;
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
    pub(crate) codec_decode_full: &'a std::path::Path,
    pub(crate) codec_decode_step: &'a std::path::Path,
    pub(crate) codec_shared_data: &'a std::path::Path,
    pub(crate) codec_metadata: &'a std::path::Path,
    pub(crate) silence_frame: &'a std::path::Path,
}

impl ConfiguredZeroTts {
    pub(crate) fn load(
        artifacts: ZeroTtsArtifacts<'_>,
        runtime_library: &std::path::Path,
        num_threads: i32,
    ) -> Result<Self, TtsError> {
        let contract = zerotts_onnx::ZeroTtsContract::load_engine(
            artifacts.config,
            artifacts.tokenizer,
            artifacts.voice,
            artifacts.text_encoder,
            artifacts.prefix_step,
            artifacts.local_frame_decode,
            artifacts.codec_decode_full,
            artifacts.codec_decode_step,
            artifacts.codec_shared_data,
            artifacts.codec_metadata,
            artifacts.silence_frame,
            runtime_library,
            num_threads,
        )?;
        // This content is startup-only and never becomes a Voice Session or user lease.
        let pcm = contract.synthesize_pcm("ZeroTTS startup readiness.", 256)?;
        validate_warmup_pcm(WarmupPcm::new(
            pcm.sample_rate_hz(),
            1,
            pcm.samples().to_vec(),
        ))?;
        let codes = contract.synthesize_codes("ZeroTTS startup readiness.", 256)?;
        if codes.eoa.is_none() {
            return Err(TtsError::InvalidWarmupPcm);
        }
        contract.validate_full_decode(&codes.frames)?;
        Ok(Self { contract })
    }
}

impl TtsProvider for ConfiguredZeroTts {
    fn adapter(&self) -> &'static str {
        "zerotts_onnx"
    }

    fn synthesize(&self, text: &str) -> Result<PcmF32Mono, TtsError> {
        self.contract.synthesize_pcm(text, 256)
    }

    fn synthesize_stream(
        &self,
        text: &str,
        on_pcm: &mut dyn FnMut(PcmF32Mono) -> Result<(), TtsError>,
    ) -> Result<(), TtsError> {
        self.contract.synthesize_pcm_stream(text, 256, on_pcm)
    }

    fn open_stream(&self) -> Option<Box<dyn TtsStream>> {
        zerotts_onnx::ZeroTtsPcmStream::new(&self.contract)
            .ok()
            .map(|stream| Box::new(ZeroTtsStream { stream }) as Box<dyn TtsStream>)
    }

    fn open_worker(&self) -> Result<Box<dyn TtsWorker>, TtsError> {
        Ok(Box::new(ZeroTtsNativeWorker {
            stream: zerotts_onnx::ZeroTtsPcmStream::new(&self.contract)?,
        }))
    }
}

struct ZeroTtsStream {
    stream: zerotts_onnx::ZeroTtsPcmStream,
}

impl TtsStream for ZeroTtsStream {
    fn synthesize(
        &mut self,
        text: &str,
        on_pcm: &mut dyn FnMut(PcmF32Mono) -> Result<(), TtsError>,
    ) -> Result<(), TtsError> {
        self.stream.synthesize(text, 256, on_pcm)
    }
}

struct ZeroTtsNativeWorker {
    stream: zerotts_onnx::ZeroTtsPcmStream,
}

impl TtsWorker for ZeroTtsNativeWorker {
    fn synthesize(
        &mut self,
        text: &str,
        cancelled: &std::sync::atomic::AtomicBool,
        on_pcm: &mut dyn FnMut(PcmF32Mono) -> Result<(), TtsError>,
    ) -> Result<(), TtsError> {
        if cancelled.load(std::sync::atomic::Ordering::Acquire) {
            return Err(TtsError::Failed);
        }
        self.stream.synthesize(text, 256, &mut |pcm| {
            if cancelled.load(std::sync::atomic::Ordering::Acquire) {
                return Err(TtsError::Failed);
            }
            on_pcm(pcm)
        })
    }

    fn reset(&mut self) -> Result<(), TtsError> {
        self.stream.reset()
    }
}
