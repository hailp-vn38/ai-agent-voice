//! Retained ONNX worker for ViFastTts (`acoustic.onnx` + `vocoder.onnx`).

use std::{
    path::Path,
    sync::atomic::{AtomicBool, Ordering},
};

use ort::{session::Session, value::Tensor};

use crate::{
    audio::PcmF32Mono,
    providers::{
        tts::{TtsBinding, TtsError, TtsSynthesisRequest, TtsWorker},
        vad::initialize_ort,
    },
};

use super::{assets::VOICES, dsp, frontend};

pub const VIFASTTTS_SAMPLE_RATE_HZ: u32 = 24_000;
const PCM_CHUNK_DURATION_MS: usize = 60;
pub const PCM_CHUNK_SAMPLES: usize =
    VIFASTTTS_SAMPLE_RATE_HZ as usize * PCM_CHUNK_DURATION_MS / 1_000;

const N_FFT: usize = 1_024;
const HOP_LENGTH: usize = 256;
const FREQ_BINS: usize = N_FFT / 2 + 1;
const N_MELS: usize = 100;
const GATE_THRESHOLD: f32 = 0.5;

pub struct VifastttsWorker {
    acoustic: Session,
    vocoder: Session,
    speed: f32,
}

impl VifastttsWorker {
    pub(super) fn open(
        acoustic_model: &Path,
        vocoder_model: &Path,
        runtime_library: &Path,
        num_threads: i32,
        speed: f32,
    ) -> Result<Self, TtsError> {
        if !speed.is_finite() || speed <= 0.0 {
            return Err(TtsError::Failed);
        }
        let acoustic = build_session(acoustic_model, runtime_library, num_threads)?;
        if let Ok(meta) = acoustic.metadata()
            && let Some(onset_has_q) = meta.custom("onset_has_q")
            && onset_has_q.trim().to_ascii_lowercase() != "true"
        {
            return Err(TtsError::IncompatibleContract(
                "ViFastTts acoustic ONNX model requires onset_has_q=true".into(),
            ));
        }
        let vocoder = build_session(vocoder_model, runtime_library, num_threads)?;
        Ok(Self {
            acoustic,
            vocoder,
            speed,
        })
    }
}

fn build_session(
    model: &Path,
    runtime_library: &Path,
    num_threads: i32,
) -> Result<Session, TtsError> {
    initialize_ort(runtime_library).map_err(|error| {
        TtsError::IncompatibleContract(format!(
            "ViFastTts ONNX Runtime initialization failed: {error}"
        ))
    })?;
    Session::builder()
        .map_err(|error| TtsError::IncompatibleContract(error.to_string()))?
        .with_intra_threads(num_threads.try_into().map_err(|_| TtsError::Failed)?)
        .map_err(|error| TtsError::IncompatibleContract(error.to_string()))?
        .commit_from_file(model)
        .map_err(|error| TtsError::IncompatibleContract(error.to_string()))
}

