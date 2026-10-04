//! TTS provider boundary.

use crate::{audio::PcmF32Mono, config::ZeroTtsDeliveryMode};
use std::{
    collections::BTreeMap,
    path::PathBuf,
    sync::atomic::{AtomicBool, Ordering},
};
use thiserror::Error;

pub(crate) mod chillaudio;
mod file_delivery;
pub(crate) mod kokoro_vi;
pub(crate) mod zerotts;
pub(crate) use chillaudio::ChillAudioWsProvider;
/// Compatibility exports for native ZeroTTS tooling.
pub mod zerotts_onnx {
    pub use super::zerotts::runtime::{
        ZeroTtsContract, ZeroTtsFullPcm, ZeroTtsPcmStream, normalize_text,
    };
}

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

    /// Runs a bounded, provider-boundary diagnostic without changing the loaded runtime.
    /// Adapters must opt in to voice or language overrides explicitly; the compatibility default
    /// only accepts the already-materialized selection.
    fn synthesize_diagnostic(
        &self,
        request: &TtsDiagnosticRequest,
        cancelled: &AtomicBool,
        on_pcm: &mut dyn FnMut(PcmF32Mono) -> Result<(), TtsError>,
    ) -> Result<(), TtsError> {
        if request.voice.is_some() || request.language.is_some() {
            return Err(TtsError::DiagnosticOverrideUnsupported);
        }
        self.synthesize_stream(&request.text, &mut |pcm| {
            if cancelled.load(Ordering::Acquire) {
                return Err(TtsError::Failed);
            }
            on_pcm(pcm)
        })
    }

    fn validate_diagnostic(&self, request: &TtsDiagnosticRequest) -> Result<(), TtsError> {
        if request.voice.is_some() || request.language.is_some() {
            Err(TtsError::DiagnosticOverrideUnsupported)
        } else {
            Ok(())
        }
    }

    /// Opens state that is private to one SpeechOutput delivery. Providers that have no
    /// cross-segment state use the runtime's stateless compatibility stream instead.
    fn open_stream(&self) -> Option<Box<dyn TtsStream>> {
        None
    }

    /// Production providers override this to build native sessions once per runtime worker.
    fn open_worker(&self) -> Result<Box<dyn TtsWorker>, TtsError> {
        Err(TtsError::WorkerUnsupported)
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
    /// Native adapters validate inference on the worker retained in the pool. Remote/stateless
    /// workers intentionally perform no network request here. Warmup must reset operation state.
    fn warmup(&mut self) -> Result<(), TtsError> {
        Ok(())
    }

    fn synthesize(
        &mut self,
        request: &TtsSynthesisRequest,
        cancelled: &std::sync::atomic::AtomicBool,
        on_pcm: &mut dyn FnMut(PcmF32Mono) -> Result<(), TtsError>,
    ) -> Result<(), TtsError>;
    fn reset(&mut self) -> Result<(), TtsError>;

    fn synthesize_diagnostic(
        &mut self,
        request: &TtsSynthesisRequest,
        _: &TtsDiagnosticRequest,
        cancelled: &AtomicBool,
        on_pcm: &mut dyn FnMut(PcmF32Mono) -> Result<(), TtsError>,
    ) -> Result<(), TtsError> {
        self.synthesize(request, cancelled, on_pcm)
    }
}

/// Bounded PCM validation on the retained worker, followed by an acknowledged independent-
/// operation reset. No warmup audio becomes user delivery or cross-segment state.
pub(crate) fn warmup_retained_worker(
    worker: &mut dyn TtsWorker,
    text: &str,
    sample_rate: u32,
) -> Result<(), TtsError> {
    let cancelled = AtomicBool::new(false);
    let request = TtsSynthesisRequest {
        text: text.into(),
        selection: TtsBinding::readiness(),
    };
    let mut samples = 0usize;
    worker.synthesize(&request, &cancelled, &mut |pcm| {
        if pcm.sample_rate_hz() != sample_rate
            || pcm.samples().iter().any(|sample| !sample.is_finite())
        {
            return Err(TtsError::Failed);
        }
        samples = samples
            .checked_add(pcm.samples().len())
            .ok_or(TtsError::Failed)?;
        if samples > sample_rate as usize * 30 {
            return Err(TtsError::Failed);
        }
        Ok(())
    })?;
    if samples == 0 {
        return Err(TtsError::Failed);
    }
    worker.reset()
}

/// Selection pinned to a logical TTS provider view and carried by every queued operation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TtsBinding {
    pub voice: String,
    pub language: String,
}

