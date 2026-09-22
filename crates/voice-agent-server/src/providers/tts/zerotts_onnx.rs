//! ZeroTTS immutable model contract and per-operation synthesis inputs.

use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
};

use ndarray::{Array0, Array3};
use ndarray_npy::NpzReader;
use ort::{
    session::Session,
    value::{Tensor, TensorElementType, ValueType},
};
use serde::Deserialize;
use tokenizers::Tokenizer;
use unicode_normalization::UnicodeNormalization;

use crate::providers::vad::initialize_ort;

use super::TtsError;

const SPECIAL_TOKENS: [(&str, u32); 8] = [
    ("<pad>", 0),
    ("<bos>", 1),
    ("<eot>", 2),
    ("<soa>", 3),
    ("<slot>", 4),
    ("<eoa>", 5),
    ("<en>", 6),
    ("<vi>", 7),
];

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Config {
    text_format: String,
    vocab_size: usize,
    num_codebooks: usize,
    codebook_size: usize,
    d_model: usize,
    n_heads: usize,
    n_layers: usize,
    n_voice_queries: usize,
    sample_rate: u32,
    codec_frame_rate: f32,
    special_tokens: BTreeMap<String, u32>,
}

/// Immutable values shared by every synthesis operation for one pinned pack.
pub struct ZeroTtsContract {
    tokenizer: Tokenizer,
    config: Config,
    voice: Option<Array3<f32>>,
    graphs: Option<GraphPaths>,
}

/// Immutable graph sources. Sessions are created by `ZeroTtsOperation`, never shared.
struct GraphPaths {
    text_encoder: PathBuf,
    prefix_step: PathBuf,
    local_frame_decode: PathBuf,
    threads: usize,
}

