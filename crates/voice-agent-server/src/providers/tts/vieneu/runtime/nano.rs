use std::{
    path::Path,
    sync::{Arc, atomic::AtomicBool},
};

use ort::{
    session::{Session, SessionOutputs},
    value::{TensorElementType, ValueType},
};
use ring::rand::{SecureRandom, SystemRandom};

use super::{
    TtsError, check_cancelled, failure,
    graphs::tensor,
};
use crate::{
    audio::PcmF32Mono,
    providers::tts::{
        TtsBinding, TtsSynthesisRequest,
        vieneu::{
            assets::VieneuNanoAssets,
            config::VieneuNativeConfig,
            contract::{NANO_SPEAKER_DIM, NanoContract, NanoModelConfig},
            text::VieneuTextFrontend,
        },
    },
};

pub(super) const NANO_MAX_CHUNK_CHARS: usize = 140;

#[cfg(test)]
#[path = "nano_tests.rs"]
mod tests;

pub(in crate::providers::tts::vieneu) struct NanoNativeState {
    pub contract: NanoContract,
    pub frontend: VieneuTextFrontend,
}

impl NanoNativeState {
    pub fn load(assets: &VieneuNanoAssets) -> Result<Self, TtsError> {
        let contract = NanoContract::load(assets)?;
        let frontend = VieneuTextFrontend::load(&assets.g2p_dictionary)?;
        Ok(Self { contract, frontend })
    }
}

pub(in crate::providers::tts::vieneu) struct NanoWorker {
    state: Arc<NanoNativeState>,
    options: VieneuNativeConfig,
    text_encoder: Session,
    duration_predictor: Session,
    vector_estimator: Session,
    codec_decoder: Session,
    null_ctx: Vec<f32>,
}

fn load_nano_session(
    path: &Path,
    threads: usize,
    expected_inputs: &[(&str, TensorElementType, usize)],
    expected_output_rank: std::ops::RangeInclusive<usize>,
) -> Result<Session, TtsError> {
    let session = Session::builder()
        .map_err(failure)?
        .with_intra_threads(threads)
        .map_err(failure)?
        .with_inter_threads(1)
        .map_err(failure)?
        .commit_from_file(path)
        .map_err(failure)?;

    if session.inputs().len() != expected_inputs.len() || session.outputs().is_empty() {
        return Err(failure("Nano ONNX graph I/O count differs from contract"));
    }

    for (actual, (expected_name, expected_ty, expected_rank)) in
        session.inputs().iter().zip(expected_inputs.iter())
    {
        if actual.name() != *expected_name {
            return Err(failure("Nano ONNX input name differs from contract"));
        }
        let ValueType::Tensor { ty, shape, .. } = actual.dtype() else {
            return Err(failure("Nano ONNX input must be a tensor"));
        };
        if *ty != *expected_ty || shape.len() != *expected_rank {
            return Err(failure(
                "Nano ONNX input dtype or rank differs from contract",
            ));
        }
    }

    let ValueType::Tensor { ty, shape, .. } = session.outputs()[0].dtype() else {
        return Err(failure("Nano ONNX output must be a tensor"));
    };
    if *ty != TensorElementType::Float32 || !expected_output_rank.contains(&shape.len()) {
        return Err(failure(
            "Nano ONNX output dtype or rank differs from contract",
        ));
    }

    Ok(session)
}

fn extract_first_f32_tensor(
    output: &SessionOutputs<'_>,
    expected_len: Option<usize>,
) -> Result<Vec<f32>, TtsError> {
    if output.len() == 0 {
        return Err(TtsError::Failed);
    }
    let (name, value) = output.iter().next().ok_or(TtsError::Failed)?;
    let (shape, data) = value.try_extract_tensor::<f32>().map_err(failure)?;
    if data.is_empty()
        || expected_len.is_some_and(|len| data.len() != len)
        || data.iter().any(|v| !v.is_finite())
    {
        return Err(failure(format!(
            "invalid Nano ONNX output tensor `{name}` (shape={shape:?}, len={}, expected={expected_len:?}, non_finite={})",
            data.len(),
            data.iter().filter(|v| !v.is_finite()).count()
        )));
    }
    Ok(data.to_vec())
}

