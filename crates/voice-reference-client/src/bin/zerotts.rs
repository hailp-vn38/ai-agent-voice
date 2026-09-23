//! Local ZeroTTS reference runner. It writes provider PCM before server resampling or Opus.
//! The inference loop is kept here so experiments do not change the server's delivery path.

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

use clap::Parser;
use std::sync::OnceLock;
use std::time::Instant;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum TtsError {
    #[error("ZeroTTS contract is incompatible: {0}")]
    IncompatibleContract(String),
}

pub struct PcmF32Mono {
    samples: Vec<f32>,
    sample_rate_hz: u32,
}

impl PcmF32Mono {
    fn new(samples: Vec<f32>, sample_rate_hz: u32) -> Self {
        Self {
            samples,
            sample_rate_hz,
        }
    }

    fn samples(&self) -> &[f32] {
        &self.samples
    }

    fn sample_rate_hz(&self) -> u32 {
        self.sample_rate_hz
    }
}

fn initialize_ort(path: &Path) -> Result<(), TtsError> {
    static INIT: OnceLock<Result<(), String>> = OnceLock::new();
    INIT.get_or_init(|| {
        if !path.is_file() {
            return Err(format!(
                "ONNX Runtime library is missing: {}",
                path.display()
            ));
        }
        ort::init_from(path)
            .map_err(|error| error.to_string())?
            .commit()
            .then_some(())
            .ok_or_else(|| "ONNX Runtime was initialized before this runner".to_owned())
    })
    .as_ref()
    .map_err(|error| TtsError::IncompatibleContract(error.clone()))
    .copied()
}

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

#[derive(Clone, Deserialize)]
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
#[derive(Clone)]
pub struct ZeroTtsContract {
    tokenizer: Tokenizer,
    config: Config,
    voice: Option<Array3<f32>>,
    graphs: Option<GraphPaths>,
}

/// Immutable graph sources. Runtime workers load their own sessions from these paths once.
#[derive(Clone)]
struct GraphPaths {
    text_encoder: PathBuf,
    prefix_step: PathBuf,
    local_frame_decode: PathBuf,
    threads: usize,
    codec: CodecPaths,
}