impl TtsBinding {
    /// Compatibility selection for adapters whose worker does not use a voice embedding.
    pub fn readiness() -> Self {
        Self {
            voice: "__readiness__".into(),
            language: "und".into(),
        }
    }
}

/// Fully resolved work sent to a retained TTS worker. It never contains optional selection.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TtsSynthesisRequest {
    pub text: String,
    pub selection: TtsBinding,
}

/// A typed, temporary TTS selection. It never represents desired configuration.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TtsDiagnosticRequest {
    pub text: String,
    pub voice: Option<String>,
    pub language: Option<String>,
}

#[derive(Debug, Error)]
pub enum TtsError {
    #[error("adapter uses the explicit compatibility worker")]
    WorkerUnsupported,
    #[error("TTS synthesis failed")]
    Failed,
    #[error("ZeroTTS warmup must produce finite, non-empty 48 kHz mono PCM")]
    InvalidWarmupPcm,
    #[error("ZeroTTS contract is incompatible: {0}")]
    IncompatibleContract(String),
    #[error("TTS temporary audio file failed: {0}")]
    TemporaryAudio(String),
    #[error("TTS remote connection failed")]
    RemoteConnection,
    #[error("TTS remote task failed")]
    RemoteTask,
    #[error("TTS remote timeout")]
    RemoteTimeout,
    #[error("TTS audio decode failed")]
    AudioDecode,
    #[error("TTS diagnostic override is not available in the loaded runtime")]
    DiagnosticOverrideUnsupported,
    #[error("TTS diagnostic response is invalid")]
    InvalidDiagnosticResponse,
    #[error("TTS does not support voice {0}")]
    UnsupportedVoice(String),
    #[error("TTS does not support language {0}")]
    UnsupportedLanguage(String),
    #[error("TTS stream binding does not match its logical runtime")]
    BindingMismatch,
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
    delivery_mode: ZeroTtsDeliveryMode,
}

pub(crate) struct ZeroTtsArtifacts<'a> {
    pub(crate) config: &'a std::path::Path,
    pub(crate) tokenizer: &'a std::path::Path,
    pub(crate) voices_index: &'a std::path::Path,
    pub(crate) voices: BTreeMap<String, PathBuf>,
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
        delivery_mode: ZeroTtsDeliveryMode,
    ) -> Result<Self, TtsError> {
        let contract = zerotts_onnx::ZeroTtsContract::load_engine_with_voices(
            artifacts.config,
            artifacts.tokenizer,
            artifacts.voices_index,
            artifacts.voices,
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
        // Readiness executes on each retained worker, avoiding temporary native engines.
        Ok(Self {
            contract,
            delivery_mode,
        })
    }
}

