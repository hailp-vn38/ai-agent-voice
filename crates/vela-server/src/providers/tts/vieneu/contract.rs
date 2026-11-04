//! Verified checkpoint metadata and preset data for the native VieNeu engine.
//!
//! This module intentionally does not instantiate ONNX sessions. It validates
//! immutable artifacts before a retained worker is allowed to use them.

use std::{
    collections::BTreeMap,
    fs::{self, File},
    path::Path,
};

use ndarray::ArrayD;
use ndarray_npy::NpzReader;
use serde::Deserialize;
use tokenizers::Tokenizer;

use super::assets::{NANO_VOICES, VOICES, VieneuNanoAssets, VieneuNativeAssets};
use crate::providers::tts::TtsError;

pub const NANO_SPEAKER_DIM: usize = 192;

fn invalid(reason: &str) -> TtsError {
    TtsError::IncompatibleContract(format!("VieNeu v3 Turbo: {reason}"))
}

fn invalid_nano(reason: &str) -> TtsError {
    TtsError::IncompatibleContract(format!("VieNeu v3 Nano: {reason}"))
}

#[derive(Clone, Debug, Deserialize)]
pub struct ModelConfig {
    pub n_vq: usize,
    pub audio_vocab_size: usize,
    pub text_vocab_size: usize,
    pub hidden_size: usize,
    pub num_hidden_layers: usize,
    pub local_num_hidden_layers: usize,
    pub local_num_attention_heads: usize,
    pub audio_sample_rate: u32,
    pub audio_pad_token_id: usize,
    pub text_prompt_start_token_id: usize,
    pub text_prompt_end_token_id: usize,
    pub speech_generation_start_token_id: usize,
    pub speech_generation_end_token_id: usize,
    pub audio_ref_slot_token_id: usize,
    pub default_style_token_id: usize,
    pub use_speaker_embedding: bool,
    pub speaker_embedding_dim: usize,
}

impl ModelConfig {
    pub fn parse(bytes: &[u8]) -> Result<Self, TtsError> {
        let config: Self =
            serde_json::from_slice(bytes).map_err(|_| invalid("invalid config.json"))?;
        if config.n_vq != 16
            || config.audio_vocab_size != 1024
            || config.text_vocab_size < 40
            || config.hidden_size != 768
            || config.num_hidden_layers != 12
            || config.local_num_hidden_layers != 1
            || config.local_num_attention_heads != 8
            || config.audio_sample_rate != 48_000
            || config.audio_pad_token_id != 1024
            || !config.use_speaker_embedding
            || config.speaker_embedding_dim != 192
        {
            return Err(invalid("unsupported ONNX FP32 model profile"));
        }
        let tokens = [
            config.text_prompt_start_token_id,
            config.text_prompt_end_token_id,
            config.speech_generation_start_token_id,
            config.speech_generation_end_token_id,
            config.audio_ref_slot_token_id,
            config.default_style_token_id,
        ];
        if tokens.iter().any(|id| *id >= config.text_vocab_size) {
            return Err(invalid("special token outside text vocabulary"));
        }
        Ok(config)
    }
}

#[derive(Clone, Debug)]
pub struct PresetVoice {
    pub speaker_embedding: Vec<f32>,
    pub reference_codes: Vec<Vec<i32>>,
}

#[derive(Clone, Debug)]
pub struct VoiceRegistry {
    pub default_voice: String,
    pub presets: BTreeMap<String, PresetVoice>,
}

#[derive(Deserialize)]
struct PresetFile {
    default_voice: String,
    presets: BTreeMap<String, PresetEntry>,
}

#[derive(Deserialize)]
struct PresetEntry {
    speaker_emb: Vec<f32>,
    codes: Vec<Vec<i32>>,
}

