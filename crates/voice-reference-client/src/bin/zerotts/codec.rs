use super::*;

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct CodecMetadata {
    pub(super) format_version: u32,
    pub(super) checkpoint_path: String,
    pub(super) files: CodecFiles,
    pub(super) external_data_files: BTreeMap<String, Vec<String>>,
    pub(super) codec_config: CodecConfig,
    pub(super) onnx: CodecOnnx,
    pub(super) streaming_decode: StreamingDecode,
}

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct CodecFiles {
    pub(super) decode_full: String,
    pub(super) decode_step: String,
}

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct CodecConfig {
    pub(super) sample_rate: u32,
    pub(super) channels: usize,
    pub(super) downsample_rate: usize,
    pub(super) num_quantizers: usize,
}

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct CodecOnnx {
    pub(super) opset: u32,
    pub(super) decode_input_names: Vec<String>,
    pub(super) decode_output_names: Vec<String>,
    pub(super) decode_step_input_names: Vec<String>,
    pub(super) decode_step_output_names: Vec<String>,
}

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct StreamingDecode {
    pub(super) batch_size: usize,
    pub(super) transformer_offsets: Vec<TransformerOffset>,
    pub(super) attention_caches: Vec<AttentionCache>,
}
#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct TransformerOffset {
    pub(super) index: usize,
    pub(super) decoder_index: usize,
    pub(super) input_name: String,
    pub(super) output_name: String,
    pub(super) shape: Vec<usize>,
    pub(super) dtype: String,
}
#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct AttentionCache {
    pub(super) index: usize,
    pub(super) decoder_index: usize,
    pub(super) layer_index: usize,
    pub(super) context: usize,
    pub(super) num_heads: usize,
    pub(super) head_dim: usize,
    pub(super) offset_input_name: String,
    pub(super) offset_output_name: String,
    pub(super) cached_keys_input_name: String,
    pub(super) cached_keys_output_name: String,
    pub(super) cached_values_input_name: String,
    pub(super) cached_values_output_name: String,
    pub(super) cached_positions_input_name: String,
    pub(super) cached_positions_output_name: String,
    pub(super) offset_shape: Vec<usize>,
    pub(super) cache_shape: Vec<usize>,
    pub(super) positions_shape: Vec<usize>,
    pub(super) cache_dtype: String,
    pub(super) positions_dtype: String,
}

pub(super) struct CodecOperation {
    pub(super) contract: ZeroTtsContract,
    // The step graph is owned by one native worker; only its cache is reset between leases.
    pub(super) decode_step: Session,
    pub(super) streaming_state: BTreeMap<String, CodecStreamState>,
}

pub(super) enum CodecStreamState {
    I32 { shape: Vec<usize>, data: Vec<i32> },
    F32 { shape: Vec<usize>, data: Vec<f32> },
}

impl CodecOperation {
    pub(super) fn new(contract: &ZeroTtsContract) -> Result<Self, TtsError> {
        let graphs = contract
            .graphs
            .as_ref()
            .ok_or_else(|| TtsError::IncompatibleContract("ZeroTTS engine is not loaded".into()))?;
        let streaming_state = Self::initial_streaming_state(contract)?;
        Ok(Self {
            contract: contract.clone(),
            decode_step: load_codec_graph(
                &graphs.codec.decode_step,
                graphs.threads,
                &graphs.codec.metadata.onnx.decode_step_input_names,
                &graphs.codec.metadata.onnx.decode_step_output_names,
            )?,
            streaming_state,
        })
    }

    pub(super) fn initial_streaming_state(
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

    pub(super) fn reset(&mut self) -> Result<(), TtsError> {
        self.streaming_state = Self::initial_streaming_state(&self.contract)?;
        Ok(())
    }

    pub(super) fn decode_step(&mut self, frames: &[Vec<i32>]) -> Result<PcmF32Mono, TtsError> {
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

    pub(super) fn next_streaming_state(
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

    pub(super) fn mono_pcm(
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

pub(super) fn load_codec_metadata(
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

pub(super) fn load_codec_graph(
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
