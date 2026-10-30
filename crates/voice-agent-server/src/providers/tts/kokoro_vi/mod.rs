//! Native ONNX adapter for the ContextBox Vietnamese Kokoro model.
//!
//! The runtime receives already-converted `KOVI_VOICEPACK_V1` voicepacks. It never imports
//! PyTorch and never converts assets during synthesis; `assets` owns conversion.

pub mod assets;
pub(crate) mod config;
pub(crate) mod descriptor;
mod g2p;
mod runtime;
mod tokenizer;
mod voicepack;

use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::{Arc, atomic::AtomicBool},
};

use crate::{
    audio::PcmF32Mono,
    config::{KokoroViOnnxConfig, RuntimeConfig},
    providers::tts::{
        TtsBinding, TtsDiagnosticRequest, TtsError, TtsProvider, TtsSynthesisRequest, TtsWorker,
    },
};

use runtime::KokoroViWorker;
use tokenizer::Tokenizer;
use voicepack::Voicepack;

pub(crate) struct KokoroViArtifacts<'a> {
    pub(crate) model: &'a Path,
    pub(crate) config: &'a Path,
    pub(crate) voicepacks: &'a BTreeMap<String, PathBuf>,
}

/// Immutable factory state. ONNX sessions and the G2P child are worker-owned.
pub(crate) struct ConfiguredKokoroVi {
    model: PathBuf,
    tokenizer: Tokenizer,
    default_voice: String,
    language: String,
    voicepacks: Arc<BTreeMap<String, Voicepack>>,
    g2p_executable: PathBuf,
    runtime_library: PathBuf,
    num_threads: i32,
    speed: f32,
}

impl ConfiguredKokoroVi {
    pub(crate) fn load(
        options: &KokoroViOnnxConfig,
        runtime: &RuntimeConfig,
        artifacts: KokoroViArtifacts<'_>,
    ) -> Result<Self, TtsError> {
        if !artifacts.model.is_file() {
            return Err(TtsError::IncompatibleContract(
                "Kokoro ONNX model artifact is missing".into(),
            ));
        }
        if !runtime.kokoro_vi.g2p_executable.is_file() {
            return Err(TtsError::IncompatibleContract(
                "Kokoro Vietnamese G2P executable is missing".into(),
            ));
        }
        let tokenizer = Tokenizer::load(artifacts.config)?;
        let mut loaded_voicepacks = BTreeMap::new();
        for (voice_id, path) in artifacts.voicepacks {
            let vp = Voicepack::load(path)?;
            loaded_voicepacks.insert(voice_id.clone(), vp);
        }
        if !loaded_voicepacks.contains_key(&options.voice) {
            return Err(TtsError::UnsupportedVoice(options.voice.clone()));
        }
        // Graph validation and warmup belong to the exact retained workers.
        Ok(Self {
            model: artifacts.model.into(),
            tokenizer,
            default_voice: options.voice.clone(),
            language: options.language.clone(),
            voicepacks: Arc::new(loaded_voicepacks),
            g2p_executable: runtime.kokoro_vi.g2p_executable.clone(),
            runtime_library: runtime.onnx.library.clone(),
            num_threads: options.num_threads,
            speed: f32::from(options.speed_percent) / 100.0,
        })
    }
}