impl VoiceRegistry {
    pub fn parse(bytes: &[u8], model: &ModelConfig) -> Result<Self, TtsError> {
        let document: PresetFile =
            serde_json::from_slice(bytes).map_err(|_| invalid("invalid voices JSON"))?;
        let mut presets = BTreeMap::new();
        if document.presets.len() != VOICES.len() {
            return Err(invalid("preset voice count differs from pinned catalog"));
        }
        for named in VOICES {
            let entry = document
                .presets
                .get(named.source_name)
                .ok_or_else(|| invalid("missing pinned preset voice"))?;
            if entry.speaker_emb.len() != model.speaker_embedding_dim
                || entry.speaker_emb.iter().any(|value| !value.is_finite())
                || entry.speaker_emb.iter().all(|value| *value == 0.0)
                || entry.codes.is_empty()
                || entry.codes.len() > 512
                || entry.codes.iter().any(|frame| {
                    frame.len() != model.n_vq
                        || frame
                            .iter()
                            .any(|code| *code < 0 || *code >= model.audio_vocab_size as i32)
                })
            {
                return Err(invalid(
                    "preset embedding or acoustic code contract mismatch",
                ));
            }
            presets.insert(
                named.id.into(),
                PresetVoice {
                    speaker_embedding: entry.speaker_emb.clone(),
                    reference_codes: entry.codes.clone(),
                },
            );
        }
        let default_voice = VOICES
            .iter()
            .find(|preset| preset.source_name == document.default_voice)
            .ok_or_else(|| invalid("unknown upstream default voice"))?
            .id
            .to_owned();
        Ok(Self {
            default_voice,
            presets,
        })
    }

    pub fn get(&self, id: &str) -> Result<&PresetVoice, TtsError> {
        self.presets
            .get(id)
            .ok_or_else(|| TtsError::UnsupportedVoice(id.into()))
    }
}

#[derive(Deserialize)]
struct CodecMetadata {
    format_version: u32,
    codec_config: CodecConfig,
    files: CodecFiles,
    streaming_decode: StreamingMetadata,
}

#[derive(Deserialize)]
struct CodecConfig {
    sample_rate: u32,
    channels: usize,
    downsample_rate: usize,
    num_quantizers: usize,
}

#[derive(Deserialize)]
struct CodecFiles {
    decode_step: String,
}

#[derive(Deserialize)]
struct StreamingMetadata {
    transformer_offsets: Vec<serde_json::Value>,
    attention_caches: Vec<serde_json::Value>,
}

fn validate_codec_metadata(bytes: &[u8], config: &ModelConfig) -> Result<(), TtsError> {
    let metadata: CodecMetadata =
        serde_json::from_slice(bytes).map_err(|_| invalid("invalid MOSS codec metadata"))?;
    if metadata.format_version != 2
        || metadata.codec_config.sample_rate != 48_000
        || metadata.codec_config.channels != 2
        || metadata.codec_config.downsample_rate != 3840
        || metadata.codec_config.num_quantizers != config.n_vq
        || metadata.files.decode_step != "moss_audio_tokenizer_decode_step.onnx"
        || metadata.streaming_decode.transformer_offsets.len() != 4
        || metadata.streaming_decode.attention_caches.len() != 12
    {
        return Err(invalid("unsupported MOSS streaming codec profile"));
    }
    Ok(())
}

pub struct VieneuContract {
    pub config: ModelConfig,
    pub tokenizer: Tokenizer,
    pub voices: VoiceRegistry,
}

impl VieneuContract {
    pub fn load(assets: &VieneuNativeAssets) -> Result<Self, TtsError> {
        let load =
            |path: &Path| fs::read(path).map_err(|_| invalid("model file could not be read"));
        let config = ModelConfig::parse(&load(&assets.config)?)?;
        let tokenizer = Tokenizer::from_file(&assets.tokenizer)
            .map_err(|_| invalid("invalid tokenizer.json"))?;
        for (token, id) in [
            ("<|TEXT_PROMPT_START|>", config.text_prompt_start_token_id),
            ("<|TEXT_PROMPT_END|>", config.text_prompt_end_token_id),
            (
                "<|SPEECH_GENERATION_START|>",
                config.speech_generation_start_token_id,
            ),
            (
                "<|SPEECH_GENERATION_END|>",
                config.speech_generation_end_token_id,
            ),
            ("<|audio_ref_slot|>", config.audio_ref_slot_token_id),
            ("<|style_0|>", config.default_style_token_id),
        ] {
            if tokenizer.token_to_id(token) != Some(id as u32) {
                return Err(invalid("tokenizer special IDs differ from model config"));
            }
        }
        let voices = VoiceRegistry::parse(&load(&assets.presets)?, &config)?;
        validate_codec_metadata(&load(&assets.codec_metadata)?, &config)?;
        // Ensure binary artifacts were resolved without accidentally treating empty stubs
        // as ready. Graph-specific dimensions are verified when ORT sessions are built.
        for path in [
            &assets.prefill,
            &assets.decode_step,
            &assets.acoustic,
            &assets.backbone_data,
            &assets.heads,
            &assets.codec_step,
            &assets.codec_data,
            &assets.g2p_dictionary,
        ] {
            if !path.is_file() || path.metadata().map_or(true, |m| m.len() == 0) {
                return Err(invalid("missing or empty required binary asset"));
            }
        }
        Ok(Self {
            config,
            tokenizer,
            voices,
        })
    }
}