#[derive(Clone)]
struct CodecPaths {
    decode_full: PathBuf,
    decode_step: PathBuf,
    metadata: CodecMetadata,
}

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct CodecMetadata {
    format_version: u32,
    checkpoint_path: String,
    files: CodecFiles,
    external_data_files: BTreeMap<String, Vec<String>>,
    codec_config: CodecConfig,
    onnx: CodecOnnx,
    streaming_decode: StreamingDecode,
}

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct CodecFiles {
    decode_full: String,
    decode_step: String,
}

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct CodecConfig {
    sample_rate: u32,
    channels: usize,
    downsample_rate: usize,
    num_quantizers: usize,
}

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct CodecOnnx {
    opset: u32,
    decode_input_names: Vec<String>,
    decode_output_names: Vec<String>,
    decode_step_input_names: Vec<String>,
    decode_step_output_names: Vec<String>,
}

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct StreamingDecode {
    batch_size: usize,
    transformer_offsets: Vec<TransformerOffset>,
    attention_caches: Vec<AttentionCache>,
}
#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct TransformerOffset {
    index: usize,
    decoder_index: usize,
    input_name: String,
    output_name: String,
    shape: Vec<usize>,
    dtype: String,
}
#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct AttentionCache {
    index: usize,
    decoder_index: usize,
    layer_index: usize,
    context: usize,
    num_heads: usize,
    head_dim: usize,
    offset_input_name: String,
    offset_output_name: String,
    cached_keys_input_name: String,
    cached_keys_output_name: String,
    cached_values_input_name: String,
    cached_values_output_name: String,
    cached_positions_input_name: String,
    cached_positions_output_name: String,
    offset_shape: Vec<usize>,
    cache_shape: Vec<usize>,
    positions_shape: Vec<usize>,
    cache_dtype: String,
    positions_dtype: String,
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
        codec_decode_full_path: &Path,
        codec_decode_step_path: &Path,
        codec_shared_data_path: &Path,
        codec_metadata_path: &Path,
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
        let metadata = load_codec_metadata(
            codec_metadata_path,
            codec_decode_full_path,
            codec_decode_step_path,
            codec_shared_data_path,
            config.num_codebooks,
        )?;
        load_codec_graph(
            codec_decode_full_path,
            threads,
            &metadata.onnx.decode_input_names,
            &metadata.onnx.decode_output_names,
        )?;
        load_codec_graph(
            codec_decode_step_path,
            threads,
            &metadata.onnx.decode_step_input_names,
            &metadata.onnx.decode_step_output_names,
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
                codec: CodecPaths {
                    decode_full: codec_decode_full_path.into(),
                    decode_step: codec_decode_step_path.into(),
                    metadata,
                },
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

    /// Produces the provider boundary PCM. Codec layout and profile are validated at startup.
    pub fn synthesize_pcm(&self, text: &str, max_frames: usize) -> Result<PcmF32Mono, TtsError> {
        let codes = self.synthesize_codes(text, max_frames)?;
        if codes.eoa.is_none() {
            return Err(TtsError::IncompatibleContract(
                "ZeroTTS synthesis reached its frame bound".into(),
            ));
        }
        let mut codec = CodecOperation::new(self)?;
        // The startup/user delivery contract pins both decode paths. The full graph validates
        // the complete code sequence before the streaming graph produces the PCM boundary.
        let _ = codec.decode_full(&codes.frames)?;
        codec.decode_step(&codes.frames)
    }

    /// Mirrors ZeroTTS `synthesize_stream`: generate AR frames and pass ramped groups through
    /// the KV-cached codec decoder immediately.  No complete utterance PCM is retained here.
    pub fn synthesize_pcm_stream(
        &self,
        text: &str,
        max_frames: usize,
        on_pcm: &mut dyn FnMut(PcmF32Mono) -> Result<(), TtsError>,
    ) -> Result<(), TtsError> {
        let mut stream = ZeroTtsPcmStream::new(self)?;
        stream.synthesize(text, max_frames, on_pcm)
    }
}

/// Codec state is scoped to one SpeechOutput delivery, while every text segment still creates a
/// fresh ZeroTtsOperation for its AR/KV state.
pub struct ZeroTtsPcmStream {
    operation: ZeroTtsOperation,
    codec: CodecOperation,
    first_segment: bool,
}

impl ZeroTtsPcmStream {
    pub fn new(contract: &ZeroTtsContract) -> Result<Self, TtsError> {
        Ok(Self {
            operation: ZeroTtsOperation::new(contract)?,
            codec: CodecOperation::new(contract)?,
            first_segment: true,
        })
    }

    pub fn synthesize(
        &mut self,
        text: &str,
        max_frames: usize,
        on_pcm: &mut dyn FnMut(PcmF32Mono) -> Result<(), TtsError>,
    ) -> Result<(), TtsError> {
        let mut frames = Vec::new();
        let mut target = if self.first_segment { 1 } else { 16 };
        let ramp = self.first_segment;
        let result = self.operation.synthesize_with_frame_sink(
            text,
            max_frames,
            |frame, _index, _is_eoa| {
                frames.push(frame.to_vec());
                if frames.len() >= target {
                    on_pcm(self.codec.decode_step(&frames)?)?;
                    frames.clear();
                    if ramp {
                        target = (target * 2).min(16);
                    }
                }
                Ok(())
            },
        )?;
        if result.eoa.is_none() {
            return Err(TtsError::IncompatibleContract(
                "ZeroTTS synthesis reached its frame bound".into(),
            ));
        }
        if !frames.is_empty() {
            on_pcm(self.codec.decode_step(&frames)?)?;
        }
        self.first_segment = false;
        Ok(())
    }

