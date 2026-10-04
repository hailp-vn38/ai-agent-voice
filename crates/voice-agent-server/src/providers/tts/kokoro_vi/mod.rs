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

use std::path::Path;

use crate::{
    audio::PcmF32Mono,
    config::{KokoroViOnnxConfig, RuntimeConfig},
    providers::tts::{TtsBinding, TtsError, TtsProvider, TtsSynthesisRequest, TtsWorker},
};

use runtime::KokoroViWorker;
use tokenizer::Tokenizer;
use voicepack::Voicepack;

pub(crate) struct KokoroViArtifacts<'a> {
    pub(crate) model: &'a Path,
    pub(crate) config: &'a Path,
    pub(crate) voicepack: &'a Path,
}

/// Immutable factory state. ONNX sessions and the G2P child are worker-owned.
pub(crate) struct ConfiguredKokoroVi {
    model: std::path::PathBuf,
    tokenizer: Tokenizer,
    voicepack: Voicepack,
    g2p_executable: std::path::PathBuf,
    runtime_library: std::path::PathBuf,
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
        let voicepack = Voicepack::load(artifacts.voicepack)?;
        // Graph validation and warmup belong to the exact retained workers.
        Ok(Self {
            model: artifacts.model.into(),
            tokenizer,
            voicepack,
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
        let cancelled = std::sync::atomic::AtomicBool::new(false);
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
        let cancelled = std::sync::atomic::AtomicBool::new(false);
        worker.synthesize(
            &TtsSynthesisRequest {
                text: text.into(),
                selection: TtsBinding::readiness(),
            },
            &cancelled,
            on_pcm,
        )
    }

    fn open_worker(&self) -> Result<Box<dyn TtsWorker>, TtsError> {
        Ok(Box::new(KokoroViWorker::open(
            &self.model,
            &self.tokenizer,
            self.voicepack.clone(),
            &self.g2p_executable,
            &self.runtime_library,
            self.num_threads,
            self.speed,
        )?))
    }
}

#[cfg(test)]
mod real_model_tests {
    use std::path::PathBuf;

    use crate::{
        config::{KokoroViOnnxConfig, KokoroViRuntimeConfig, OnnxRuntimeConfig, RuntimeConfig},
        providers::tts::TtsProvider,
    };

    use super::{ConfiguredKokoroVi, KokoroViArtifacts};

    #[test]
    #[ignore = "requires models/tts/kokoro-vi and runtime/kokoro-vi local assets"]
    fn loads_local_contextbox_model_and_synthesizes_24khz_pcm() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../models/tts/kokoro-vi");
        let runtime_root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../runtime");
        let provider = ConfiguredKokoroVi::load(
            &KokoroViOnnxConfig::default(),
            &RuntimeConfig {
                onnx: OnnxRuntimeConfig {
                    library: runtime_root.join("onnxruntime/libonnxruntime.dylib"),
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
                voicepack: &root.join("voicepacks/diem_trinh.bin"),
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