pub(super) fn split_text_into_chunks(text: &str, max_chars: usize) -> Vec<String> {
    let text = text.trim();
    if text.is_empty() || max_chars == 0 {
        return Vec::new();
    }

    let mut sentences = Vec::new();
    let mut current = String::new();
    for ch in text.chars() {
        current.push(ch);
        if matches!(ch, '.' | '!' | '?' | ';' | '\n') {
            let trimmed = current.trim();
            if !trimmed.is_empty() {
                sentences.push(trimmed.to_owned());
            }
            current.clear();
        }
    }
    let tail = current.trim();
    if !tail.is_empty() {
        sentences.push(tail.to_owned());
    }

    let mut chunks = Vec::new();
    let mut buf = String::new();
    for sentence in sentences {
        let sentence_len = sentence.chars().count();
        if sentence_len > max_chars {
            if !buf.trim().is_empty() {
                chunks.push(buf.trim().to_owned());
                buf.clear();
            }
            split_oversized_sentence(&sentence, max_chars, &mut chunks);
            continue;
        }
        let combined_len = if buf.is_empty() {
            sentence_len
        } else {
            buf.chars().count() + 1 + sentence_len
        };
        if combined_len <= max_chars {
            if !buf.is_empty() {
                buf.push(' ');
            }
            buf.push_str(&sentence);
        } else {
            if !buf.trim().is_empty() {
                chunks.push(buf.trim().to_owned());
            }
            buf = sentence;
        }
    }
    if !buf.trim().is_empty() {
        chunks.push(buf.trim().to_owned());
    }
    chunks
}

fn split_oversized_sentence(sentence: &str, max_chars: usize, out: &mut Vec<String>) {
    let mut current = String::new();
    for word in sentence.split_whitespace() {
        let word_len = word.chars().count();
        if word_len > max_chars {
            if !current.trim().is_empty() {
                out.push(current.trim().to_owned());
                current.clear();
            }
            let mut slice = String::new();
            let mut count = 0;
            for ch in word.chars() {
                slice.push(ch);
                count += 1;
                if count == max_chars {
                    out.push(slice.clone());
                    slice.clear();
                    count = 0;
                }
            }
            if !slice.is_empty() {
                current = slice;
            }
            continue;
        }
        let next_len = if current.is_empty() {
            word_len
        } else {
            current.chars().count() + 1 + word_len
        };
        if next_len <= max_chars {
            if !current.is_empty() {
                current.push(' ');
            }
            current.push_str(word);
        } else {
            out.push(current.trim().to_owned());
            current = word.to_owned();
        }
    }
    if !current.trim().is_empty() {
        out.push(current.trim().to_owned());
    }
}

pub(super) fn phonemes_to_ids(
    phonemes: &str,
    config: &NanoModelConfig,
) -> Result<(Vec<i64>, Vec<bool>), TtsError> {
    let mut ids = Vec::with_capacity(phonemes.chars().count() + 2);
    ids.push(config.bos_id);
    for ch in phonemes.chars() {
        let mut buf = [0_u8; 4];
        let key = ch.encode_utf8(&mut buf);
        if let Some(&id) = config.vocab.get(key) {
            ids.push(id);
        }
    }
    ids.push(config.eos_id);
    if ids.len() <= 2 {
        return Err(TtsError::Failed);
    }
    let mask = ids.iter().map(|&id| id != config.pad_id).collect();
    Ok((ids, mask))
}

/// Converts the predicted log utterance duration into latent frames at the
/// requested speaking rate. The natural duration keeps its predictor guard
/// (0.05–15 s); the flow model then spreads phonemes over the scaled length.
pub(super) fn latent_frames_for(log_s: f32, flow_fps: f32, speed: f32) -> usize {
    let natural_secs = log_s.exp().clamp(0.05, 15.0);
    let secs = natural_secs / speed;
    ((secs * flow_fps).round() as usize).max(2)
}