    /// Retains native ONNX sessions but discards per-response codec history before this worker
    /// can be assigned to another Voice Session.
    pub fn reset(&mut self) -> Result<(), TtsError> {
        self.codec.reset()?;
        self.first_segment = true;
        Ok(())
    }
}

struct CodecOperation {
    contract: ZeroTtsContract,
    decode_full: Session,
    // The step graph is owned by one native worker; only its cache is reset between leases.
    decode_step: Session,
    streaming_state: BTreeMap<String, CodecStreamState>,
}

enum CodecStreamState {
    I32 { shape: Vec<usize>, data: Vec<i32> },
    F32 { shape: Vec<usize>, data: Vec<f32> },
}

impl CodecOperation {
    fn new(contract: &ZeroTtsContract) -> Result<Self, TtsError> {
        let graphs = contract
            .graphs
            .as_ref()
            .ok_or_else(|| TtsError::IncompatibleContract("ZeroTTS engine is not loaded".into()))?;
        let streaming_state = Self::initial_streaming_state(contract)?;
        Ok(Self {
            contract: contract.clone(),
            decode_full: load_codec_graph(
                &graphs.codec.decode_full,
                graphs.threads,
                &graphs.codec.metadata.onnx.decode_input_names,
                &graphs.codec.metadata.onnx.decode_output_names,
            )?,
            decode_step: load_codec_graph(
                &graphs.codec.decode_step,
                graphs.threads,
                &graphs.codec.metadata.onnx.decode_step_input_names,
                &graphs.codec.metadata.onnx.decode_step_output_names,
            )?,
            streaming_state,
        })
    }

    fn initial_streaming_state(
        contract: &ZeroTtsContract,
    ) -> Result<BTreeMap<String, CodecStreamState>, TtsError> {
        let graphs = contract
            .graphs
            .as_ref()
            .ok_or_else(|| TtsError::IncompatibleContract("ZeroTTS engine is not loaded".into()))?;
        let mut streaming_state = BTreeMap::new();
        for offset in &graphs.codec.metadata.streaming_decode.transformer_offsets {
            streaming_state.insert(
                offset.input_name.clone(),
                CodecStreamState::I32 {
                    shape: offset.shape.clone(),
                    data: vec![0; offset.shape.iter().product()],
                },
            );
        }
        for cache in &graphs.codec.metadata.streaming_decode.attention_caches {
            streaming_state.insert(
                cache.offset_input_name.clone(),
                CodecStreamState::I32 {
                    shape: cache.offset_shape.clone(),
                    data: vec![0; cache.offset_shape.iter().product()],
                },
            );
            for name in [
                &cache.cached_keys_input_name,
                &cache.cached_values_input_name,
            ] {
                streaming_state.insert(
                    name.clone(),
                    CodecStreamState::F32 {
                        shape: cache.cache_shape.clone(),
                        data: vec![0.0; cache.cache_shape.iter().product()],
                    },
                );
            }
            streaming_state.insert(
                cache.cached_positions_input_name.clone(),
                CodecStreamState::I32 {
                    shape: cache.positions_shape.clone(),
                    data: vec![-1; cache.positions_shape.iter().product()],
                },
            );
        }
        Ok(streaming_state)
    }

    fn reset(&mut self) -> Result<(), TtsError> {
        self.streaming_state = Self::initial_streaming_state(&self.contract)?;
        Ok(())
    }

