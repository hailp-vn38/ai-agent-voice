use std::{fs::File, path::Path};

use ndarray::{Array0, Array1, Array2, Array3, Axis};
use ndarray_npy::NpzReader;

use super::{TtsError, failure};
use crate::providers::tts::vieneu_native::contract::ModelConfig;

pub(super) struct Heads {
    pub text: Array2<f32>,
    pub audio: Array3<f32>,
    speaker: Array2<f32>,
    bias: Array1<f32>,
    norm_weight: Array1<f32>,
    norm_bias: Array1<f32>,
    epsilon: f32,
}

#[cfg(test)]
#[path = "heads_tests.rs"]
mod tests;

impl Heads {
    pub fn load(path: &Path, config: &ModelConfig) -> Result<Self, TtsError> {
        let mut archive = NpzReader::new(File::open(path).map_err(failure)?).map_err(failure)?;
        let epsilon: Array0<f32> = archive.by_name("xvec_ln_eps").map_err(failure)?;
        let heads = Self {
            text: archive.by_name("text_emb").map_err(failure)?,
            audio: archive.by_name("audio_emb").map_err(failure)?,
            speaker: archive.by_name("xvec_w").map_err(failure)?,
            bias: archive.by_name("xvec_b").map_err(failure)?,
            norm_weight: archive.by_name("xvec_ln_w").map_err(failure)?,
            norm_bias: archive.by_name("xvec_ln_b").map_err(failure)?,
            epsilon: epsilon[()],
        };
        let width = config.hidden_size;
        if heads.text.shape() != [config.text_vocab_size, width]
            || heads.audio.shape() != [config.n_vq, config.audio_vocab_size, width]
            || heads.speaker.shape() != [width, config.speaker_embedding_dim]
            || heads.bias.len() != width
            || heads.norm_weight.len() != width
            || heads.norm_bias.len() != width
            || !heads.epsilon.is_finite()
            || heads.epsilon <= 0.0
            || heads
                .text
                .iter()
                .chain(heads.audio.iter())
                .chain(heads.speaker.iter())
                .chain(heads.bias.iter())
                .chain(heads.norm_weight.iter())
                .chain(heads.norm_bias.iter())
                .any(|value| !value.is_finite())
        {
            return Err(failure(
                "embedding/head shapes differ from the pinned model",
            ));
        }
        Ok(heads)
    }

    pub fn anchor(&self, speaker: &[f32]) -> Result<Array1<f32>, TtsError> {
        if speaker.len() != self.speaker.ncols() || speaker.iter().any(|value| !value.is_finite()) {
            return Err(TtsError::Failed);
        }
        let mut anchor = self.speaker.dot(&Array1::from_vec(speaker.to_vec())) + &self.bias;
        let mean = anchor.sum() / anchor.len() as f32;
        let variance = anchor
            .iter()
            .map(|value| (value - mean).powi(2))
            .sum::<f32>()
            / anchor.len() as f32;
        let denominator = (variance + self.epsilon).sqrt();
        for ((value, weight), bias) in anchor
            .iter_mut()
            .zip(&self.norm_weight)
            .zip(&self.norm_bias)
        {
            *value = (*value - mean) / denominator * weight + bias;
        }
        if anchor.iter().any(|value| !value.is_finite()) {
            return Err(TtsError::Failed);
        }
        Ok(anchor)
    }

    pub fn embed(
        &self,
        text: usize,
        codes: &[i32],
        anchor: &Array1<f32>,
    ) -> Result<Vec<f32>, TtsError> {
        if text >= self.text.nrows()
            || (!codes.is_empty() && codes.len() != self.audio.len_of(Axis(0)))
        {
            return Err(TtsError::Failed);
        }
        let mut embedding = self.text.row(text).to_owned();
        for (channel, code) in codes.iter().enumerate() {
            let code = usize::try_from(*code).map_err(failure)?;
            if code >= self.audio.len_of(Axis(1)) {
                return Err(TtsError::Failed);
            }
            embedding += &self.audio.index_axis(Axis(0), channel).row(code);
        }
        embedding += anchor;
        Ok(embedding.to_vec())
    }
}