impl TtsWorker for VifastttsWorker {
    fn warmup(&mut self) -> Result<(), TtsError> {
        crate::providers::tts::warmup_retained_worker(self, "Xin chào.", VIFASTTTS_SAMPLE_RATE_HZ)
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
        validate_selection(&request.selection)?;

        let text = request.text.trim();
        if text.is_empty() || text.len() > 8_192 || text.chars().any(|c| c == '\0') {
            return Err(TtsError::Failed);
        }

        let normalized = frontend::normalize_text(text);
        let chunks = frontend::chunk_text(&normalized, frontend::MAX_CHUNK_CHARS);
        if chunks.is_empty() {
            return Err(TtsError::Failed);
        }

        let mut emitted = false;
        for (idx, chunk) in chunks.iter().enumerate() {
            if cancelled.load(Ordering::Acquire) {
                return Err(TtsError::Failed);
            }
            let rows = frontend::features_from_text(chunk);
            if rows.is_empty() {
                continue;
            }
            let num_syllables = rows.len();
            let mut flat = Vec::with_capacity(num_syllables * frontend::NUM_FEATURES);
            for row in rows {
                flat.extend_from_slice(&row);
            }

            let (frames, mut mel) = {
                let acoustic_outputs = self
                    .acoustic
                    .run(ort::inputs! {
                        "features" => Tensor::<i64>::from_array(([1usize, num_syllables, frontend::NUM_FEATURES], flat)).map_err(|_| TtsError::Failed)?,
                        "speed" => Tensor::<f32>::from_array(([1usize], vec![self.speed])).map_err(|_| TtsError::Failed)?,
                        "gate_threshold" => Tensor::<f32>::from_array(([1usize], vec![GATE_THRESHOLD])).map_err(|_| TtsError::Failed)?,
                    })
                    .map_err(|_| TtsError::Failed)?;
                let mel_slice = acoustic_outputs["mel"]
                    .try_extract_tensor::<f32>()
                    .map_err(|_| TtsError::Failed)?
                    .1;
                let frames = mel_slice.len() / N_MELS;
                if frames < 2 || mel_slice.len() != frames * N_MELS {
                    return Err(TtsError::Failed);
                }
                (frames, mel_slice.to_vec())
            };

            if cancelled.load(Ordering::Acquire) {
                return Err(TtsError::Failed);
            }

            dsp::sharpen_mel(&mut mel, frames, N_MELS, dsp::DEFAULT_SHARPEN_ALPHA);

            let mut wav = {
                let vocoder_outputs = self
                    .vocoder
                    .run(ort::inputs! {
                        "mel" => Tensor::<f32>::from_array(([1usize, frames, N_MELS], mel)).map_err(|_| TtsError::Failed)?,
                    })
                    .map_err(|_| TtsError::Failed)?;
                let spec_real = vocoder_outputs["spec_real"]
                    .try_extract_tensor::<f32>()
                    .map_err(|_| TtsError::Failed)?
                    .1;
                let spec_imag = vocoder_outputs["spec_imag"]
                    .try_extract_tensor::<f32>()
                    .map_err(|_| TtsError::Failed)?
                    .1;
                dsp::istft(spec_real, spec_imag, FREQ_BINS, frames, N_FFT, HOP_LENGTH)
            };

            if cancelled.load(Ordering::Acquire) || wav.is_empty() {
                return Err(TtsError::Failed);
            }

            dsp::fade(
                &mut wav,
                VIFASTTTS_SAMPLE_RATE_HZ,
                dsp::DEFAULT_FADE_IN_MS,
                dsp::DEFAULT_FADE_OUT_MS,
            );

            if idx + 1 < chunks.len() {
                let pause_samples =
                    (frontend::pause_seconds(chunk) * (VIFASTTTS_SAMPLE_RATE_HZ as f32)) as usize;
                if pause_samples > 0 {
                    wav.resize(wav.len() + pause_samples, 0.0);
                }
            }

            emit_pcm_chunks(&wav, cancelled, on_pcm)?;
            emitted = true;
        }

        if !emitted {
            return Err(TtsError::Failed);
        }
        Ok(())
    }

    fn reset(&mut self) -> Result<(), TtsError> {
        Ok(())
    }
}

pub fn validate_selection(selection: &TtsBinding) -> Result<(), TtsError> {
    if *selection == TtsBinding::readiness() {
        return Ok(());
    }
    if !selection.language.is_empty() && selection.language != "vi-VN" {
        return Err(TtsError::UnsupportedLanguage(selection.language.clone()));
    }
    if !selection.voice.is_empty() && !VOICES.iter().any(|v| v.id == selection.voice) {
        return Err(TtsError::UnsupportedVoice(selection.voice.clone()));
    }
    Ok(())
}

pub fn emit_pcm_chunks(
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
        on_pcm(PcmF32Mono::new(chunk.to_vec(), VIFASTTTS_SAMPLE_RATE_HZ))?;
    }
    Ok(())
}