    fn decode_step(&mut self, frames: &[Vec<i32>]) -> Result<PcmF32Mono, TtsError> {
        let metadata = &self
            .contract
            .graphs
            .as_ref()
            .expect("engine invariant")
            .codec
            .metadata;
        let codes = frames.iter().flatten().copied().collect::<Vec<_>>();
        let mut inputs = vec![
            (
                "audio_codes".to_owned(),
                ort::session::SessionInputValue::from(tensor(
                    vec![1, frames.len(), metadata.codec_config.num_quantizers],
                    codes,
                )?),
            ),
            (
                "audio_code_lengths".to_owned(),
                ort::session::SessionInputValue::from(tensor(vec![1], vec![frames.len() as i32])?),
            ),
        ];
        for (name, state) in &self.streaming_state {
            let value = match state {
                CodecStreamState::I32 { shape, data } => {
                    tensor(shape.clone(), data.clone())?.into()
                }
                CodecStreamState::F32 { shape, data } => {
                    tensor(shape.clone(), data.clone())?.into()
                }
            };
            inputs.push((name.clone(), value));
        }
        if metadata.streaming_decode.batch_size != 1 {
            return Err(TtsError::IncompatibleContract(
                "codec streaming batch must be one".into(),
            ));
        }
        let output = self.decode_step.run(inputs).map_err(contract_error)?;
        let next_state = Self::next_streaming_state(metadata, &output)?;
        let pcm = Self::mono_pcm(metadata, &output)?;
        drop(output);
        self.streaming_state = next_state;
        Ok(pcm)
    }

    fn next_streaming_state(
        metadata: &CodecMetadata,
        output: &ort::session::SessionOutputs<'_>,
    ) -> Result<BTreeMap<String, CodecStreamState>, TtsError> {
        let mut state = BTreeMap::new();
        for offset in &metadata.streaming_decode.transformer_offsets {
            let value = i32_tensor(&output[offset.output_name.as_str()])?;
            state.insert(
                offset.input_name.clone(),
                CodecStreamState::I32 {
                    shape: value.shape,
                    data: value.data,
                },
            );
        }
        for cache in &metadata.streaming_decode.attention_caches {
            let offset = i32_tensor(&output[cache.offset_output_name.as_str()])?;
            state.insert(
                cache.offset_input_name.clone(),
                CodecStreamState::I32 {
                    shape: offset.shape,
                    data: offset.data,
                },
            );
            for (input, output_name) in [
                (
                    &cache.cached_keys_input_name,
                    &cache.cached_keys_output_name,
                ),
                (
                    &cache.cached_values_input_name,
                    &cache.cached_values_output_name,
                ),
            ] {
                let value = f32_tensor(&output[output_name.as_str()])?;
                state.insert(
                    input.clone(),
                    CodecStreamState::F32 {
                        shape: value.shape,
                        data: value.data,
                    },
                );
            }
            let positions = i32_tensor(&output[cache.cached_positions_output_name.as_str()])?;
            state.insert(
                cache.cached_positions_input_name.clone(),
                CodecStreamState::I32 {
                    shape: positions.shape,
                    data: positions.data,
                },
            );
        }
        Ok(state)
    }

    fn decode_full(&mut self, frames: &[Vec<i32>]) -> Result<PcmF32Mono, TtsError> {
        let metadata = &self
            .contract
            .graphs
            .as_ref()
            .expect("engine invariant")
            .codec
            .metadata;
        if frames.is_empty()
            || frames
                .iter()
                .any(|frame| frame.len() != metadata.codec_config.num_quantizers)
        {
            return Err(TtsError::IncompatibleContract(
                "codec frames do not match pinned quantizer count".into(),
            ));
        }
        // ZeroTTS emits K values per time frame. The codec accepts int32 (B, T, K).
        let codes = frames.iter().flatten().copied().collect::<Vec<_>>();
        let output = self.decode_full.run(ort::inputs! {
            "audio_codes" => tensor(vec![1, frames.len(), metadata.codec_config.num_quantizers], codes)?,
            "audio_code_lengths" => tensor(vec![1], vec![frames.len() as i32])?,
        }).map_err(contract_error)?;
        Self::mono_pcm(metadata, &output)
    }