pub(super) fn trim_and_fade(samples: &[f32], sample_rate: u32) -> Vec<f32> {
    if samples.is_empty() {
        return Vec::new();
    }
    let clamped: Vec<f32> = samples.iter().map(|s| s.clamp(-1.0, 1.0)).collect();
    let win = ((0.010 * sample_rate as f32).round() as usize).max(1);
    let thresh = 10.0_f32.powf(-45.0 / 20.0);

    let mut trimmed = if clamped.len() >= win * 4 {
        let num_wins = clamped.len() / win;
        let mut first_active = None;
        let mut last_active = 0;
        for w in 0..num_wins {
            let slice = &clamped[w * win..(w + 1) * win];
            let mean_abs = slice.iter().map(|v| v.abs()).sum::<f32>() / win as f32;
            if mean_abs > thresh {
                if first_active.is_none() {
                    first_active = Some(w * win);
                }
                last_active = (w + 1) * win;
            }
        }
        if let Some(first) = first_active {
            let pad = (0.040 * sample_rate as f32).round() as usize;
            let start = first.saturating_sub(pad);
            let end = (last_active + pad).min(clamped.len());
            clamped[start..end].to_vec()
        } else {
            clamped
        }
    } else {
        clamped
    };

    let fade = ((0.015 * sample_rate as f32).round() as usize).min(trimmed.len() / 2);
    if fade > 1 {
        let total = trimmed.len();
        for i in 0..fade {
            let ramp = 0.5 * (1.0 - (std::f32::consts::PI * (i as f32 / (fade - 1) as f32)).cos());
            trimmed[i] *= ramp;
            trimmed[total - fade + i] *= 1.0 - ramp;
        }
    }
    trimmed
}

pub(super) fn sample_standard_normal(len: usize) -> Result<Vec<f32>, TtsError> {
    let mut seed_bytes = [0_u8; 8];
    SystemRandom::new()
        .fill(&mut seed_bytes)
        .map_err(|_| TtsError::Failed)?;
    let mut state = u64::from_le_bytes(seed_bytes);
    if state == 0 {
        state = 0x9E37_79B9_7F4A_7C15;
    }
    let mut next_f32 = || -> f32 {
        state ^= state >> 12;
        state ^= state << 25;
        state ^= state >> 27;
        let word = state.wrapping_mul(0x2545_F491_4F6C_DD1D);
        ((word >> 40) as u32 as f32) / ((1_u32 << 24) as f32)
    };
    let mut out = Vec::with_capacity(len);
    while out.len() < len {
        let u1 = next_f32().clamp(1e-7, 1.0 - 1e-7);
        let u2 = next_f32();
        let r = (-2.0_f32 * u1.ln()).sqrt();
        let theta = 2.0_f32 * std::f32::consts::PI * u2;
        out.push(r * theta.cos());
        if out.len() < len {
            out.push(r * theta.sin());
        }
    }
    Ok(out)
}

impl NanoWorker {
    pub fn open(
        state: Arc<NanoNativeState>,
        assets: &VieneuNanoAssets,
        options: &VieneuNativeConfig,
    ) -> Result<Self, TtsError> {
        let threads = usize::try_from(options.num_threads).map_err(failure)?;
        let config = &state.contract.config;
        let mut text_encoder = load_nano_session(
            &assets.text_encoder,
            threads,
            &[
                ("ids", TensorElementType::Int64, 2),
                ("style", TensorElementType::Float32, 3),
            ],
            3..=3,
        )?;
        let duration_predictor = load_nano_session(
            &assets.duration_predictor,
            threads,
            &[
                ("ctx", TensorElementType::Float32, 3),
                ("ctx_mask", TensorElementType::Bool, 2),
                ("spk", TensorElementType::Float32, 2),
            ],
            0..=2,
        )?;
        let vector_estimator = load_nano_session(
            &assets.vector_estimator,
            threads,
            &[
                ("x", TensorElementType::Float32, 3),
                ("t", TensorElementType::Float32, 1),
                ("ctx", TensorElementType::Float32, 3),
                ("ctx_mask", TensorElementType::Bool, 2),
                ("spk", TensorElementType::Float32, 2),
                ("style", TensorElementType::Float32, 3),
            ],
            3..=3,
        )?;
        let codec_decoder = load_nano_session(
            &assets.codec_decoder,
            threads,
            &[("x", TensorElementType::Float32, 3)],
            3..=3,
        )?;

        let null_ids = vec![config.bos_id, config.eos_id];
        let null_out = text_encoder
            .run(ort::inputs! {
                "ids" => tensor(vec![1, 2], null_ids)?,
                "style" => tensor(vec![1, config.n_style, config.style_dim], state.contract.null_style.clone())?,
            })
            .map_err(failure)?;
        let null_ctx = extract_first_f32_tensor(&null_out, Some(2 * config.style_dim))?;
        drop(null_out);

        Ok(Self {
            state,
            options: options.clone(),
            text_encoder,
            duration_predictor,
            vector_estimator,
            codec_decoder,
            null_ctx,
        })
    }

