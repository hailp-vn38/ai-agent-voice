use std::{
    path::Path,
    sync::atomic::{AtomicBool, Ordering},
};

use ort::{session::Session, value::Tensor};

use crate::{
    audio::PcmF32Mono,
    providers::{
        tts::{TtsError, TtsWorker},
        vad::initialize_ort,
    },
};

use super::{g2p::G2pSidecar, tokenizer::Tokenizer, voicepack::Voicepack};

const KOKORO_SAMPLE_RATE_HZ: u32 = 24_000;
const PCM_CHUNK_DURATION_MS: usize = 60;
const PCM_CHUNK_SAMPLES: usize = KOKORO_SAMPLE_RATE_HZ as usize * PCM_CHUNK_DURATION_MS / 1_000;

pub(super) struct KokoroViWorker {
    session: Session,
    tokenizer: Tokenizer,
    voicepack: Voicepack,
    g2p: G2pSidecar,
    speed: f32,
}

impl KokoroViWorker {
    pub(super) fn open(
        model: &Path,
        tokenizer: &Tokenizer,
        voicepack: Voicepack,
        g2p_executable: &Path,
        runtime_library: &Path,
        num_threads: i32,
        speed: f32,
    ) -> Result<Self, TtsError> {
        if !speed.is_finite() || speed <= 0.0 {
            return Err(TtsError::Failed);
        }
        let session = build_session(model, runtime_library, num_threads)?;
        Ok(Self {
            session,
            tokenizer: tokenizer.clone(),
            voicepack,
            g2p: G2pSidecar::start(g2p_executable)?,
            speed,
        })
    }
}

/// Validate the graph while providers are loading, before the server binds a listener.
pub(super) fn verify_model(
    model: &Path,
    runtime_library: &Path,
    num_threads: i32,
) -> Result<(), TtsError> {
    drop(build_session(model, runtime_library, num_threads)?);
    Ok(())
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
    fn synthesize(
        &mut self,
        text: &str,
        cancelled: &AtomicBool,
        on_pcm: &mut dyn FnMut(PcmF32Mono) -> Result<(), TtsError>,
    ) -> Result<(), TtsError> {
        if cancelled.load(Ordering::Acquire) {
            return Err(TtsError::Failed);
        }
        let ids = self.tokenizer.encode(&self.g2p.phonemes(text)?)?;
        if cancelled.load(Ordering::Acquire) {
            return Err(TtsError::Failed);
        }
        let phoneme_count = ids.len();
        let waveform = self.session.run(ort::inputs! {
            "input_ids" => Tensor::<i64>::from_array(([1usize, ids.len()], ids)).map_err(|_| TtsError::Failed)?,
            "ref_s" => Tensor::<f32>::from_array(([1usize, 256], self.voicepack.latent(phoneme_count).to_vec())).map_err(|_| TtsError::Failed)?,
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