fn default_nano_model_type() -> String {
    "vieneu_v3_nano".into()
}

#[derive(Clone, Debug, Deserialize)]
pub struct NanoModelConfig {
    #[serde(default = "default_nano_model_type")]
    pub model_type: String,
    pub sample_rate: u32,
    pub latent_dim: usize,
    pub group: usize,
    pub flow_fps: f32,
    pub dim: usize,
    pub style_dim: usize,
    pub n_style: usize,
    pub steps_default: usize,
    pub cfg_default: f32,
    pub pad_id: i64,
    pub bos_id: i64,
    pub eos_id: i64,
    pub vocab: BTreeMap<String, i64>,
}

impl NanoModelConfig {
    pub const fn latent_channels(&self) -> usize {
        self.latent_dim * self.group
    }

    pub fn parse(bytes: &[u8]) -> Result<Self, TtsError> {
        let config: Self =
            serde_json::from_slice(bytes).map_err(|_| invalid_nano("invalid config.json"))?;
        if config.model_type != "vieneu_v3_nano"
            || config.sample_rate != 24_000
            || config.latent_dim != 24
            || config.group != 6
            || !config.flow_fps.is_finite()
            || (config.flow_fps - 15.625).abs() >= 1e-4
            || config.dim != 512
            || config.style_dim != 256
            || config.n_style != 50
            || !(1..=64).contains(&config.steps_default)
            || !config.cfg_default.is_finite()
            || !(0.0..=16.0).contains(&config.cfg_default)
            || config.pad_id != 0
            || config.bos_id != 1
            || config.eos_id != 2
            || config.vocab.len() < 80
            || config.vocab.get("<pad>") != Some(&0)
            || config.vocab.get("<bos>") != Some(&1)
            || config.vocab.get("<eos>") != Some(&2)
            || config.vocab.values().any(|&id| id < 0)
        {
            return Err(invalid_nano("unsupported v3 Nano ONNX FP32 model profile"));
        }
        Ok(config)
    }
}

#[derive(Clone, Debug)]
pub struct NanoPresetVoice {
    pub speaker_embedding: Vec<f32>,
    pub style: Vec<f32>,
}

#[derive(Clone, Debug)]
pub struct NanoVoiceRegistry {
    pub default_voice: String,
    pub presets: BTreeMap<String, NanoPresetVoice>,
}

#[derive(Deserialize)]
struct NanoPresetFile {
    default_voice: String,
    presets: BTreeMap<String, NanoPresetEntry>,
}

#[derive(Deserialize)]
struct NanoPresetEntry {
    speaker_emb: Vec<f32>,
    style: Vec<Vec<f32>>,
}

