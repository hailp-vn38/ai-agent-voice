//! Verified checkpoint metadata and preset data for the native VieNeu engine.
//!
//! This module intentionally does not instantiate ONNX sessions. It validates
//! immutable artifacts before a retained worker is allowed to use them.

use std::{collections::BTreeMap, fs, path::Path};

use serde::Deserialize;
use tokenizers::Tokenizer;

use super::assets::{VieneuNativeAssets, VOICES};
use crate::providers::tts::TtsError;

fn invalid(reason: &str) -> TtsError {
    TtsError::IncompatibleContract(format!("VieNeu v3 Turbo: {reason}"))
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
        let config: Self = serde_json::from_slice(bytes)
            .map_err(|_| invalid("invalid config.json"))?;
        if config.n_vq != 16
            || config.audio_vocab_size != 1024
            || config.text_vocab_size < 40
            || config.hidden_size != 768
            || config.num_hidden_layers != 12
            || config.local_num_hidden_layers == 0
            || config.local_num_attention_heads == 0
            || config.hidden_size % config.local_num_attention_heads != 0
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
                        || frame.iter().any(|code| *code < 0 || *code >= model.audio_vocab_size as i32)
                })
            {
                return Err(invalid("preset embedding or acoustic code contract mismatch"));
            }
            presets.insert(named.id.into(), PresetVoice {
                speaker_embedding: entry.speaker_emb.clone(),
                reference_codes: entry.codes.clone(),
            });
        }
        let default_voice = VOICES.iter()
            .find(|preset| preset.source_name == document.default_voice)
            .ok_or_else(|| invalid("unknown upstream default voice"))?
            .id.to_owned();
        Ok(Self { default_voice, presets })
    }

    pub fn get(&self, id: &str) -> Result<&PresetVoice, TtsError> {
        self.presets.get(id).ok_or_else(|| TtsError::UnsupportedVoice(id.into()))
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
    let metadata: CodecMetadata = serde_json::from_slice(bytes)
        .map_err(|_| invalid("invalid MOSS codec metadata"))?;
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
        let load = |path: &Path| fs::read(path)
            .map_err(|_| invalid("model file could not be read"));
        let config = ModelConfig::parse(&load(&assets.config)?)?;
        let tokenizer = Tokenizer::from_file(&assets.tokenizer)
            .map_err(|_| invalid("invalid tokenizer.json"))?;
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
        Ok(Self { config, tokenizer, voices })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn model() -> ModelConfig {
        ModelConfig::parse(br#"{
            "n_vq":16,"audio_vocab_size":1024,"text_vocab_size":419,
            "hidden_size":768,"num_hidden_layers":12,
            "local_num_hidden_layers":1,"local_num_attention_heads":8,
            "audio_sample_rate":48000,"audio_pad_token_id":1024,
            "text_prompt_start_token_id":3,"text_prompt_end_token_id":4,
            "speech_generation_start_token_id":5,"speech_generation_end_token_id":6,
            "audio_ref_slot_token_id":7,"default_style_token_id":16,
            "use_speaker_embedding":true,"speaker_embedding_dim":192
        }"#).unwrap()
    }

    #[test]
    fn accepts_pinned_model_profile() {
        assert_eq!(model().n_vq, 16);
    }

    #[test]
    fn rejects_legacy_model_without_speaker_anchor() {
        let bytes = br#"{
            "n_vq":16,"audio_vocab_size":1024,"text_vocab_size":419,
            "hidden_size":768,"num_hidden_layers":12,
            "local_num_hidden_layers":1,"local_num_attention_heads":8,
            "audio_sample_rate":48000,"audio_pad_token_id":1024,
            "text_prompt_start_token_id":3,"text_prompt_end_token_id":4,
            "speech_generation_start_token_id":5,"speech_generation_end_token_id":6,
            "audio_ref_slot_token_id":7,"default_style_token_id":16,
            "use_speaker_embedding":false,"speaker_embedding_dim":192
        }"#;
        assert!(ModelConfig::parse(bytes).is_err());
    }

    #[test]
    fn rejects_empty_or_unrecognized_preset_registry() {
        assert!(VoiceRegistry::parse(
            br#"{"default_voice":"Hải Đăng","presets":{}}"#,
            &model(),
        ).is_err());
    }

    #[test]
    fn rejects_non_streaming_codec_metadata() {
        let json = br#"{"format_version":2,
            "codec_config":{"sample_rate":48000,"channels":2,
                "downsample_rate":3840,"num_quantizers":16},
            "files":{"decode_step":"moss_audio_tokenizer_decode_step.onnx"},
            "streaming_decode":{"transformer_offsets":[],"attention_caches":[]}
        }"#;
        assert!(validate_codec_metadata(json, &model()).is_err());
    }
}
