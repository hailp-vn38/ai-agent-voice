use super::*;

pub(super) const SPECIAL_TOKENS: [(&str, u32); 8] = [
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
pub(super) struct Config {
    pub(super) text_format: String,
    pub(super) vocab_size: usize,
    pub(super) num_codebooks: usize,
    pub(super) codebook_size: usize,
    pub(super) d_model: usize,
    pub(super) n_heads: usize,
    pub(super) n_layers: usize,
    pub(super) n_voice_queries: usize,
    pub(super) sample_rate: u32,
    pub(super) codec_frame_rate: f32,
    pub(super) special_tokens: BTreeMap<String, u32>,
}

/// Immutable values shared by every synthesis operation for one pinned pack.
#[derive(Clone)]
pub struct ZeroTtsContract {
    pub(super) tokenizer: Tokenizer,
    pub(super) config: Config,
    pub(super) voice: Option<Array3<f32>>,
    pub(super) silence_frame: Vec<i32>,
    pub(super) graphs: Option<GraphPaths>,
}

/// Immutable graph sources. Runtime workers load their own sessions from these paths once.
#[derive(Clone)]
pub(super) struct GraphPaths {
    pub(super) text_encoder: PathBuf,
    pub(super) prefix_step: PathBuf,
    pub(super) local_frame_decode: PathBuf,
    pub(super) threads: usize,
    pub(super) codec: CodecPaths,
}

#[derive(Clone)]
pub(super) struct CodecPaths {
    pub(super) decode_full: PathBuf,
    pub(super) decode_step: PathBuf,
    pub(super) metadata: CodecMetadata,
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
            silence_frame: Vec::new(),
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
        silence_frame_path: &Path,
        runtime_library: &Path,
        num_threads: i32,
    ) -> Result<Self, TtsError> {
        let config: Config =
            serde_json::from_slice(&fs::read(config_path).map_err(contract_error)?)
                .map_err(contract_error)?;
        validate_config(&config)?;
        let tokenizer = load_tokenizer(tokenizer_path)?;
        let voice = load_voice(voice_path, &config)?;
        let silence_frame = load_silence_frame(
            silence_frame_path,
            config.num_codebooks,
            config.codebook_size,
        )?;
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
            silence_frame,
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

    pub(crate) fn validate_full_decode(&self, frames: &[Vec<i32>]) -> Result<(), TtsError> {
        let pcm = validate_full_decode(self, frames)?;
        if pcm.samples().is_empty() || pcm.sample_rate_hz() != self.config.sample_rate {
            return Err(TtsError::InvalidWarmupPcm);
        }
        Ok(())
    }

    /// Produces the provider boundary PCM. Codec layout and profile are validated at startup.
    pub fn synthesize_pcm(
        &self,
        text: &str,
        max_frames: usize,
    ) -> Result<crate::audio::PcmF32Mono, TtsError> {
        let mut samples = Vec::new();
        self.synthesize_pcm_stream(text, max_frames, &mut |pcm| {
            samples.extend_from_slice(pcm.samples());
            Ok(())
        })?;
        Ok(PcmF32Mono::new(samples, self.config.sample_rate))
    }

    /// Mirrors ZeroTTS `synthesize_stream`: generate AR frames and pass ramped groups through
    /// the KV-cached codec decoder immediately.  No complete utterance PCM is retained here.
    pub fn synthesize_pcm_stream(
        &self,
        text: &str,
        max_frames: usize,
        on_pcm: &mut dyn FnMut(crate::audio::PcmF32Mono) -> Result<(), TtsError>,
    ) -> Result<(), TtsError> {
        let mut stream = ZeroTtsPcmStream::new(self)?;
        stream.synthesize(text, max_frames, on_pcm)
    }
}

fn load_silence_frame(
    path: &Path,
    num_codebooks: usize,
    codebook_size: usize,
) -> Result<Vec<i32>, TtsError> {
    let frame: ndarray::Array2<i64> = ndarray_npy::read_npy(path).map_err(contract_error)?;
    if frame.shape() != [1, num_codebooks] {
        return Err(TtsError::IncompatibleContract(
            "ZeroTTS silence frame shape does not match the pinned codec".into(),
        ));
    }
    frame
        .iter()
        .copied()
        .map(|code| {
            usize::try_from(code)
                .ok()
                .filter(|code| *code < codebook_size)
                .and_then(|code| i32::try_from(code).ok())
                .ok_or_else(|| {
                    TtsError::IncompatibleContract(
                        "ZeroTTS silence frame contains an out-of-range code".into(),
                    )
                })
        })
        .collect()
}

/// Codec state is scoped to one SpeechOutput delivery, while every text segment still creates a
/// fresh ZeroTtsOperation for its AR/KV state.
pub(super) fn tokenizer_special_id(document: &serde_json::Value, token: &str) -> Option<u32> {
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

pub(super) const TEXT_ENCODER_INPUTS: &[(&str, TensorElementType, usize)] = &[
    ("text_ids", TensorElementType::Int64, 2),
    ("txt_lengths", TensorElementType::Int64, 1),
];
pub(super) const TEXT_ENCODER_OUTPUTS: &[(&str, TensorElementType, usize)] = &[
    ("text_states", TensorElementType::Float32, 3),
    ("text_valid", TensorElementType::Bool, 2),
    ("soa_embed", TensorElementType::Float32, 3),
    ("cross_kv", TensorElementType::Float32, 6),
];
pub(super) const PREFIX_STEP_INPUTS: &[(&str, TensorElementType, usize)] = &[
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
pub(super) const PREFIX_STEP_OUTPUTS: &[(&str, TensorElementType, usize)] = &[
    ("hidden", TensorElementType::Float32, 3),
    ("new_packed_kv", TensorElementType::Float32, 6),
    ("full_valid", TensorElementType::Bool, 2),
];
pub(super) const LOCAL_FRAME_INPUTS: &[(&str, TensorElementType, usize)] = &[
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
pub(super) const LOCAL_FRAME_OUTPUTS: &[(&str, TensorElementType, usize)] = &[
    ("is_eoa", TensorElementType::Int32, 1),
    ("codes", TensorElementType::Int32, 2),
];

pub(super) fn load_tokenizer(path: &Path) -> Result<Tokenizer, TtsError> {
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

pub(super) fn load_voice(path: &Path, config: &Config) -> Result<Array3<f32>, TtsError> {
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

pub(super) fn load_graph(
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

pub(super) fn validate_graph_io(
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

pub(super) fn validate_config(config: &Config) -> Result<(), TtsError> {
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
