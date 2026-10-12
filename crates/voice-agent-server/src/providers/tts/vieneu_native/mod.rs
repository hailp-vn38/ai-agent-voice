//! Native VieNeu v3 Turbo inference with retained ONNX workers and SEA-G2P.
pub mod assets;
pub mod config;
pub mod contract;
pub mod descriptor;
mod runtime;
pub mod text;

use std::{
    path::Path,
    sync::{Arc, atomic::AtomicBool},
};

use crate::{
    audio::PcmF32Mono,
    providers::{
        tts::{
            TtsBinding, TtsDiagnosticRequest, TtsError, TtsProvider, TtsSynthesisRequest, TtsWorker,
        },
        vad::initialize_ort,
    },
};
use assets::VieneuNativeAssets;
use config::VieneuNativeConfig;
use runtime::{NativeState, VieneuWorker};

pub(crate) struct ConfiguredVieneuNative {
    state: Arc<NativeState>,
    assets: VieneuNativeAssets,
    options: VieneuNativeConfig,
}

impl ConfiguredVieneuNative {
    pub(crate) fn load(
        assets: VieneuNativeAssets,
        options: VieneuNativeConfig,
        library: &Path,
    ) -> Result<Self, TtsError> {
        if !options.valid_selection() {
            return Err(TtsError::Failed);
        }
        initialize_ort(library)
            .map_err(|error| TtsError::IncompatibleContract(error.to_string()))?;
        Ok(Self {
            state: Arc::new(NativeState::load(&assets)?),
            assets,
            options,
        })
    }
}

impl TtsProvider for ConfiguredVieneuNative {
    fn adapter(&self) -> &'static str {
        "vieneu_native"
    }

    fn open_worker(&self) -> Result<Box<dyn TtsWorker>, TtsError> {
        Ok(Box::new(VieneuWorker::open(
            Arc::clone(&self.state),
            &self.assets,
            &self.options,
        )?))
    }

    fn synthesize(&self, text: &str) -> Result<PcmF32Mono, TtsError> {
        let mut samples = Vec::new();
        self.synthesize_stream(text, &mut |pcm| {
            samples.extend_from_slice(pcm.samples());
            Ok(())
        })?;
        if samples.is_empty() {
            return Err(TtsError::Failed);
        }
        Ok(PcmF32Mono::new(samples, 48_000))
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
        if !assets::VOICES.iter().any(|preset| preset.id == voice) {
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
