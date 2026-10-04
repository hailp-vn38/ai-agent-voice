//! ZeroTTS model inference and streaming 48 kHz PCM for native TTS workers.

mod codec;
mod contract;
mod synthesis;
pub mod text;

use ndarray::{Array0, Array3};
use ndarray_npy::NpzReader;
use ort::{
    session::Session,
    value::{Tensor, TensorElementType, ValueType},
};
use serde::Deserialize;
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    sync::{
        Arc, OnceLock,
        atomic::{AtomicU64, Ordering},
    },
};
use tokenizers::Tokenizer;
use unicode_normalization::UnicodeNormalization;

use super::super::TtsError;
use crate::{audio::PcmF32Mono, providers::vad::initialize_ort};
use codec::*;
pub use contract::ZeroTtsContract;
use contract::*;
use synthesis::*;
pub use synthesis::{CodeFrames, ZeroTtsFullPcm, ZeroTtsPcmStream};

pub use text::normalize_text;

/// Immutable, deterministic in-memory registry of verified voice embeddings.
#[derive(Default)]
pub struct ZeroTtsVoiceRegistry {
    voices: BTreeMap<String, Arc<Array3<f32>>>,
}

impl ZeroTtsVoiceRegistry {
    pub(super) fn from_legacy(voice: Array3<f32>) -> Self {
        Self {
            voices: BTreeMap::from([("__legacy__".into(), Arc::new(voice))]),
        }
    }

    pub fn get_arc(&self, id: &str) -> Result<Arc<Array3<f32>>, TtsError> {
        self.voices
            .get(id)
            .cloned()
            .ok_or_else(|| TtsError::UnsupportedVoice(id.into()))
    }

    pub fn first(&self) -> Option<&Array3<f32>> {
        self.voices.values().next().map(AsRef::as_ref)
    }

    pub fn first_arc(&self) -> Option<Arc<Array3<f32>>> {
        self.voices.values().next().cloned()
    }

    pub fn first_id(&self) -> Option<&str> {
        self.voices.keys().next().map(String::as_str)
    }

    pub(super) fn load(
        index_path: &Path,
        expected: &BTreeMap<String, PathBuf>,
        config: &Config,
    ) -> Result<Self, TtsError> {
        #[derive(Deserialize)]
        struct Index {
            voices: Vec<VoiceMetadata>,
        }
        #[derive(Deserialize)]
        struct VoiceMetadata {
            name: String,
            shape: Vec<usize>,
        }

        let index: Index = serde_json::from_slice(&fs::read(index_path).map_err(contract_error)?)
            .map_err(contract_error)?;
        let mut indexed = BTreeMap::new();
        for voice in index.voices {
            if voice.name.trim().is_empty() || indexed.insert(voice.name, voice.shape).is_some() {
                return Err(TtsError::IncompatibleContract(
                    "ZeroTTS voices index contains duplicate or empty voice id".into(),
                ));
            }
        }
        if indexed.keys().ne(expected.keys()) {
            return Err(TtsError::IncompatibleContract(
                "ZeroTTS descriptor voices, index, and manifest roles differ".into(),
            ));
        }
        let expected_shape = [1, config.n_voice_queries, config.d_model];
        let mut voices = BTreeMap::new();
        for (id, path) in expected {
            if indexed
                .get(id)
                .is_none_or(|shape| shape.as_slice() != expected_shape)
            {
                return Err(TtsError::IncompatibleContract(
                    "ZeroTTS voice index shape does not match the pinned model".into(),
                ));
            }
            voices.insert(id.clone(), Arc::new(load_voice(path, config)?));
        }
        Ok(Self { voices })
    }
}

fn contract_error(error: impl std::fmt::Display) -> TtsError {
    TtsError::IncompatibleContract(error.to_string())
}

/// Process-local count of committed ONNX sessions. Regression tests use it to prove one physical
/// replica retains each graph exactly once across many turns; the inference path never reads it.
static SESSION_CONSTRUCTIONS: AtomicU64 = AtomicU64::new(0);

/// ONNX sessions committed by this process since it started.
pub fn session_constructions() -> u64 {
    SESSION_CONSTRUCTIONS.load(Ordering::Acquire)
}

pub(super) fn record_session_construction() {
    SESSION_CONSTRUCTIONS.fetch_add(1, Ordering::AcqRel);
}