impl TtsProvider for ConfiguredZeroTts {
    fn adapter(&self) -> &'static str {
        "zerotts_onnx"
    }

    fn synthesize(&self, _: &str) -> Result<PcmF32Mono, TtsError> {
        // This object is a physical resource. Only its `TtsWorkerRuntime` logical views own a
        // configured voice binding, so a direct fallback must never pick a registry default.
        Err(TtsError::WorkerUnsupported)
    }

    fn synthesize_stream(
        &self,
        _: &str,
        _: &mut dyn FnMut(PcmF32Mono) -> Result<(), TtsError>,
    ) -> Result<(), TtsError> {
        Err(TtsError::WorkerUnsupported)
    }

    fn synthesize_diagnostic(
        &self,
        request: &TtsDiagnosticRequest,
        cancelled: &AtomicBool,
        on_pcm: &mut dyn FnMut(PcmF32Mono) -> Result<(), TtsError>,
    ) -> Result<(), TtsError> {
        let voice = request
            .voice
            .as_deref()
            .ok_or(TtsError::InvalidDiagnosticResponse)?;
        let language = request
            .language
            .as_deref()
            .ok_or(TtsError::InvalidDiagnosticResponse)?;
        if language != "vi-VN" {
            return Err(TtsError::UnsupportedLanguage(language.into()));
        }
        let voice = self.contract.voice(voice)?;
        match self.delivery_mode {
            ZeroTtsDeliveryMode::Stream => zerotts_onnx::ZeroTtsPcmStream::new(&self.contract)?
                .synthesize_with_voice(&request.text, 256, &voice, &mut |pcm| {
                    if cancelled.load(Ordering::Acquire) {
                        return Err(TtsError::Failed);
                    }
                    on_pcm(pcm)
                }),
            ZeroTtsDeliveryMode::File => {
                let pcm = zerotts_onnx::ZeroTtsFullPcm::new(&self.contract)?
                    .synthesize_with_voice(&request.text, 256, cancelled, &voice)?;
                file_delivery::deliver_via_temporary_wav(pcm, cancelled, on_pcm)
            }
        }
    }

    fn validate_diagnostic(&self, request: &TtsDiagnosticRequest) -> Result<(), TtsError> {
        let voice = request
            .voice
            .as_deref()
            .ok_or(TtsError::InvalidDiagnosticResponse)?;
        let language = request
            .language
            .as_deref()
            .ok_or(TtsError::InvalidDiagnosticResponse)?;
        if language != "vi-VN" {
            return Err(TtsError::UnsupportedLanguage(language.into()));
        }
        self.contract.voice(voice).map(|_| ())
    }

    fn open_stream(&self) -> Option<Box<dyn TtsStream>> {
        if self.delivery_mode == ZeroTtsDeliveryMode::File {
            return None;
        }
        zerotts_onnx::ZeroTtsPcmStream::new(&self.contract)
            .ok()
            .map(|stream| Box::new(ZeroTtsStream { stream }) as Box<dyn TtsStream>)
    }

    fn open_worker(&self) -> Result<Box<dyn TtsWorker>, TtsError> {
        let delivery = match self.delivery_mode {
            ZeroTtsDeliveryMode::Stream => ZeroTtsNativeDelivery::Stream(Box::new(
                zerotts_onnx::ZeroTtsPcmStream::new(&self.contract)?,
            )),
            ZeroTtsDeliveryMode::File => ZeroTtsNativeDelivery::File(Box::new(
                zerotts_onnx::ZeroTtsFullPcm::new(&self.contract)?,
            )),
        };
        Ok(Box::new(ZeroTtsNativeWorker {
            contract: self.contract.clone(),
            delivery,
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
    contract: zerotts_onnx::ZeroTtsContract,
    delivery: ZeroTtsNativeDelivery,
}

enum ZeroTtsNativeDelivery {
    Stream(Box<zerotts_onnx::ZeroTtsPcmStream>),
    File(Box<zerotts_onnx::ZeroTtsFullPcm>),
}

impl TtsWorker for ZeroTtsNativeWorker {
    fn warmup(&mut self) -> Result<(), TtsError> {
        let request = TtsSynthesisRequest {
            text: "ZeroTTS startup readiness.".into(),
            selection: TtsBinding {
                voice: self.contract.readiness_voice_id()?.into(),
                language: "vi-VN".into(),
            },
        };
        let cancelled = AtomicBool::new(false);
        let mut samples = 0usize;
        self.synthesize(&request, &cancelled, &mut |pcm| {
            if pcm.sample_rate_hz() != 48_000
                || pcm.samples().iter().any(|sample| !sample.is_finite())
            {
                return Err(TtsError::InvalidWarmupPcm);
            }
            samples += pcm.samples().len();
            Ok(())
        })?;
        if samples == 0 {
            return Err(TtsError::InvalidWarmupPcm);
        }
        self.reset()
    }

    fn synthesize(
        &mut self,
        request: &TtsSynthesisRequest,
        cancelled: &std::sync::atomic::AtomicBool,
        on_pcm: &mut dyn FnMut(PcmF32Mono) -> Result<(), TtsError>,
    ) -> Result<(), TtsError> {
        if cancelled.load(std::sync::atomic::Ordering::Acquire) {
            return Err(TtsError::Failed);
        }
        if request.selection.language != "vi-VN" {
            return Err(TtsError::UnsupportedLanguage(
                request.selection.language.clone(),
            ));
        }
        let voice = self.contract.voice(&request.selection.voice)?;
        match &mut self.delivery {
            ZeroTtsNativeDelivery::Stream(stream) => {
                stream.synthesize_with_voice(&request.text, 256, &voice, &mut |pcm| {
                    if cancelled.load(std::sync::atomic::Ordering::Acquire) {
                        return Err(TtsError::Failed);
                    }
                    on_pcm(pcm)
                })
            }
            ZeroTtsNativeDelivery::File(full) => {
                let pcm = full.synthesize_with_voice(&request.text, 256, cancelled, &voice)?;
                file_delivery::deliver_via_temporary_wav(pcm, cancelled, on_pcm)
            }
        }
    }

    fn reset(&mut self) -> Result<(), TtsError> {
        match &mut self.delivery {
            ZeroTtsNativeDelivery::Stream(stream) => stream.reset(),
            ZeroTtsNativeDelivery::File(full) => {
                full.reset();
                Ok(())
            }
        }
    }
}
