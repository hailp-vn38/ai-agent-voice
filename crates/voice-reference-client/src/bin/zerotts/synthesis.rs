use super::*;

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
        if result.is_none() {
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

/// Mutable ONNX sessions and turn-local decode state for exactly one synthesis operation.
pub(super) struct ZeroTtsOperation {
    pub(super) contract: ZeroTtsContract,
    pub(super) text_encoder: Session,
    pub(super) prefix_step: Session,
    pub(super) local_frame_decode: Session,
}

impl ZeroTtsOperation {
    pub(super) fn new(contract: &ZeroTtsContract) -> Result<Self, TtsError> {
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

    pub(super) fn synthesize_with_frame_sink<F>(
        &mut self,
        text: &str,
        max_frames: usize,
        mut on_frame: F,
    ) -> Result<Option<usize>, TtsError>
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
        Ok(eoa)
    }
}
pub(super) struct Data<T> {
    pub(super) shape: Vec<usize>,
    pub(super) data: Vec<T>,
}
pub(super) fn tensor<
    T: ort::value::PrimitiveTensorElementType + Clone + std::fmt::Debug + 'static,
>(
    shape: Vec<usize>,
    data: Vec<T>,
) -> Result<Tensor<T>, TtsError> {
    Tensor::from_array((shape, data)).map_err(contract_error)
}
pub(super) fn f32_tensor(value: &ort::value::DynValue) -> Result<Data<f32>, TtsError> {
    let (s, d) = value.try_extract_tensor::<f32>().map_err(contract_error)?;
    Ok(Data {
        shape: s.iter().map(|x| *x as usize).collect(),
        data: d.to_vec(),
    })
}
pub(super) fn bool_tensor(value: &ort::value::DynValue) -> Result<Data<bool>, TtsError> {
    let (s, d) = value.try_extract_tensor::<bool>().map_err(contract_error)?;
    Ok(Data {
        shape: s.iter().map(|x| *x as usize).collect(),
        data: d.to_vec(),
    })
}
pub(super) fn i32_tensor(value: &ort::value::DynValue) -> Result<Data<i32>, TtsError> {
    let (s, d) = value.try_extract_tensor::<i32>().map_err(contract_error)?;
    Ok(Data {
        shape: s.iter().map(|x| *x as usize).collect(),
        data: d.to_vec(),
    })
}
pub(super) fn final_hidden(
    value: &ort::value::DynValue,
    width: usize,
) -> Result<Vec<f32>, TtsError> {
    let data = f32_tensor(value)?.data;
    data.get(data.len().saturating_sub(width)..)
        .map(ToOwned::to_owned)
        .ok_or_else(|| TtsError::IncompatibleContract("missing hidden output".into()))
}
pub(super) fn draw(frame: usize, stream: usize) -> f32 {
    let mut x = (frame as u64 + 1).wrapping_mul(0x9E3779B97F4A7C15) ^ stream as u64;
    x ^= x >> 30;
    x = x.wrapping_mul(0xBF58476D1CE4E5B9);
    (x >> 40) as f32 / (1_u32 << 24) as f32
}