    /// Bounded startup warmup that touches each of the 4 ONNX graphs on the hot path once.
    pub fn warmup(&mut self) -> Result<(), TtsError> {
        let voice = self.state.contract.voices.get(&self.options.voice)?;
        let phonemes = self.state.frontend.phonemize("Xin chào.")?;
        let config = &self.state.contract.config;
        let (ids, mask) = phonemes_to_ids(&phonemes, config)?;
        let seq_len = ids.len();

        let enc_out = self
            .text_encoder
            .run(ort::inputs! {
                "ids" => tensor(vec![1, seq_len], ids)?,
                "style" => tensor(vec![1, config.n_style, config.style_dim], voice.style.clone())?,
            })
            .map_err(failure)?;
        let ctx = extract_first_f32_tensor(&enc_out, Some(seq_len * config.style_dim))?;
        drop(enc_out);

        let dur_out = self
            .duration_predictor
            .run(ort::inputs! {
                "ctx" => tensor(vec![1, seq_len, config.style_dim], ctx.clone())?,
                "ctx_mask" => tensor(vec![1, seq_len], mask.clone())?,
                "spk" => tensor(vec![1, NANO_SPEAKER_DIM], voice.speaker_embedding.clone())?,
            })
            .map_err(failure)?;
        let _ = extract_first_f32_tensor(&dur_out, Some(1))?;
        drop(dur_out);

        let channels = config.latent_channels();
        let warmup_frames = 4;
        let x = vec![0.0_f32; channels * warmup_frames];
        let vec_out = self
            .vector_estimator
            .run(ort::inputs! {
                "x" => tensor(vec![1, channels, warmup_frames], x)?,
                "t" => tensor(vec![1], vec![0.0_f32])?,
                "ctx" => tensor(vec![1, seq_len, config.style_dim], ctx)?,
                "ctx_mask" => tensor(vec![1, seq_len], mask)?,
                "spk" => tensor(vec![1, NANO_SPEAKER_DIM], voice.speaker_embedding.clone())?,
                "style" => tensor(vec![1, config.n_style, config.style_dim], voice.style.clone())?,
            })
            .map_err(failure)?;
        let v = extract_first_f32_tensor(&vec_out, Some(channels * warmup_frames))?;
        drop(vec_out);

        let dec_out = self
            .codec_decoder
            .run(ort::inputs! {
                "x" => tensor(vec![1, channels, warmup_frames], v)?,
            })
            .map_err(failure)?;
        let wav = extract_first_f32_tensor(&dec_out, None)?;
        if wav.is_empty() {
            return Err(TtsError::Failed);
        }
        Ok(())
    }

    pub fn synthesize(
        &mut self,
        request: &TtsSynthesisRequest,
        cancelled: &AtomicBool,
        on_pcm: &mut dyn FnMut(PcmF32Mono) -> Result<(), TtsError>,
    ) -> Result<(), TtsError> {
        check_cancelled(cancelled)?;
        let selection = if request.selection == TtsBinding::readiness() {
            TtsBinding {
                voice: self.options.voice.clone(),
                language: self.options.language.clone(),
            }
        } else {
            request.selection.clone()
        };
        if selection.language != "vi-VN" {
            return Err(TtsError::UnsupportedLanguage(selection.language));
        }
        let voice = self.state.contract.voices.get(&selection.voice)?.clone();
        let chunks = split_text_into_chunks(&request.text, NANO_MAX_CHUNK_CHARS);
        if chunks.is_empty() {
            return Err(TtsError::Failed);
        }

        for chunk in chunks {
            check_cancelled(cancelled)?;
            let pcm = self.synthesize_chunk(&chunk, &voice, cancelled)?;
            check_cancelled(cancelled)?;
            on_pcm(pcm)?;
        }
        Ok(())
    }