impl TtsProvider for ConfiguredKokoroVi {
    fn adapter(&self) -> &'static str {
        "kokoro_vi_onnx"
    }

    fn synthesize(&self, text: &str) -> Result<PcmF32Mono, TtsError> {
        let mut worker = self.open_worker()?;
        let cancelled = AtomicBool::new(false);
        let mut samples = Vec::new();
        worker.synthesize(
            &TtsSynthesisRequest {
                text: text.into(),
                selection: TtsBinding::readiness(),
            },
            &cancelled,
            &mut |pcm| {
                if pcm.sample_rate_hz() != 24_000 {
                    return Err(TtsError::Failed);
                }
                samples.extend_from_slice(pcm.samples());
                Ok(())
            },
        )?;
        if samples.is_empty() {
            return Err(TtsError::Failed);
        }
        Ok(PcmF32Mono::new(samples, 24_000))
    }

    fn synthesize_stream(
        &self,
        text: &str,
        on_pcm: &mut dyn FnMut(PcmF32Mono) -> Result<(), TtsError>,
    ) -> Result<(), TtsError> {
        let mut worker = self.open_worker()?;
        let cancelled = AtomicBool::new(false);
        worker.synthesize(
            &TtsSynthesisRequest {
                text: text.into(),
                selection: TtsBinding::readiness(),
            },
            &cancelled,
            on_pcm,
        )
    }

    fn validate_diagnostic(&self, request: &TtsDiagnosticRequest) -> Result<(), TtsError> {
        let voice = request.voice.as_deref().unwrap_or(&self.default_voice);
        let language = request.language.as_deref().unwrap_or(&self.language);
        if language != "vi-VN" {
            return Err(TtsError::UnsupportedLanguage(language.into()));
        }
        if !self.voicepacks.contains_key(voice) {
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
        let mut worker = self.open_worker()?;
        worker.synthesize(
            &TtsSynthesisRequest {
                text: request.text.clone(),
                selection: TtsBinding {
                    voice: request
                        .voice
                        .clone()
                        .unwrap_or_else(|| self.default_voice.clone()),
                    language: request
                        .language
                        .clone()
                        .unwrap_or_else(|| self.language.clone()),
                },
            },
            cancelled,
            on_pcm,
        )
    }

    fn open_worker(&self) -> Result<Box<dyn TtsWorker>, TtsError> {
        Ok(Box::new(KokoroViWorker::open(
            &self.model,
            &self.tokenizer,
            self.default_voice.clone(),
            Arc::clone(&self.voicepacks),
            &self.g2p_executable,
            &self.runtime_library,
            self.num_threads,
            self.speed,
        )?))
    }
}

#[cfg(test)]
mod real_model_tests {
    use std::{collections::BTreeMap, path::PathBuf};

    use crate::{
        config::{KokoroViOnnxConfig, KokoroViRuntimeConfig, OnnxRuntimeConfig, RuntimeConfig},
        providers::tts::TtsProvider,
    };

    use super::{ConfiguredKokoroVi, KokoroViArtifacts};

    #[test]
    #[ignore = "requires models/TTS/kokoro_vi and runtime/kokoro-vi local assets"]
    fn loads_local_contextbox_model_and_synthesizes_24khz_pcm() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../models/TTS/kokoro_vi");
        let runtime_root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../runtime");
        let voicepacks = BTreeMap::from([(
            "diem_trinh".to_owned(),
            root.join("voicepacks/diem_trinh.bin"),
        )]);
        let provider = ConfiguredKokoroVi::load(
            &KokoroViOnnxConfig::default(),
            &RuntimeConfig {
                onnx: OnnxRuntimeConfig {
                    library: runtime_root.join(if cfg!(target_os = "macos") {
                        "onnxruntime/libonnxruntime.dylib"
                    } else {
                        "onnxruntime/libonnxruntime.so"
                    }),
                    ..Default::default()
                },
                kokoro_vi: KokoroViRuntimeConfig {
                    g2p_executable: runtime_root.join("kokoro-vi/kokoro_vi_g2p"),
                },
                ..Default::default()
            },
            KokoroViArtifacts {
                model: &root.join("kokoro_vi.onnx"),
                config: &root.join("config.json"),
                voicepacks: &voicepacks,
            },
        )
        .expect("load prepared local Kokoro Vietnamese provider");

        let mut chunks = Vec::new();
        provider
            .synthesize_stream("Tường nhà khách.", &mut |pcm| {
                chunks.push(pcm);
                Ok(())
            })
            .expect("synthesize a Vietnamese smoke utterance");
        assert!(chunks.len() > 1);
        assert!(chunks.iter().all(|pcm| pcm.sample_rate_hz() == 24_000));
        assert!(
            chunks
                .iter()
                .all(|pcm| !pcm.samples().is_empty() && pcm.samples().len() <= 1_440)
        );
        assert!(
            chunks
                .iter()
                .flat_map(|pcm| pcm.samples())
                .all(|sample| sample.is_finite())
        );
    }
}
