use std::{
    collections::BTreeMap,
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

use ort::{session::Session, value::Tensor};

use crate::{
    audio::PcmF32Mono,
    providers::{
        tts::{TtsError, TtsSynthesisRequest, TtsWorker},
        vad::initialize_ort,
    },
};

use super::{ConfiguredKokoroVi, g2p::G2pSidecar, tokenizer::Tokenizer, voicepack::Voicepack};

const KOKORO_SAMPLE_RATE_HZ: u32 = 24_000;
const PCM_CHUNK_DURATION_MS: usize = 60;
const PCM_CHUNK_SAMPLES: usize = KOKORO_SAMPLE_RATE_HZ as usize * PCM_CHUNK_DURATION_MS / 1_000;

pub(super) struct KokoroViWorker {
    session: Session,
    tokenizer: Tokenizer,
    default_voice: String,
    voicepacks: Arc<BTreeMap<String, Voicepack>>,
    g2p: G2pSidecar,
    speed: f32,
}

impl KokoroViWorker {
    pub(super) fn open(config: &ConfiguredKokoroVi) -> Result<Self, TtsError> {
        if !config.speed.is_finite() || config.speed <= 0.0 {
            return Err(TtsError::Failed);
        }
        if !config.voicepacks.contains_key(&config.default_voice) {
            return Err(TtsError::UnsupportedVoice(config.default_voice.clone()));
        }
        let session = build_session(&config.model, &config.runtime_library, config.num_threads)?;
        Ok(Self {
            session,
            tokenizer: config.tokenizer.clone(),
            default_voice: config.default_voice.clone(),
            voicepacks: Arc::clone(&config.voicepacks),
            g2p: G2pSidecar::start(&config.g2p_executable)?,
            speed: config.speed,
        })
    }
}

fn build_session(
    model: &Path,
    runtime_library: &Path,
    num_threads: i32,
) -> Result<Session, TtsError> {
    // The application initializes exactly one deployment-selected ONNX Runtime.
    initialize_ort(runtime_library).map_err(|error| {
        TtsError::IncompatibleContract(format!(
            "Kokoro ONNX Runtime initialization failed: {error}"
        ))
    })?;
    Session::builder()
        .map_err(|error| TtsError::IncompatibleContract(error.to_string()))?
        .with_intra_threads(num_threads.try_into().map_err(|_| TtsError::Failed)?)
        .map_err(|error| TtsError::IncompatibleContract(error.to_string()))?
        .commit_from_file(model)
        .map_err(|error| TtsError::IncompatibleContract(error.to_string()))
}

impl TtsWorker for KokoroViWorker {
    fn warmup(&mut self) -> Result<(), TtsError> {
        crate::providers::tts::warmup_retained_worker(self, "Xin chào.", KOKORO_SAMPLE_RATE_HZ)
    }

    fn synthesize(
        &mut self,
        request: &TtsSynthesisRequest,
        cancelled: &AtomicBool,
        on_pcm: &mut dyn FnMut(PcmF32Mono) -> Result<(), TtsError>,
    ) -> Result<(), TtsError> {
        if cancelled.load(Ordering::Acquire) {
            return Err(TtsError::Failed);
        }
        let voice_id =
            if request.selection.voice.is_empty() || request.selection.voice == "__readiness__" {
                self.default_voice.as_str()
            } else {
                request.selection.voice.as_str()
            };
        let voicepack = self
            .voicepacks
            .get(voice_id)
            .ok_or_else(|| TtsError::UnsupportedVoice(voice_id.to_owned()))?;
        let ids = self.tokenizer.encode(&self.g2p.phonemes(&request.text)?)?;
        if cancelled.load(Ordering::Acquire) {
            return Err(TtsError::Failed);
        }
        let phoneme_count = ids.len();
        let waveform = self.session.run(ort::inputs! {
            "input_ids" => Tensor::<i64>::from_array(([1usize, ids.len()], ids)).map_err(|_| TtsError::Failed)?,
            "ref_s" => Tensor::<f32>::from_array(([1usize, 256], voicepack.latent(phoneme_count).to_vec())).map_err(|_| TtsError::Failed)?,
            "speed" => Tensor::<f32>::from_array(((), vec![self.speed])).map_err(|_| TtsError::Failed)?,
        }).map_err(|_| TtsError::Failed)?;
        if cancelled.load(Ordering::Acquire) {
            return Err(TtsError::Failed);
        }
        let samples = waveform["waveform"]
            .try_extract_tensor::<f32>()
            .map_err(|_| TtsError::Failed)?
            .1;
        emit_pcm_chunks(samples, cancelled, on_pcm)
    }

    fn reset(&mut self) -> Result<(), TtsError> {
        Ok(())
    }
}

/// The exported graph returns one complete waveform. Emit bounded PCM only after inference so
/// `SpeechOutput` can preserve its audio high-water mark without claiming model-level streaming.
fn emit_pcm_chunks(
    samples: &[f32],
    cancelled: &AtomicBool,
    on_pcm: &mut dyn FnMut(PcmF32Mono) -> Result<(), TtsError>,
) -> Result<(), TtsError> {
    if samples.is_empty() || samples.iter().any(|sample| !sample.is_finite()) {
        return Err(TtsError::Failed);
    }
    for chunk in samples.chunks(PCM_CHUNK_SAMPLES) {
        if cancelled.load(Ordering::Acquire) {
            return Err(TtsError::Failed);
        }
        on_pcm(PcmF32Mono::new(chunk.to_vec(), KOKORO_SAMPLE_RATE_HZ))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests;