    fn synthesize_chunk(
        &mut self,
        chunk: &str,
        voice: &crate::providers::tts::vieneu::contract::NanoPresetVoice,
        cancelled: &AtomicBool,
    ) -> Result<PcmF32Mono, TtsError> {
        let phonemes = self.state.frontend.phonemize(chunk)?;
        let config = self.state.contract.config.clone();
        let (ids, mask) = phonemes_to_ids(&phonemes, &config)?;
        let seq_len = ids.len();

        check_cancelled(cancelled)?;
        let enc_out = self
            .text_encoder
            .run(ort::inputs! {
                "ids" => tensor(vec![1, seq_len], ids)?,
                "style" => tensor(vec![1, config.n_style, config.style_dim], voice.style.clone())?,
            })
            .map_err(failure)?;
        let ctx = extract_first_f32_tensor(&enc_out, Some(seq_len * config.style_dim))?;
        drop(enc_out);

        check_cancelled(cancelled)?;
        let dur_out = self
            .duration_predictor
            .run(ort::inputs! {
                "ctx" => tensor(vec![1, seq_len, config.style_dim], ctx.clone())?,
                "ctx_mask" => tensor(vec![1, seq_len], mask.clone())?,
                "spk" => tensor(vec![1, NANO_SPEAKER_DIM], voice.speaker_embedding.clone())?,
            })
            .map_err(failure)?;
        let log_s = extract_first_f32_tensor(&dur_out, Some(1))?[0];
        drop(dur_out);

        let latent_frames =
            latent_frames_for(log_s, config.flow_fps, self.options.speed_factor());
        let channels = config.latent_channels();
        let latent_len = channels * latent_frames;

        let mut x = sample_standard_normal(latent_len)?;
        let steps = config.steps_default;
        let dt = 1.0_f32 / steps as f32;
        let cfg = config.cfg_default;
        let null_mask = vec![true, true];
        let null_spk = self.state.contract.null_spk.clone();
        let null_style = self.state.contract.null_style.clone();
        let null_ctx = self.null_ctx.clone();

        for step in 0..steps {
            check_cancelled(cancelled)?;
            let t_val = step as f32 * dt;
            let cond_out = self
                .vector_estimator
                .run(ort::inputs! {
                    "x" => tensor(vec![1, channels, latent_frames], x.clone())?,
                    "t" => tensor(vec![1], vec![t_val])?,
                    "ctx" => tensor(vec![1, seq_len, config.style_dim], ctx.clone())?,
                    "ctx_mask" => tensor(vec![1, seq_len], mask.clone())?,
                    "spk" => tensor(vec![1, NANO_SPEAKER_DIM], voice.speaker_embedding.clone())?,
                    "style" => tensor(vec![1, config.n_style, config.style_dim], voice.style.clone())?,
                })
                .map_err(failure)?;
            let v_cond = extract_first_f32_tensor(&cond_out, Some(latent_len))?;
            drop(cond_out);

            if cfg > 0.0 {
                check_cancelled(cancelled)?;
                let uncond_out = self
                    .vector_estimator
                    .run(ort::inputs! {
                        "x" => tensor(vec![1, channels, latent_frames], x.clone())?,
                        "t" => tensor(vec![1], vec![t_val])?,
                        "ctx" => tensor(vec![1, 2, config.style_dim], null_ctx.clone())?,
                        "ctx_mask" => tensor(vec![1, 2], null_mask.clone())?,
                        "spk" => tensor(vec![1, NANO_SPEAKER_DIM], null_spk.clone())?,
                        "style" => tensor(vec![1, config.n_style, config.style_dim], null_style.clone())?,
                    })
                    .map_err(failure)?;
                let v_uncond = extract_first_f32_tensor(&uncond_out, Some(latent_len))?;
                drop(uncond_out);

                for ((x_i, &vc), &vu) in x.iter_mut().zip(&v_cond).zip(&v_uncond) {
                    let v = vu + cfg * (vc - vu);
                    *x_i += dt * v;
                }
            } else {
                for (x_i, &vc) in x.iter_mut().zip(&v_cond) {
                    *x_i += dt * vc;
                }
            }
        }

        check_cancelled(cancelled)?;
        let dec_out = self
            .codec_decoder
            .run(ort::inputs! {
                "x" => tensor(vec![1, channels, latent_frames], x)?,
            })
            .map_err(failure)?;
        let wav = extract_first_f32_tensor(&dec_out, None)?;
        drop(dec_out);

        let samples = trim_and_fade(&wav, config.sample_rate);
        if samples.is_empty() {
            return Err(TtsError::Failed);
        }
        Ok(PcmF32Mono::new(samples, config.sample_rate))
    }
}