    fn mono_pcm(
        metadata: &CodecMetadata,
        output: &ort::session::SessionOutputs<'_>,
    ) -> Result<PcmF32Mono, TtsError> {
        let audio = f32_tensor(&output["audio"])?;
        let lengths = i32_tensor(&output["audio_lengths"])?;
        let length = usize::try_from(*lengths.data.first().ok_or_else(|| {
            TtsError::IncompatibleContract("codec returned no audio length".into())
        })?)
        .map_err(contract_error)?;
        if audio.shape.len() != 3
            || audio.shape[0] != 1
            || audio.shape[1] != metadata.codec_config.channels
            || length == 0
            || length > audio.shape[2]
        {
            return Err(TtsError::IncompatibleContract(
                "codec output shape does not match pinned stereo profile".into(),
            ));
        }
        let mut mono = Vec::with_capacity(length);
        for index in 0..length {
            let sum = (0..metadata.codec_config.channels)
                .map(|channel| audio.data[channel * audio.shape[2] + index])
                .sum::<f32>();
            let sample = sum / metadata.codec_config.channels as f32;
            if !sample.is_finite() {
                return Err(TtsError::IncompatibleContract(
                    "codec produced non-finite PCM".into(),
                ));
            }
            mono.push(sample.clamp(-1.0, 1.0));
        }
        Ok(PcmF32Mono::new(mono, metadata.codec_config.sample_rate))
    }
}

/// Mutable ONNX sessions and turn-local decode state for exactly one synthesis operation.
struct ZeroTtsOperation {
    contract: ZeroTtsContract,
    text_encoder: Session,
    prefix_step: Session,
    local_frame_decode: Session,
}

