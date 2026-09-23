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
    sync::OnceLock,
};
use tokenizers::Tokenizer;
use unicode_normalization::UnicodeNormalization;

use super::TtsError;
use crate::{audio::PcmF32Mono, providers::vad::initialize_ort};
use codec::*;
pub use contract::ZeroTtsContract;
use contract::*;
use synthesis::*;
pub use synthesis::{CodeFrames, ZeroTtsPcmStream};

pub use text::normalize_text;

fn contract_error(error: impl std::fmt::Display) -> TtsError {
    TtsError::IncompatibleContract(error.to_string())
}
