//! Local inference boundary. Session code only depends on these domain contracts.

use std::{
    path::Path,
    sync::{Arc, Mutex},
};

use crate::{
    audio::PcmF32Mono,
    config::{LimitsConfig, ProvidersConfig},
};
use sherpa_onnx::{
    OnlineRecognizer, OnlineRecognizerConfig, OnlineStream, SileroVadModelConfig, VadModelConfig,
    VoiceActivityDetector,
};
use thiserror::Error;

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum AsrError {
    #[error("ASR provider failed: {0}")]
    Failed(String),
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum VadError {
    #[error("VAD provider failed: {0}")]
    Failed(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VadEvent {
    SpeechStart,
    SpeechEnd,
}

#[derive(Debug, Error)]
pub enum ProviderLoadError {
    #[error("unsupported {kind} adapter `{adapter}`")]
    UnsupportedAdapter { kind: &'static str, adapter: String },
    #[error("required model artifact is missing: {0}")]
    MissingArtifact(String),
    #[error("cannot initialize local {0} provider")]
    Initialize(&'static str),
}

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

/// VAD runtime boundary; its per-cycle session API is added with Auto in ticket 02.
pub trait VadProvider: Send + Sync {
    fn open(&self) -> Result<Box<dyn VadSession>, VadError>;
    fn adapter(&self) -> &'static str;
}

/// A VAD session is pinned to one Auto listening cycle. Its events contain no audio or text.
pub trait VadSession: Send {
    fn push_pcm(&mut self, pcm: &PcmF32Mono) -> Result<Vec<VadEvent>, VadError>;
    fn reset(&mut self) -> Result<(), VadError>;
    fn close(&mut self) -> Result<(), VadError>;
}

/// Application-owned providers injected into each Voice Session.
pub struct ProviderSet {
    asr: Arc<dyn AsrProvider>,
    vad: Arc<dyn VadProvider>,
    capacity: Arc<ProviderCapacity>,
}

#[derive(Clone, Copy, Debug)]
pub struct ProviderCapacityLimits {
    pub max_asr_streams: usize,
    pub max_vad_sessions: usize,
}

impl Default for ProviderCapacityLimits {
    fn default() -> Self {
        Self {
            max_asr_streams: 8,
            max_vad_sessions: 8,
        }
    }
}

struct ProviderCapacity {
    limits: ProviderCapacityLimits,
    in_use: Mutex<ProviderCapacityUsage>,
}

#[derive(Default)]
struct ProviderCapacityUsage {
    asr_streams: usize,
    vad_sessions: usize,
}

impl ProviderCapacity {
    fn new(limits: ProviderCapacityLimits) -> Self {
        Self {
            limits,
            in_use: Mutex::new(ProviderCapacityUsage::default()),
        }
    }

    fn acquire_asr(self: &Arc<Self>) -> Result<CapacityLease, AsrError> {
        let mut in_use = self.in_use.lock().expect("capacity mutex poisoned");
        if in_use.asr_streams == self.limits.max_asr_streams {
            return Err(AsrError::Failed("ASR stream capacity exhausted".into()));
        }
        in_use.asr_streams += 1;
        Ok(CapacityLease::Asr(Arc::clone(self)))
    }

    fn acquire_vad(self: &Arc<Self>) -> Result<CapacityLease, VadError> {
        let mut in_use = self.in_use.lock().expect("capacity mutex poisoned");
        if in_use.vad_sessions == self.limits.max_vad_sessions {
            return Err(VadError::Failed("VAD session capacity exhausted".into()));
        }
        in_use.vad_sessions += 1;
        Ok(CapacityLease::Vad(Arc::clone(self)))
    }
}

enum CapacityLease {
    Asr(Arc<ProviderCapacity>),
    Vad(Arc<ProviderCapacity>),
}

impl Drop for CapacityLease {
    fn drop(&mut self) {
        let is_asr = matches!(self, Self::Asr(_));
        let capacity = match self {
            Self::Asr(capacity) | Self::Vad(capacity) => capacity,
        };
        let mut in_use = capacity.in_use.lock().expect("capacity mutex poisoned");
        if is_asr {
            in_use.asr_streams -= 1;
        } else {
            in_use.vad_sessions -= 1;
        }
    }
}

impl ProviderSet {
    pub fn new(asr: Arc<dyn AsrProvider>) -> Self {
        Self::with_vad(Arc::new(UnavailableVad), asr)
    }

    pub fn with_vad(vad: Arc<dyn VadProvider>, asr: Arc<dyn AsrProvider>) -> Self {
        Self::with_vad_capacity(vad, asr, ProviderCapacityLimits::default())
    }

    pub fn with_vad_capacity(
        vad: Arc<dyn VadProvider>,
        asr: Arc<dyn AsrProvider>,
        capacity: ProviderCapacityLimits,
    ) -> Self {
        Self {
            asr,
            vad,
            capacity: Arc::new(ProviderCapacity::new(capacity)),
        }
    }

    pub fn open_asr(&self) -> Result<Box<dyn AsrSession>, AsrError> {
        let lease = self.capacity.acquire_asr()?;
        let session = self.asr.open()?;
        Ok(Box::new(LeasedAsrSession {
            session,
            _lease: lease,
        }))
    }

    pub fn open_vad(&self) -> Result<Box<dyn VadSession>, VadError> {
        let lease = self.capacity.acquire_vad()?;
        let session = self.vad.open()?;
        Ok(Box::new(LeasedVadSession {
            session,
            _lease: lease,
        }))
    }

    pub fn vad_adapter(&self) -> &'static str {
        self.vad.adapter()
    }

    pub fn unavailable() -> Self {
        Self::new(Arc::new(UnavailableAsr))
    }

    /// Loads and warms all required local runtimes before the application binds a socket.
    pub fn load(
        config: &ProvidersConfig,
        limits: &LimitsConfig,
    ) -> Result<Self, ProviderLoadError> {
        if config.vad.adapter != "silero_onnx" {
            return Err(ProviderLoadError::UnsupportedAdapter {
                kind: "VAD",
                adapter: config.vad.adapter.clone(),
            });
        }
        if config.asr.adapter != "zipformer_sherpa" {
            return Err(ProviderLoadError::UnsupportedAdapter {
                kind: "ASR",
                adapter: config.asr.adapter.clone(),
            });
        }
        require_file(&config.vad.model)?;
        for artifact in ["encoder.onnx", "decoder.onnx", "joiner.onnx", "tokens.txt"] {
            require_file(&config.asr.model_dir.join(artifact))?;
        }

        let vad_config = VadModelConfig {
            sample_rate: 16_000,
            num_threads: config.vad.num_threads,
            provider: Some("cpu".into()),
            silero_vad: SileroVadModelConfig {
                model: Some(config.vad.model.display().to_string()),
                threshold: 0.5,
                min_silence_duration: 0.6,
                min_speech_duration: 0.18,
                window_size: 512,
                max_speech_duration: 30.0,
            },
            ..Default::default()
        };
        let vad = VoiceActivityDetector::create(&vad_config, 60.0)
            .ok_or(ProviderLoadError::Initialize("Silero VAD"))?;
        drop(vad);

        let mut asr_config = OnlineRecognizerConfig::default();
        asr_config.model_config.transducer.encoder =
            Some(path(&config.asr.model_dir, "encoder.onnx"));
        asr_config.model_config.transducer.decoder =
            Some(path(&config.asr.model_dir, "decoder.onnx"));
        asr_config.model_config.transducer.joiner =
            Some(path(&config.asr.model_dir, "joiner.onnx"));
        asr_config.model_config.tokens = Some(path(&config.asr.model_dir, "tokens.txt"));
        asr_config.model_config.num_threads = config.asr.num_threads;
        asr_config.model_config.provider = Some("cpu".into());
        asr_config.decoding_method = Some("greedy_search".into());
        asr_config.enable_endpoint = false;
        let recognizer = Arc::new(
            OnlineRecognizer::create(&asr_config)
                .ok_or(ProviderLoadError::Initialize("Zipformer ASR"))?,
        );
        // Constructing a stream forces the recognizer's stream runtime to initialize before bind.
        drop(recognizer.create_stream());
        Ok(Self::with_vad_capacity(
            Arc::new(LoadedSileroVad { config: vad_config }),
            Arc::new(ZipformerAsrProvider { recognizer }),
            ProviderCapacityLimits {
                max_asr_streams: limits.max_asr_streams,
                max_vad_sessions: limits.max_vad_sessions,
            },
        ))
    }
}

struct LeasedAsrSession {
    session: Box<dyn AsrSession>,
    _lease: CapacityLease,
}

impl AsrSession for LeasedAsrSession {
    fn push_pcm(&mut self, pcm: &PcmF32Mono) -> Result<Vec<AsrEvent>, AsrError> {
        self.session.push_pcm(pcm)
    }

    fn finish(&mut self) -> Result<AsrResult, AsrError> {
        self.session.finish()
    }

    fn cancel(&mut self) {
        self.session.cancel();
    }
}

struct LeasedVadSession {
    session: Box<dyn VadSession>,
    _lease: CapacityLease,
}

impl VadSession for LeasedVadSession {
    fn push_pcm(&mut self, pcm: &PcmF32Mono) -> Result<Vec<VadEvent>, VadError> {
        self.session.push_pcm(pcm)
    }

    fn reset(&mut self) -> Result<(), VadError> {
        self.session.reset()
    }

    fn close(&mut self) -> Result<(), VadError> {
        self.session.close()
    }
}

fn require_file(path: &Path) -> Result<(), ProviderLoadError> {
    if path.is_file() {
        Ok(())
    } else {
        Err(ProviderLoadError::MissingArtifact(
            path.display().to_string(),
        ))
    }
}

fn path(directory: &Path, file: &str) -> String {
    directory.join(file).display().to_string()
}

struct UnavailableAsr;

struct UnavailableVad;

impl VadProvider for UnavailableVad {
    fn open(&self) -> Result<Box<dyn VadSession>, VadError> {
        Err(VadError::Failed("VAD provider is not initialized".into()))
    }

    fn adapter(&self) -> &'static str {
        "unavailable"
    }
}

struct LoadedSileroVad {
    config: VadModelConfig,
}

impl VadProvider for LoadedSileroVad {
    fn open(&self) -> Result<Box<dyn VadSession>, VadError> {
        let detector = VoiceActivityDetector::create(&self.config, 60.0)
            .ok_or_else(|| VadError::Failed("cannot initialize Silero VAD session".into()))?;
        Ok(Box::new(SileroVadSession {
            detector,
            detecting_speech: false,
        }))
    }

    fn adapter(&self) -> &'static str {
        "silero_onnx"
    }
}

struct SileroVadSession {
    detector: VoiceActivityDetector,
    detecting_speech: bool,
}

impl VadSession for SileroVadSession {
    fn push_pcm(&mut self, pcm: &PcmF32Mono) -> Result<Vec<VadEvent>, VadError> {
        if pcm.sample_rate_hz() != 16_000 {
            return Err(VadError::Failed("VAD requires canonical 16 kHz PCM".into()));
        }
        self.detector.accept_waveform(pcm.samples());
        let detected = self.detector.detected();
        let event = match (self.detecting_speech, detected) {
            (false, true) => Some(VadEvent::SpeechStart),
            (true, false) => Some(VadEvent::SpeechEnd),
            _ => None,
        };
        self.detecting_speech = detected;
        Ok(event.into_iter().collect())
    }

    fn reset(&mut self) -> Result<(), VadError> {
        self.detector.reset();
        self.detecting_speech = false;
        Ok(())
    }

    fn close(&mut self) -> Result<(), VadError> {
        Ok(())
    }
}

impl AsrProvider for UnavailableAsr {
    fn open(&self) -> Result<Box<dyn AsrSession>, AsrError> {
        Ok(Box::new(UnavailableAsrSession))
    }
}

struct UnavailableAsrSession;

impl AsrSession for UnavailableAsrSession {
    fn push_pcm(&mut self, _: &PcmF32Mono) -> Result<Vec<AsrEvent>, AsrError> {
        Ok(Vec::new())
    }

    fn finish(&mut self) -> Result<AsrResult, AsrError> {
        Err(AsrError::Failed("ASR provider is not initialized".into()))
    }

    fn cancel(&mut self) {}
}

struct ZipformerAsrProvider {
    recognizer: Arc<OnlineRecognizer>,
}

impl AsrProvider for ZipformerAsrProvider {
    fn open(&self) -> Result<Box<dyn AsrSession>, AsrError> {
        Ok(Box::new(ZipformerAsrSession {
            recognizer: Arc::clone(&self.recognizer),
            stream: self.recognizer.create_stream(),
        }))
    }
}

struct ZipformerAsrSession {
    recognizer: Arc<OnlineRecognizer>,
    stream: OnlineStream,
}

impl AsrSession for ZipformerAsrSession {
    fn push_pcm(&mut self, pcm: &PcmF32Mono) -> Result<Vec<AsrEvent>, AsrError> {
        if pcm.sample_rate_hz() != 16_000 {
            return Err(AsrError::Failed("ASR requires canonical 16 kHz PCM".into()));
        }
        self.stream
            .accept_waveform(pcm.sample_rate_hz() as i32, pcm.samples());
        while self.recognizer.is_ready(&self.stream) {
            self.recognizer.decode(&self.stream);
        }
        Ok(self
            .recognizer
            .get_result(&self.stream)
            .filter(|result| !result.text.trim().is_empty())
            .map(|result| vec![AsrEvent::Partial(result.text)])
            .unwrap_or_default())
    }

    fn finish(&mut self) -> Result<AsrResult, AsrError> {
        self.stream.input_finished();
        while self.recognizer.is_ready(&self.stream) {
            self.recognizer.decode(&self.stream);
        }
        self.recognizer
            .get_result(&self.stream)
            .map(|result| AsrResult::new(result.text))
            .ok_or_else(|| AsrError::Failed("Zipformer returned no final result".into()))
    }

    fn cancel(&mut self) {}
}
