//! Pure-Rust ONNX adapter for ViFastTts Ngọc Huyền Small (`dvchd/vifasttts-ngochuyen-small`).

pub mod assets;
pub mod config;
pub mod descriptor;
pub mod dsp;
pub mod frontend;
pub mod runtime;

use std::{path::PathBuf, sync::atomic::AtomicBool};

use crate::{
    audio::PcmF32Mono,
    config::{RuntimeConfig, VifastttsOnnxConfig},
    providers::tts::{
        TtsBinding, TtsDiagnosticRequest, TtsError, TtsProvider, TtsSynthesisRequest, TtsWorker,
    },
};

use assets::{VOICES, VifastttsAssets};
use runtime::{VIFASTTTS_SAMPLE_RATE_HZ, VifastttsWorker};

pub struct ConfiguredVifasttts {
    assets: VifastttsAssets,
    options: VifastttsOnnxConfig,
    runtime_library: PathBuf,
    num_threads: i32,
    speed: f32,
}

impl ConfiguredVifasttts {
    pub fn load(
        options: &VifastttsOnnxConfig,
        runtime: &RuntimeConfig,
        assets: VifastttsAssets,
    ) -> Result<Self, TtsError> {
        if !options.valid_selection() {
            return Err(TtsError::Failed);
        }
        if !assets.acoustic.is_file() || !assets.vocoder.is_file() {
            return Err(TtsError::IncompatibleContract(
                "ViFastTts ONNX model artifacts are missing".into(),
            ));
        }
        Ok(Self {
            assets,
            options: options.clone(),
            runtime_library: runtime.onnx.library.clone(),
            num_threads: options.num_threads,
            speed: f32::from(options.speed_percent) / 100.0,
        })
    }
}

impl TtsProvider for ConfiguredVifasttts {
    fn adapter(&self) -> &'static str {
        "vifasttts_onnx"
    }

    fn open_worker(&self) -> Result<Box<dyn TtsWorker>, TtsError> {
        Ok(Box::new(VifastttsWorker::open(
            &self.assets.acoustic,
            &self.assets.vocoder,
            &self.runtime_library,
            self.num_threads,
            self.speed,
        )?))
    }

    fn synthesize(&self, text: &str) -> Result<PcmF32Mono, TtsError> {
        let mut samples = Vec::new();
        self.synthesize_stream(text, &mut |pcm| {
            if pcm.sample_rate_hz() != VIFASTTTS_SAMPLE_RATE_HZ {
                return Err(TtsError::Failed);
            }
            samples.extend_from_slice(pcm.samples());
            Ok(())
        })?;
        if samples.is_empty() {
            return Err(TtsError::Failed);
        }
        Ok(PcmF32Mono::new(samples, VIFASTTTS_SAMPLE_RATE_HZ))
    }

    fn synthesize_stream(
        &self,
        text: &str,
        on_pcm: &mut dyn FnMut(PcmF32Mono) -> Result<(), TtsError>,
    ) -> Result<(), TtsError> {
        self.synthesize_diagnostic(
            &TtsDiagnosticRequest {
                text: text.into(),
                voice: None,
                language: None,
            },
            &AtomicBool::new(false),
            on_pcm,
        )
    }

    fn validate_diagnostic(&self, request: &TtsDiagnosticRequest) -> Result<(), TtsError> {
        let voice = request.voice.as_deref().unwrap_or(&self.options.voice);
        let language = request
            .language
            .as_deref()
            .unwrap_or(&self.options.language);
        if language != "vi-VN" {
            return Err(TtsError::UnsupportedLanguage(language.into()));
        }
        if !VOICES.iter().any(|preset| preset.id == voice) {
            return Err(TtsError::UnsupportedVoice(voice.into()));
        }
        Ok(())
    }

    fn synthesize_diagnostic(
        &self,
        request: &TtsDiagnosticRequest,
        cancelled: &AtomicBool,
        on_pcm: &mut dyn FnMut(PcmF32Mono) -> Result<(), TtsError>,
    ) -> Result<(), TtsError> {
        self.validate_diagnostic(request)?;
        self.open_worker()?.synthesize(
            &TtsSynthesisRequest {
                text: request.text.clone(),
                selection: TtsBinding {
                    voice: request
                        .voice
                        .clone()
                        .unwrap_or_else(|| self.options.voice.clone()),
                    language: request
                        .language
                        .clone()
                        .unwrap_or_else(|| self.options.language.clone()),
                },
            },
            cancelled,
            on_pcm,
        )
    }
}