impl NanoVoiceRegistry {
    pub fn parse(bytes: &[u8], model: &NanoModelConfig) -> Result<Self, TtsError> {
        let document: NanoPresetFile =
            serde_json::from_slice(bytes).map_err(|_| invalid_nano("invalid Nano voices JSON"))?;
        let mut presets = BTreeMap::new();
        if document.presets.len() != NANO_VOICES.len() {
            return Err(invalid_nano(
                "preset voice count differs from pinned Nano catalog",
            ));
        }
        let expected_style_len = model.n_style * model.style_dim;
        for named in NANO_VOICES {
            let entry = document
                .presets
                .get(named.source_name)
                .ok_or_else(|| invalid_nano("missing pinned Nano preset voice"))?;
            if entry.speaker_emb.len() != NANO_SPEAKER_DIM
                || entry.speaker_emb.iter().any(|value| !value.is_finite())
                || entry.speaker_emb.iter().all(|value| *value == 0.0)
                || entry.style.len() != model.n_style
                || entry.style.iter().any(|row| {
                    row.len() != model.style_dim || row.iter().any(|value| !value.is_finite())
                })
            {
                return Err(invalid_nano(
                    "preset speaker embedding or style token contract mismatch",
                ));
            }
            let flat_style: Vec<f32> = entry
                .style
                .iter()
                .flat_map(|row| row.iter().copied())
                .collect();
            if flat_style.len() != expected_style_len
                || flat_style.iter().all(|value| *value == 0.0)
            {
                return Err(invalid_nano("preset style matrix is empty or all zeros"));
            }
            presets.insert(
                named.id.into(),
                NanoPresetVoice {
                    speaker_embedding: entry.speaker_emb.clone(),
                    style: flat_style,
                },
            );
        }
        let default_voice = NANO_VOICES
            .iter()
            .find(|preset| preset.source_name == document.default_voice)
            .ok_or_else(|| invalid_nano("unknown upstream Nano default voice"))?
            .id
            .to_owned();
        Ok(Self {
            default_voice,
            presets,
        })
    }

    pub fn get(&self, id: &str) -> Result<&NanoPresetVoice, TtsError> {
        self.presets
            .get(id)
            .ok_or_else(|| TtsError::UnsupportedVoice(id.into()))
    }
}

fn load_nano_constants(
    path: &Path,
    config: &NanoModelConfig,
) -> Result<(Vec<f32>, Vec<f32>), TtsError> {
    let file = File::open(path).map_err(|_| invalid_nano("constants.npz could not be opened"))?;
    let mut archive =
        NpzReader::new(file).map_err(|_| invalid_nano("invalid constants.npz archive"))?;
    let null_spk: ArrayD<f32> = archive
        .by_name("null_spk")
        .map_err(|_| invalid_nano("missing null_spk in constants.npz"))?;
    let null_style: ArrayD<f32> = archive
        .by_name("null_style")
        .map_err(|_| invalid_nano("missing null_style in constants.npz"))?;

    let spk_shape = null_spk.shape();
    let valid_spk_shape = spk_shape == [NANO_SPEAKER_DIM] || spk_shape == [1, NANO_SPEAKER_DIM];
    let spk_vec: Vec<f32> = null_spk.iter().copied().collect();
    if !valid_spk_shape || spk_vec.iter().any(|value| !value.is_finite()) {
        return Err(invalid_nano(
            "null_spk shape or values differ from contract",
        ));
    }

    let style_shape = null_style.shape();
    let valid_style_shape = style_shape == [config.n_style, config.style_dim]
        || style_shape == [1, config.n_style, config.style_dim];
    let style_vec: Vec<f32> = null_style.iter().copied().collect();
    if !valid_style_shape || style_vec.iter().any(|value| !value.is_finite()) {
        return Err(invalid_nano(
            "null_style shape or values differ from contract",
        ));
    }

    Ok((spk_vec, style_vec))
}

pub struct NanoContract {
    pub config: NanoModelConfig,
    pub null_spk: Vec<f32>,
    pub null_style: Vec<f32>,
    pub voices: NanoVoiceRegistry,
}

impl NanoContract {
    pub fn load(assets: &VieneuNanoAssets) -> Result<Self, TtsError> {
        let load = |path: &Path| {
            fs::read(path).map_err(|_| invalid_nano("Nano model file could not be read"))
        };
        let config = NanoModelConfig::parse(&load(&assets.config)?)?;
        let (null_spk, null_style) = load_nano_constants(&assets.constants, &config)?;
        let voices = NanoVoiceRegistry::parse(&load(&assets.presets)?, &config)?;
        for path in [
            &assets.text_encoder,
            &assets.duration_predictor,
            &assets.vector_estimator,
            &assets.codec_decoder,
            &assets.g2p_dictionary,
        ] {
            if !path.is_file() || path.metadata().map_or(true, |m| m.len() == 0) {
                return Err(invalid_nano("missing or empty required Nano binary asset"));
            }
        }
        Ok(Self {
            config,
            null_spk,
            null_style,
            voices,
        })
    }
}

#[cfg(test)]
#[path = "contract_tests.rs"]
mod tests;