impl ZeroTtsOperation {
    fn new(contract: &ZeroTtsContract) -> Result<Self, TtsError> {
        let graphs = contract
            .graphs
            .as_ref()
            .ok_or_else(|| TtsError::IncompatibleContract("ZeroTTS engine is not loaded".into()))?;
        Ok(Self {
            contract: contract.clone(),
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
        self.synthesize_with_frame_sink(text, max_frames, |_, _, _| Ok(()))
    }

    fn synthesize_with_frame_sink<F>(
        &mut self,
        text: &str,
        max_frames: usize,
        mut on_frame: F,
    ) -> Result<CodeFrames, TtsError>
    where
        F: FnMut(&[i32], usize, bool) -> Result<(), TtsError>,
    {
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
                "forbid_eoa" => tensor(vec![1], vec![frame_index < 4])?, "text_temperature" => tensor(vec![1], vec![1.0_f32])?, "text_topk" => tensor(vec![1], vec![50_i64])?,
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
            on_frame(&codes, frame_index, is_eoa)?;
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

fn load_codec_metadata(
    path: &Path,
    full: &Path,
    step: &Path,
    shared_data: &Path,
    codebooks: usize,
) -> Result<CodecMetadata, TtsError> {
    let metadata: CodecMetadata =
        serde_json::from_slice(&fs::read(path).map_err(contract_error)?).map_err(contract_error)?;
    if metadata.format_version != 2
        || metadata.checkpoint_path != "MOSS-Audio-Tokenizer-Nano"
        || metadata.codec_config.sample_rate != 48_000
        || metadata.codec_config.channels != 2
        || metadata.codec_config.downsample_rate != 3_840
        || metadata.codec_config.num_quantizers != codebooks
        || metadata.onnx.opset != 17
        || metadata.streaming_decode.transformer_offsets.len() != 4
        || metadata.streaming_decode.attention_caches.len() != 12
        || metadata.files.decode_full
            != full
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or_default()
        || metadata.files.decode_step
            != step
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or_default()
        || metadata.onnx.decode_input_names != ["audio_codes", "audio_code_lengths"]
        || metadata.onnx.decode_output_names != ["audio", "audio_lengths"]
        || !metadata
            .onnx
            .decode_step_input_names
            .starts_with(&metadata.onnx.decode_input_names)
        || !metadata
            .onnx
            .decode_step_output_names
            .starts_with(&metadata.onnx.decode_output_names)
    {
        return Err(TtsError::IncompatibleContract(
            "codec metadata does not match the pinned 48 kHz stereo contract".into(),
        ));
    }
    for graph in [full, step] {
        let name = graph
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_default();
        let declared = metadata.external_data_files.get(name).ok_or_else(|| {
            TtsError::IncompatibleContract("codec metadata omits graph external data".into())
        })?;
        if declared.len() != 1
            || declared[0]
                != shared_data
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or_default()
            || !shared_data.is_file()
        {
            return Err(TtsError::IncompatibleContract(
                "codec shared external data does not match metadata".into(),
            ));
        }
    }
    for (index, offset) in metadata
        .streaming_decode
        .transformer_offsets
        .iter()
        .enumerate()
    {
        if offset.index != index
            || offset.decoder_index != 1 + index * 2
            || offset.input_name != format!("transformer_offset_{index}")
            || offset.output_name != format!("transformer_offset_out_{index}")
            || offset.shape != [1]
            || offset.dtype != "int32"
        {
            return Err(TtsError::IncompatibleContract(
                "codec transformer-offset metadata does not match the pinned graph".into(),
            ));
        }
    }
    for (index, cache) in metadata
        .streaming_decode
        .attention_caches
        .iter()
        .enumerate()
    {
        let (decoder_index, context) = match index {
            0..=3 => (1, 500),
            4..=5 => (3, 800),
            6..=7 => (5, 1_200),
            8..=11 => (7, 1_600),
            _ => unreachable!("checked cache count"),
        };
        if cache.index != index
            || cache.decoder_index != decoder_index
            || cache.layer_index >= 4
            || cache.context != context
            || cache.num_heads != 4
            || cache.head_dim != 64
            || cache.offset_input_name != format!("attn_offset_{index}")
            || cache.offset_output_name != format!("attn_offset_out_{index}")
            || cache.cached_keys_input_name != format!("attn_cached_keys_{index}")
            || cache.cached_keys_output_name != format!("attn_cached_keys_out_{index}")
            || cache.cached_values_input_name != format!("attn_cached_values_{index}")
            || cache.cached_values_output_name != format!("attn_cached_values_out_{index}")
            || cache.cached_positions_input_name != format!("attn_cached_positions_{index}")
            || cache.cached_positions_output_name != format!("attn_cached_positions_out_{index}")
            || cache.offset_shape != [1]
            || cache.cache_shape != [1, 4, context, 64]
            || cache.positions_shape != [1, context]
            || cache.cache_dtype != "float32"
            || cache.positions_dtype != "int32"
        {
            return Err(TtsError::IncompatibleContract(
                "codec attention-cache metadata does not match the pinned graph".into(),
            ));
        }
    }
    Ok(metadata)
}

fn load_codec_graph(
    path: &Path,
    threads: usize,
    expected_inputs: &[String],
    expected_outputs: &[String],
) -> Result<Session, TtsError> {
    let session = Session::builder()
        .map_err(contract_error)?
        .with_intra_threads(threads)
        .map_err(contract_error)?
        .commit_from_file(path)
        .map_err(contract_error)?;
    let actual_inputs = session
        .inputs()
        .iter()
        .map(ort::value::Outlet::name)
        .collect::<Vec<_>>();
    let actual_outputs = session
        .outputs()
        .iter()
        .map(ort::value::Outlet::name)
        .collect::<Vec<_>>();
    if actual_inputs
        != expected_inputs
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>()
        || actual_outputs
            != expected_outputs
                .iter()
                .map(String::as_str)
                .collect::<Vec<_>>()
    {
        return Err(TtsError::IncompatibleContract(
            "codec graph I/O does not match pinned metadata".into(),
        ));
    }
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

#[derive(Parser)]
#[command(
    name = "zerotts",
    about = "Local 48 kHz ZeroTTS streaming reference runner"
)]
struct Args {
    /// Text passed directly to the ZeroTTS tokenizer, as in Python synthesize_stream.
    text: String,
    #[arg(long, default_value = "models/zerotts")]
    model_dir: PathBuf,
    #[arg(long)]
    ort_library: Option<PathBuf>,
    #[arg(long, default_value_t = 2)]
    threads: i32,
    #[arg(long, default_value_t = 1500)]
    max_frames: usize,
    #[arg(long, default_value_t = 2)]
    repeats: usize,
    #[arg(long)]
    no_warmup: bool,
    /// Write the final run as mono float32 PCM WAV at 48 kHz.
    #[arg(long)]
    out: PathBuf,
}

fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    anyhow::ensure!(!args.text.trim().is_empty(), "text must not be empty");
    anyhow::ensure!(
        args.repeats > 0 && args.max_frames > 0,
        "repeats and max-frames must be positive"
    );
    let library = args
        .ort_library
        .or_else(|| std::env::var_os("VOICE_ONNX_RUNTIME_LIB").map(PathBuf::from))
        .ok_or_else(|| anyhow::anyhow!("pass --ort-library or set VOICE_ONNX_RUNTIME_LIB"))?;
    let model = &args.model_dir;
    let codec = model.join("onnx/codec");
    let started = Instant::now();
    let contract = ZeroTtsContract::load_engine(
        &model.join("config.json"),
        &model.join("tokenizer.json"),
        &model.join("voices/maichi/voice.npz"),
        &model.join("onnx/text_encoder.onnx"),
        &model.join("onnx/prefix_step.onnx"),
        &model.join("onnx/local_frame_decode.onnx"),
        &codec.join("moss_audio_tokenizer_decode_full.onnx"),
        &codec.join("moss_audio_tokenizer_decode_step.onnx"),
        &codec.join("moss_audio_tokenizer_decode_shared.data"),
        &codec.join("codec_browser_onnx_meta.json"),
        &library,
        args.threads,
    )?;
    let mut stream = ZeroTtsPcmStream::new(&contract)?;
    if !args.no_warmup {
        stream.synthesize("ZeroTTS startup readiness.", 256, &mut |_| Ok(()))?;
        stream.reset()?;
    }
    eprintln!("init_ms={:.1}", started.elapsed().as_secs_f64() * 1000.0);

    let mut final_pcm = Vec::new();
    for run in 0..args.repeats {
        let started = Instant::now();
        let mut first_ms = None;
        let mut chunks = 0;
        let mut pcm = Vec::new();
        stream.synthesize(&args.text, args.max_frames, &mut |chunk| {
            if chunk.sample_rate_hz() != 48_000 {
                return Err(TtsError::IncompatibleContract(
                    "codec did not return 48 kHz PCM".into(),
                ));
            }
            if first_ms.is_none() {
                first_ms = Some(started.elapsed().as_secs_f64() * 1000.0);
            }
            chunks += 1;
            pcm.extend_from_slice(chunk.samples());
            Ok(())
        })?;
        let elapsed = started.elapsed().as_secs_f64();
        let duration = pcm.len() as f64 / 48_000.0;
        println!(
            "run={} first_pcm_ms={:.1} synthesis_ms={:.1} audio_ms={:.1} rtf={:.3} chunks={} samples={}",
            run + 1,
            first_ms.unwrap_or(f64::NAN),
            elapsed * 1000.0,
            duration * 1000.0,
            elapsed / duration,
            chunks,
            pcm.len()
        );
        final_pcm = pcm;
        stream.reset()?;
    }
    let mut writer = hound::WavWriter::create(
        &args.out,
        hound::WavSpec {
            channels: 1,
            sample_rate: 48_000,
            bits_per_sample: 32,
            sample_format: hound::SampleFormat::Float,
        },
    )?;
    for sample in final_pcm {
        writer.write_sample(sample)?;
    }
    writer.finalize()?;
    println!("wav={}", args.out.display());
    Ok(())
}