impl ZeroTtsContract {
    pub fn load(config_path: &Path, tokenizer_path: &Path) -> Result<Self, TtsError> {
        let config: Config =
            serde_json::from_slice(&fs::read(config_path).map_err(contract_error)?)
                .map_err(contract_error)?;
        validate_config(&config)?;
        let tokenizer_bytes = fs::read(tokenizer_path).map_err(contract_error)?;
        let tokenizer = Tokenizer::from_bytes(&tokenizer_bytes)
            .map_err(|error| TtsError::IncompatibleContract(error.to_string()))?;
        let tokenizer_document: serde_json::Value =
            serde_json::from_slice(&tokenizer_bytes).map_err(contract_error)?;
        for (token, expected) in SPECIAL_TOKENS {
            if tokenizer_special_id(&tokenizer_document, token) != Some(expected) {
                return Err(TtsError::IncompatibleContract(format!(
                    "tokenizer special token {token} is not {expected}"
                )));
            }
        }
        Ok(Self {
            tokenizer,
            config,
            voice: None,
            graphs: None,
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub fn load_engine(
        config_path: &Path,
        tokenizer_path: &Path,
        voice_path: &Path,
        text_encoder_path: &Path,
        prefix_step_path: &Path,
        local_frame_decode_path: &Path,
        runtime_library: &Path,
        num_threads: i32,
    ) -> Result<Self, TtsError> {
        let config: Config =
            serde_json::from_slice(&fs::read(config_path).map_err(contract_error)?)
                .map_err(contract_error)?;
        validate_config(&config)?;
        let tokenizer = load_tokenizer(tokenizer_path)?;
        let voice = load_voice(voice_path, &config)?;
        initialize_ort(runtime_library)
            .map_err(|error| TtsError::IncompatibleContract(error.to_string()))?;
        let threads = usize::try_from(num_threads).map_err(contract_error)?;
        if threads == 0 {
            return Err(TtsError::IncompatibleContract(
                "ZeroTTS thread count must be positive".into(),
            ));
        }
        load_graph(
            text_encoder_path,
            threads,
            TEXT_ENCODER_INPUTS,
            TEXT_ENCODER_OUTPUTS,
        )?;
        load_graph(
            prefix_step_path,
            threads,
            PREFIX_STEP_INPUTS,
            PREFIX_STEP_OUTPUTS,
        )?;
        load_graph(
            local_frame_decode_path,
            threads,
            LOCAL_FRAME_INPUTS,
            LOCAL_FRAME_OUTPUTS,
        )?;
        Ok(Self {
            tokenizer,
            config,
            voice: Some(voice),
            graphs: Some(GraphPaths {
                text_encoder: text_encoder_path.into(),
                prefix_step: prefix_step_path.into(),
                local_frame_decode: local_frame_decode_path.into(),
                threads,
            }),
        })
    }

    /// Encodes one operation's text; no cache survives this call.
    pub fn encode(&self, text: &str) -> Result<Vec<i64>, TtsError> {
        let encoding = self
            .tokenizer
            .encode(normalize_text(text), false)
            .map_err(|error| TtsError::IncompatibleContract(error.to_string()))?;
        let mut ids = Vec::with_capacity(encoding.len() + 2);
        ids.push(1);
        ids.extend(encoding.get_ids().iter().map(|id| i64::from(*id)));
        ids.push(2);
        Ok(ids)
    }

    pub fn frame_position(&self, frame_index: usize) -> i64 {
        (self.config.n_voice_queries + 1 + frame_index) as i64
    }

    pub fn voice_shape(&self) -> Option<&[usize]> {
        self.voice.as_ref().map(ndarray::ArrayBase::shape)
    }

    pub fn synthesize_codes(&self, text: &str, max_frames: usize) -> Result<CodeFrames, TtsError> {
        ZeroTtsOperation::new(self)?.synthesize(text, max_frames)
    }
}

/// Mutable ONNX sessions and turn-local decode state for exactly one synthesis operation.
struct ZeroTtsOperation<'a> {
    contract: &'a ZeroTtsContract,
    text_encoder: Session,
    prefix_step: Session,
    local_frame_decode: Session,
}

impl<'a> ZeroTtsOperation<'a> {
    fn new(contract: &'a ZeroTtsContract) -> Result<Self, TtsError> {
        let graphs = contract
            .graphs
            .as_ref()
            .ok_or_else(|| TtsError::IncompatibleContract("ZeroTTS engine is not loaded".into()))?;
        Ok(Self {
            contract,
            text_encoder: load_graph(
                &graphs.text_encoder,
                graphs.threads,
                TEXT_ENCODER_INPUTS,
                TEXT_ENCODER_OUTPUTS,
            )?,
            prefix_step: load_graph(
                &graphs.prefix_step,
                graphs.threads,
                PREFIX_STEP_INPUTS,
                PREFIX_STEP_OUTPUTS,
            )?,
            local_frame_decode: load_graph(
                &graphs.local_frame_decode,
                graphs.threads,
                LOCAL_FRAME_INPUTS,
                LOCAL_FRAME_OUTPUTS,
            )?,
        })
    }

    fn synthesize(&mut self, text: &str, max_frames: usize) -> Result<CodeFrames, TtsError> {
        let ids = self.contract.encode(text)?;
        let voice = self.contract.voice.as_ref().expect("engine invariant");
        let text_out = self
            .text_encoder
            .run(ort::inputs! {
                "text_ids" => tensor(vec![1, ids.len()], ids.clone())?,
                "txt_lengths" => tensor(vec![1], vec![ids.len() as i64])?,
            })
            .map_err(contract_error)?;
        let states = f32_tensor(&text_out["text_states"])?;
        let text_valid = bool_tensor(&text_out["text_valid"])?;
        let soa = f32_tensor(&text_out["soa_embed"])?;
        let cross = f32_tensor(&text_out["cross_kv"])?;
        drop(text_out);
        let mut embed = voice.iter().copied().collect::<Vec<_>>();
        embed.extend(soa.data);
        let voice_queries = self.contract.config.n_voice_queries;
        let mut prefix_out = self.prefix_step.run(ort::inputs! {
            "external_embed" => tensor(vec![1, voice_queries + 1, self.contract.config.d_model], embed)?,
            "use_external_embed" => tensor(vec![1, voice_queries + 1], vec![true; voice_queries + 1])?,
            "frame_codes" => tensor(vec![1, voice_queries + 1, self.contract.config.num_codebooks], vec![0_i64; (voice_queries + 1) * self.contract.config.num_codebooks])?,
            "new_pos" => tensor(vec![1, voice_queries + 1], (0..=voice_queries).map(|x| x as i64).collect::<Vec<_>>())?,
            "new_valid" => tensor(vec![1, voice_queries + 1], vec![true; voice_queries + 1])?,
            "packed_kv" => tensor(vec![self.contract.config.n_layers, 2, 1, self.contract.config.n_heads, 0, self.contract.config.d_model / self.contract.config.n_heads], Vec::<f32>::new())?,
            "new_bidirectional" => tensor(vec![1, voice_queries + 1], (0..voice_queries).map(|_| true).chain(std::iter::once(false)).collect::<Vec<_>>())?,
            "past_valid" => tensor(vec![1, 0], Vec::<bool>::new())?,
            "cross_kv" => tensor(cross.shape.clone(), cross.data.clone())?,
            "text_valid" => tensor(text_valid.shape.clone(), text_valid.data.clone())?,
        }).map_err(contract_error)?;
        let mut hidden = final_hidden(&prefix_out["hidden"], self.contract.config.d_model)?;
        let mut kv = f32_tensor(&prefix_out["new_packed_kv"])?;
        let mut valid = bool_tensor(&prefix_out["full_valid"])?;
        drop(prefix_out);
        let mut seen =
            vec![false; self.contract.config.num_codebooks * self.contract.config.codebook_size];
        let mut frames = Vec::new();
        let mut frame_positions = Vec::new();
        let mut seen_code_counts = Vec::new();
        let mut eoa = None;
        for frame_index in 0..max_frames {
            let decoded = self.local_frame_decode.run(ort::inputs! {
                "global_hidden" => tensor(vec![1, self.contract.config.d_model], hidden)?,
                "forbid_eoa" => tensor(vec![1], vec![frame_index == 0])?, "text_temperature" => tensor(vec![1], vec![1.0_f32])?, "text_topk" => tensor(vec![1], vec![50_i64])?,
                "audio_temperature" => tensor(vec![1], vec![0.8_f32])?, "audio_topk" => tensor(vec![1], vec![25_i64])?, "audio_topp" => tensor(vec![1], vec![0.95_f32])?, "audio_repetition_penalty" => tensor(vec![1], vec![1.2_f32])?,
                "seen_mask" => tensor(vec![1, self.contract.config.num_codebooks, self.contract.config.codebook_size], seen.clone())?,
                "ctrl_random_u" => tensor(vec![1], vec![draw(frame_index, 0)])?, "audio_random_u" => tensor(vec![1, self.contract.config.num_codebooks], (0..self.contract.config.num_codebooks).map(|n| draw(frame_index, n + 1)).collect::<Vec<_>>())?, "cfg_scale" => tensor(vec![1], vec![1.0_f32])?,
            }).map_err(contract_error)?;
            let codes = i32_tensor(&decoded["codes"])?.data;
            for (book, code) in codes.iter().enumerate() {
                let code = usize::try_from(*code).map_err(contract_error)?;
                if code >= self.contract.config.codebook_size {
                    return Err(TtsError::IncompatibleContract(
                        "out-of-range audio code".into(),
                    ));
                }
                seen[book * self.contract.config.codebook_size + code] = true;
            }
            let is_eoa = i32_tensor(&decoded["is_eoa"])?.data[0] != 0;
            frames.push(codes.clone());
            frame_positions.push(self.contract.frame_position(frame_index));
            seen_code_counts.push(seen.iter().filter(|value| **value).count());
            drop(decoded);
            if is_eoa {
                eoa = Some(frame_index);
                break;
            }
            let d_model = self.contract.config.d_model;
            let codebooks = self.contract.config.num_codebooks;
            let position = self.contract.frame_position(frame_index);
            prefix_out = self.prefix_step.run(ort::inputs! {
                "external_embed" => tensor(vec![1, 1, d_model], vec![0.0_f32; d_model])?, "use_external_embed" => tensor(vec![1, 1], vec![false])?, "frame_codes" => tensor(vec![1, 1, codebooks], codes.into_iter().map(i64::from).collect::<Vec<_>>())?, "new_pos" => tensor(vec![1, 1], vec![position])?, "new_valid" => tensor(vec![1, 1], vec![true])?, "packed_kv" => tensor(kv.shape, kv.data)?, "new_bidirectional" => tensor(vec![1, 1], vec![false])?, "past_valid" => tensor(valid.shape, valid.data)?, "cross_kv" => tensor(cross.shape.clone(), cross.data.clone())?, "text_valid" => tensor(text_valid.shape.clone(), text_valid.data.clone())?,
            }).map_err(contract_error)?;
            hidden = final_hidden(&prefix_out["hidden"], self.contract.config.d_model)?;
            kv = f32_tensor(&prefix_out["new_packed_kv"])?;
            valid = bool_tensor(&prefix_out["full_valid"])?;
            drop(prefix_out);
        }
        Ok(CodeFrames {
            token_ids: ids,
            text_state_checksum: checksum(&states.data),
            frames,
            frame_positions,
            seen_code_counts,
            eoa,
        })
    }
}

pub struct CodeFrames {
    pub token_ids: Vec<i64>,
    pub text_state_checksum: f64,
    pub frames: Vec<Vec<i32>>,
    pub frame_positions: Vec<i64>,
    pub seen_code_counts: Vec<usize>,
    pub eoa: Option<usize>,
}
struct Data<T> {
    shape: Vec<usize>,
    data: Vec<T>,
}
fn tensor<T: ort::value::PrimitiveTensorElementType + Clone + std::fmt::Debug + 'static>(
    shape: Vec<usize>,
    data: Vec<T>,
) -> Result<Tensor<T>, TtsError> {
    Tensor::from_array((shape, data)).map_err(contract_error)
}
fn f32_tensor(value: &ort::value::DynValue) -> Result<Data<f32>, TtsError> {
    let (s, d) = value.try_extract_tensor::<f32>().map_err(contract_error)?;
    Ok(Data {
        shape: s.iter().map(|x| *x as usize).collect(),
        data: d.to_vec(),
    })
}
fn bool_tensor(value: &ort::value::DynValue) -> Result<Data<bool>, TtsError> {
    let (s, d) = value.try_extract_tensor::<bool>().map_err(contract_error)?;
    Ok(Data {
        shape: s.iter().map(|x| *x as usize).collect(),
        data: d.to_vec(),
    })
}
fn i32_tensor(value: &ort::value::DynValue) -> Result<Data<i32>, TtsError> {
    let (s, d) = value.try_extract_tensor::<i32>().map_err(contract_error)?;
    Ok(Data {
        shape: s.iter().map(|x| *x as usize).collect(),
        data: d.to_vec(),
    })
}
fn final_hidden(value: &ort::value::DynValue, width: usize) -> Result<Vec<f32>, TtsError> {
    let data = f32_tensor(value)?.data;
    data.get(data.len().saturating_sub(width)..)
        .map(ToOwned::to_owned)
        .ok_or_else(|| TtsError::IncompatibleContract("missing hidden output".into()))
}
fn checksum(values: &[f32]) -> f64 {
    values
        .iter()
        .enumerate()
        .map(|(i, x)| f64::from(*x) * (i + 1) as f64)
        .sum()
}
fn draw(frame: usize, stream: usize) -> f32 {
    let mut x = (frame as u64 + 1).wrapping_mul(0x9E3779B97F4A7C15) ^ stream as u64;
    x ^= x >> 30;
    x = x.wrapping_mul(0xBF58476D1CE4E5B9);
    (x >> 40) as f32 / (1_u32 << 24) as f32
}

fn tokenizer_special_id(document: &serde_json::Value, token: &str) -> Option<u32> {
    document
        .get("added_tokens")?
        .as_array()?
        .iter()
        .find_map(|entry| {
            (entry.get("content")?.as_str() == Some(token))
                .then(|| entry.get("id")?.as_u64()?.try_into().ok())
                .flatten()
        })
}

const TEXT_ENCODER_INPUTS: &[(&str, TensorElementType, usize)] = &[
    ("text_ids", TensorElementType::Int64, 2),
    ("txt_lengths", TensorElementType::Int64, 1),
];
const TEXT_ENCODER_OUTPUTS: &[(&str, TensorElementType, usize)] = &[
    ("text_states", TensorElementType::Float32, 3),
    ("text_valid", TensorElementType::Bool, 2),
    ("soa_embed", TensorElementType::Float32, 3),
    ("cross_kv", TensorElementType::Float32, 6),
];
const PREFIX_STEP_INPUTS: &[(&str, TensorElementType, usize)] = &[
    ("external_embed", TensorElementType::Float32, 3),
    ("use_external_embed", TensorElementType::Bool, 2),
    ("frame_codes", TensorElementType::Int64, 3),
    ("new_pos", TensorElementType::Int64, 2),
    ("new_valid", TensorElementType::Bool, 2),
    ("packed_kv", TensorElementType::Float32, 6),
    ("new_bidirectional", TensorElementType::Bool, 2),
    ("past_valid", TensorElementType::Bool, 2),
    ("cross_kv", TensorElementType::Float32, 6),
    ("text_valid", TensorElementType::Bool, 2),
];
const PREFIX_STEP_OUTPUTS: &[(&str, TensorElementType, usize)] = &[
    ("hidden", TensorElementType::Float32, 3),
    ("new_packed_kv", TensorElementType::Float32, 6),
    ("full_valid", TensorElementType::Bool, 2),
];
const LOCAL_FRAME_INPUTS: &[(&str, TensorElementType, usize)] = &[
    ("global_hidden", TensorElementType::Float32, 2),
    ("forbid_eoa", TensorElementType::Bool, 1),
    ("text_temperature", TensorElementType::Float32, 1),
    ("text_topk", TensorElementType::Int64, 1),
    ("audio_temperature", TensorElementType::Float32, 1),
    ("audio_topk", TensorElementType::Int64, 1),
    ("audio_topp", TensorElementType::Float32, 1),
    ("audio_repetition_penalty", TensorElementType::Float32, 1),
    ("seen_mask", TensorElementType::Bool, 3),
    ("ctrl_random_u", TensorElementType::Float32, 1),
    ("audio_random_u", TensorElementType::Float32, 2),
    ("cfg_scale", TensorElementType::Float32, 1),
];
const LOCAL_FRAME_OUTPUTS: &[(&str, TensorElementType, usize)] = &[
    ("is_eoa", TensorElementType::Int32, 1),
    ("codes", TensorElementType::Int32, 2),
];

fn load_tokenizer(path: &Path) -> Result<Tokenizer, TtsError> {
    let tokenizer_bytes = fs::read(path).map_err(contract_error)?;
    let tokenizer = Tokenizer::from_bytes(&tokenizer_bytes)
        .map_err(|error| TtsError::IncompatibleContract(error.to_string()))?;
    let document: serde_json::Value =
        serde_json::from_slice(&tokenizer_bytes).map_err(contract_error)?;
    for (token, expected) in SPECIAL_TOKENS {
        if tokenizer_special_id(&document, token) != Some(expected) {
            return Err(TtsError::IncompatibleContract(format!(
                "tokenizer special token {token} is not {expected}"
            )));
        }
    }
    Ok(tokenizer)
}

fn load_voice(path: &Path, config: &Config) -> Result<Array3<f32>, TtsError> {
    let file = fs::File::open(path).map_err(contract_error)?;
    let mut archive = NpzReader::new(file).map_err(contract_error)?;
    let declared: Array0<i64> = archive
        .by_name("n_voice_queries.npy")
        .map_err(contract_error)?;
    let voice: Array3<f32> = archive.by_name("voice_emb.npy").map_err(contract_error)?;
    if declared.into_scalar() != config.n_voice_queries as i64
        || voice.shape() != [1, config.n_voice_queries, config.d_model]
        || voice.iter().any(|sample| !sample.is_finite())
    {
        return Err(TtsError::IncompatibleContract(
            "maichi voice latent shape does not match the pinned model".into(),
        ));
    }
    Ok(voice)
}

fn load_graph(
    path: &Path,
    threads: usize,
    inputs: &[(&str, TensorElementType, usize)],
    outputs: &[(&str, TensorElementType, usize)],
) -> Result<Session, TtsError> {
    let session = Session::builder()
        .map_err(contract_error)?
        .with_intra_threads(threads)
        .map_err(contract_error)?
        .commit_from_file(path)
        .map_err(contract_error)?;
    validate_graph_io(session.inputs(), inputs)?;
    validate_graph_io(session.outputs(), outputs)?;
    Ok(session)
}

fn validate_graph_io(
    outlets: &[ort::value::Outlet],
    expected: &[(&str, TensorElementType, usize)],
) -> Result<(), TtsError> {
    if outlets.len() != expected.len() {
        let names = outlets
            .iter()
            .map(ort::value::Outlet::name)
            .collect::<Vec<_>>()
            .join(", ");
        return Err(TtsError::IncompatibleContract(format!(
            "graph declares {} tensors, but the pinned contract declares {}; graph tensors: {names}",
            outlets.len(),
            expected.len()
        )));
    }
    for (name, element_type, rank) in expected {
        let outlet = outlets
            .iter()
            .find(|outlet| outlet.name() == *name)
            .ok_or_else(|| {
                let names = outlets
                    .iter()
                    .map(ort::value::Outlet::name)
                    .collect::<Vec<_>>()
                    .join(", ");
                TtsError::IncompatibleContract(format!(
                    "graph is missing {name}; declared tensors: {names}"
                ))
            })?;
        match outlet.dtype() {
            ValueType::Tensor { ty, shape, .. } if ty == element_type && shape.len() == *rank => {}
            actual => {
                return Err(TtsError::IncompatibleContract(format!(
                    "graph tensor {name} has incompatible type or rank: {actual:?}"
                )));
            }
        }
    }
    Ok(())
}

/// Applies the exact pinned tokenizer normalization without changing case or punctuation.
pub fn normalize_text(text: &str) -> String {
    let mut normalized = String::new();
    let mut in_whitespace = false;
    for character in text.nfc() {
        if character.is_whitespace() {
            in_whitespace = true;
        } else {
            if in_whitespace && !normalized.is_empty() {
                normalized.push(' ');
            }
            normalized.push(character);
            in_whitespace = false;
        }
    }
    if in_whitespace && !normalized.is_empty() {
        normalized.push(' ');
    }
    normalized
}

fn validate_config(config: &Config) -> Result<(), TtsError> {
    if config.text_format != "bpe"
        || config.vocab_size == 0
        || config.num_codebooks == 0
        || config.codebook_size == 0
        || config.d_model == 0
        || config.n_heads == 0
        || !config.d_model.is_multiple_of(config.n_heads)
        || config.n_layers == 0
        || config.n_voice_queries == 0
        || config.sample_rate != 48_000
        || config.codec_frame_rate != 12.5
    {
        return Err(TtsError::IncompatibleContract(
            "config does not match the pinned ZeroTTS BPE/48 kHz graph contract".into(),
        ));
    }
    for (token, expected) in SPECIAL_TOKENS {
        if config.special_tokens.get(token) != Some(&expected) {
            return Err(TtsError::IncompatibleContract(format!(
                "config special token {token} is not {expected}"
            )));
        }
    }
    Ok(())
}

fn contract_error(error: impl std::fmt::Display) -> TtsError {
    TtsError::IncompatibleContract(error.to_string())
}
