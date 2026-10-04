use super::*;
use std::sync::atomic::{AtomicBool, Ordering};

const FIRST_CHUNK_FRAMES: usize = 4;
const MAX_CHUNK_FRAMES: usize = 16;
const MIN_FRAMES: usize = 4;
const EOA_EXTRA_FRAMES: usize = 1;
const INTER_SEGMENT_SILENCE_FRAMES: usize = 4;
/// Shortest text that still drives every startup hot-path graph once. Readiness must not scale
/// with utterance length: it validates native sessions, not linguistic quality.
const WARMUP_TEXT: &str = "Xin chao ZeroTTS";

/// How many autoregressive steps one pass may run.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum FrameBudget {
    /// Production synthesis decodes until end-of-audio plus its single trailing frame, bounded by
    /// the given ceiling.
    UntilEndOfAudio(usize),
    /// Startup readiness runs exactly this many autoregressive steps and never requires
    /// end-of-audio, so materializing a runtime never decodes a whole utterance.
    Readiness(usize),
}

/// Per-turn autoregressive state. Nothing here survives a request: the whole value is created for
/// one synthesis pass and dropped with it, so two Voice Sessions can never observe each other's
/// packed KV, validity mask, or repetition history.
struct ZeroTtsTurnState {
    hidden: Vec<f32>,
    packed_kv: Data<f32>,
    valid: Data<bool>,
    seen: Vec<bool>,
}

/// Startup readiness requires terminal, finite, non-empty 48 kHz mono PCM from the codec.
fn validate_warmup_pcm(pcm: &PcmF32Mono) -> Result<(), TtsError> {
    if pcm.sample_rate_hz() != 48_000
        || pcm.samples().is_empty()
        || pcm.samples().iter().any(|sample| !sample.is_finite())
    {
        return Err(TtsError::InvalidWarmupPcm);
    }
    Ok(())
}

/// What one bounded readiness pass actually executed. Reported instead of inferred, so a
/// qualification gate can prove startup warmup stayed bounded rather than reading a duration.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct WarmupReport {
    pub autoregressive_frames: usize,
    pub codec_decodes: usize,
    pub pcm_samples: usize,
}

/// Complete per-segment inference for file delivery. This path never calls decode_step.
pub struct ZeroTtsFullPcm {
    operation: ZeroTtsOperation,
    codec: FullCodecOperation,
    first_segment: bool,
}

impl ZeroTtsFullPcm {
    pub fn new(contract: &ZeroTtsContract) -> Result<Self, TtsError> {
        Ok(Self {
            operation: ZeroTtsOperation::new(contract)?,
            codec: FullCodecOperation::new(contract)?,
            first_segment: true,
        })
    }

    pub fn synthesize(
        &mut self,
        text: &str,
        max_frames: usize,
        cancelled: &AtomicBool,
    ) -> Result<PcmF32Mono, TtsError> {
        let voice = self.operation.contract.readiness_voice()?;
        self.synthesize_with_voice(text, max_frames, cancelled, &voice)
    }

    pub fn synthesize_with_voice(
        &mut self,
        text: &str,
        max_frames: usize,
        cancelled: &AtomicBool,
        voice: &Array3<f32>,
    ) -> Result<PcmF32Mono, TtsError> {
        if cancelled.load(Ordering::Acquire) {
            return Err(TtsError::Failed);
        }
        tracing::info!(
            first_segment = self.first_segment,
            "ZeroTTS full inference started"
        );
        let spoken_text = text::normalize_vi_text(text);
        let mut frames = CodeFrames::default();
        let eoa = self.operation.synthesize_with_frame_sink(
            &spoken_text,
            FrameBudget::UntilEndOfAudio(max_frames),
            voice,
            Some(&mut frames),
            |_, _, _| {
                if cancelled.load(Ordering::Acquire) {
                    Err(TtsError::Failed)
                } else {
                    Ok(())
                }
            },
        )?;
        if eoa.is_none() {
            return Err(TtsError::IncompatibleContract(
                "ZeroTTS synthesis reached its frame bound".into(),
            ));
        }
        if !self.first_segment {
            let silence = &self.operation.contract.silence_frame;
            frames.frames.splice(
                ..0,
                std::iter::repeat_n(silence.clone(), INTER_SEGMENT_SILENCE_FRAMES),
            );
        }
        if cancelled.load(Ordering::Acquire) {
            return Err(TtsError::Failed);
        }
        let pcm = self.codec.decode(&frames.frames)?;
        if cancelled.load(Ordering::Acquire) {
            return Err(TtsError::Failed);
        }
        self.first_segment = false;
        Ok(pcm)
    }

    /// Runs one bounded readiness pass: every hot-path autoregressive graph plus the full codec
    /// decoder execute once, and the caller still resets the retained worker afterwards.
    pub fn warmup(&mut self, voice: &Array3<f32>) -> Result<WarmupReport, TtsError> {
        let (frames, autoregressive_frames) = self.operation.warmup_frames(WARMUP_TEXT, voice)?;
        let pcm = self.codec.decode(&frames)?;
        validate_warmup_pcm(&pcm)?;
        Ok(WarmupReport {
            autoregressive_frames,
            codec_decodes: 1,
            pcm_samples: pcm.samples().len(),
        })
    }

    pub fn reset(&mut self) {
        self.first_segment = true;
    }
}

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
        let voice = self.operation.contract.readiness_voice()?;
        self.synthesize_with_voice(text, max_frames, &voice, on_pcm)
    }

    pub fn synthesize_with_voice(
        &mut self,
        text: &str,
        max_frames: usize,
        voice: &Array3<f32>,
        on_pcm: &mut dyn FnMut(PcmF32Mono) -> Result<(), TtsError>,
    ) -> Result<(), TtsError> {
        let spoken_text = text::normalize_vi_text(text);
        tracing::info!(
            first_segment = self.first_segment,
            "ZeroTTS inference started"
        );
        let mut frames = Vec::new();
        let mut target = if self.first_segment {
            FIRST_CHUNK_FRAMES
        } else {
            MAX_CHUNK_FRAMES
        };
        if !self.first_segment {
            let silence =
                vec![self.operation.contract.silence_frame.clone(); INTER_SEGMENT_SILENCE_FRAMES];
            on_pcm(self.codec.decode_step(&silence)?)?;
        }
        let ramp = self.first_segment;
        let result = self.operation.synthesize_with_frame_sink(
            &spoken_text,
            FrameBudget::UntilEndOfAudio(max_frames),
            voice,
            None,
            |frame, _index, _is_eoa| {
                frames.push(frame.to_vec());
                if frames.len() >= target {
                    let pcm = self.codec.decode_step(&frames)?;
                    if ramp && target == FIRST_CHUNK_FRAMES {
                        let samples = pcm.samples();
                        let peak = samples
                            .iter()
                            .fold(0.0_f32, |peak, sample| peak.max(sample.abs()));
                        let rms = (samples.iter().map(|sample| sample * sample).sum::<f32>()
                            / samples.len() as f32)
                            .sqrt();
                        tracing::info!(
                            samples = samples.len(),
                            peak,
                            rms,
                            "ZeroTTS first PCM chunk"
                        );
                    }
                    on_pcm(pcm)?;
                    frames.clear();
                    if ramp {
                        target = (target * 2).min(MAX_CHUNK_FRAMES);
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

    /// Runs one bounded readiness pass: every hot-path autoregressive graph plus the streaming
    /// codec decoder execute once, and the caller still resets the retained worker afterwards.
    pub fn warmup(&mut self, voice: &Array3<f32>) -> Result<WarmupReport, TtsError> {
        let (frames, autoregressive_frames) = self.operation.warmup_frames(WARMUP_TEXT, voice)?;
        let pcm = self.codec.decode_step(&frames)?;
        validate_warmup_pcm(&pcm)?;
        Ok(WarmupReport {
            autoregressive_frames,
            codec_decodes: 1,
            pcm_samples: pcm.samples().len(),
        })
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

    pub(super) fn synthesize(
        &mut self,
        text: &str,
        max_frames: usize,
    ) -> Result<CodeFrames, TtsError> {
        let voice = self.contract.readiness_voice()?;
        let mut diagnostics = CodeFrames::default();
        self.synthesize_with_frame_sink(
            text,
            FrameBudget::UntilEndOfAudio(max_frames),
            &voice,
            Some(&mut diagnostics),
            |_, _, _| Ok(()),
        )?;
        Ok(diagnostics)
    }

    /// Bounded readiness pass. Runs the text encoder, the cold prefix step, exactly one local frame
    /// decode and one prefix frame step, then stops. No end-of-audio loop, no retained state.
    pub(super) fn warmup_frames(
        &mut self,
        text: &str,
        voice: &Array3<f32>,
    ) -> Result<(Vec<Vec<i32>>, usize), TtsError> {
        let mut frames = Vec::new();
        self.synthesize_with_frame_sink(
            &text::normalize_vi_text(text),
            FrameBudget::Readiness(1),
            voice,
            None,
            |codes, _, _| {
                frames.push(codes.to_vec());
                Ok(())
            },
        )?;
        let autoregressive_frames = frames.len();
        Ok((frames, autoregressive_frames))
    }

    /// One autoregressive step: sample codes for the current position, record them in the turn's
    /// repetition history, and report whether the model signalled end-of-audio.
    fn local_decode_frame(
        &mut self,
        state: &mut ZeroTtsTurnState,
        frame_index: usize,
        forbid_eoa: bool,
    ) -> Result<(Vec<i32>, bool), TtsError> {
        let config = &self.contract.config;
        let decoded = self
            .local_frame_decode
            .run(ort::inputs! {
                "global_hidden" => tensor(vec![1, config.d_model], state.hidden.clone())?,
                "forbid_eoa" => tensor(vec![1], vec![forbid_eoa])?, "text_temperature" => tensor(vec![1], vec![1.0_f32])?, "text_topk" => tensor(vec![1], vec![50_i64])?,
                "audio_temperature" => tensor(vec![1], vec![0.8_f32])?, "audio_topk" => tensor(vec![1], vec![25_i64])?, "audio_topp" => tensor(vec![1], vec![0.95_f32])?, "audio_repetition_penalty" => tensor(vec![1], vec![1.2_f32])?,
                "seen_mask" => tensor(vec![1, config.num_codebooks, config.codebook_size], state.seen.clone())?,
                "ctrl_random_u" => tensor(vec![1], vec![draw(frame_index, 0)])?, "audio_random_u" => tensor(vec![1, config.num_codebooks], (0..config.num_codebooks).map(|n| draw(frame_index, n + 1)).collect::<Vec<_>>())?, "cfg_scale" => tensor(vec![1], vec![1.0_f32])?,
            })
            .map_err(contract_error)?;
        let codes = i32_tensor(&decoded["codes"])?.data;
        for (book, code) in codes.iter().enumerate() {
            let code = usize::try_from(*code).map_err(contract_error)?;
            if code >= config.codebook_size {
                return Err(TtsError::IncompatibleContract(
                    "out-of-range audio code".into(),
                ));
            }
            state.seen[book * config.codebook_size + code] = true;
        }
        let is_eoa = i32_tensor(&decoded["is_eoa"])?.data[0] != 0;
        drop(decoded);
        Ok((codes, is_eoa))
    }

    /// Cold prefix step: seed the turn-local KV from the voice queries plus the start-of-audio
    /// embedding, before any frame has been decoded.
    fn cold_prefix_step(
        &mut self,
        voice: &Array3<f32>,
        soa: &Data<f32>,
        cross: &Data<f32>,
        text_valid: &Data<bool>,
    ) -> Result<ZeroTtsTurnState, TtsError> {
        let config = &self.contract.config;
        let voice_queries = config.n_voice_queries;
        let mut embed = voice.iter().copied().collect::<Vec<_>>();
        embed.extend(soa.data.iter().copied());
        let prefix_out = self.prefix_step.run(ort::inputs! {
            "external_embed" => tensor(vec![1, voice_queries + 1, config.d_model], embed)?,
            "use_external_embed" => tensor(vec![1, voice_queries + 1], vec![true; voice_queries + 1])?,
            "frame_codes" => tensor(vec![1, voice_queries + 1, config.num_codebooks], vec![0_i64; (voice_queries + 1) * config.num_codebooks])?,
            "new_pos" => tensor(vec![1, voice_queries + 1], (0..=voice_queries).map(|x| x as i64).collect::<Vec<_>>())?,
            "new_valid" => tensor(vec![1, voice_queries + 1], vec![true; voice_queries + 1])?,
            "packed_kv" => tensor(vec![config.n_layers, 2, 1, config.n_heads, 0, config.d_model / config.n_heads], Vec::<f32>::new())?,
            "new_bidirectional" => tensor(vec![1, voice_queries + 1], (0..voice_queries).map(|_| true).chain(std::iter::once(false)).collect::<Vec<_>>())?,
            "past_valid" => tensor(vec![1, 0], Vec::<bool>::new())?,
            "cross_kv" => tensor(cross.shape.clone(), cross.data.clone())?,
            "text_valid" => tensor(text_valid.shape.clone(), text_valid.data.clone())?,
        }).map_err(contract_error)?;
        Ok(ZeroTtsTurnState {
            hidden: final_hidden(&prefix_out["hidden"], config.d_model)?,
            packed_kv: f32_tensor(&prefix_out["new_packed_kv"])?,
            valid: bool_tensor(&prefix_out["full_valid"])?,
            seen: vec![false; config.num_codebooks * config.codebook_size],
        })
    }

    /// Prefix step for one decoded frame: fold the codes into the turn-local KV and produce the
    /// hidden state the next local frame decode consumes.
    fn prefix_frame_step(
        &mut self,
        state: &mut ZeroTtsTurnState,
        codes: &[i32],
        frame_index: usize,
        cross: &Data<f32>,
        text_valid: &Data<bool>,
    ) -> Result<(), TtsError> {
        let config = &self.contract.config;
        let position = self.contract.frame_position(frame_index);
        let prefix_out = self.prefix_step.run(ort::inputs! {
            "external_embed" => tensor(vec![1, 1, config.d_model], vec![0.0_f32; config.d_model])?, "use_external_embed" => tensor(vec![1, 1], vec![false])?, "frame_codes" => tensor(vec![1, 1, config.num_codebooks], codes.iter().map(|code| i64::from(*code)).collect::<Vec<_>>())?, "new_pos" => tensor(vec![1, 1], vec![position])?, "new_valid" => tensor(vec![1, 1], vec![true])?, "packed_kv" => tensor(state.packed_kv.shape.clone(), state.packed_kv.data.clone())?, "new_bidirectional" => tensor(vec![1, 1], vec![false])?, "past_valid" => tensor(state.valid.shape.clone(), state.valid.data.clone())?, "cross_kv" => tensor(cross.shape.clone(), cross.data.clone())?, "text_valid" => tensor(text_valid.shape.clone(), text_valid.data.clone())?,
        }).map_err(contract_error)?;
        state.hidden = final_hidden(&prefix_out["hidden"], config.d_model)?;
        state.packed_kv = f32_tensor(&prefix_out["new_packed_kv"])?;
        state.valid = bool_tensor(&prefix_out["full_valid"])?;
        Ok(())
    }

    pub(super) fn synthesize_with_frame_sink<F>(
        &mut self,
        text: &str,
        budget: FrameBudget,
        voice: &Array3<f32>,
        mut diagnostics: Option<&mut CodeFrames>,
        mut on_frame: F,
    ) -> Result<Option<usize>, TtsError>
    where
        F: FnMut(&[i32], usize, bool) -> Result<(), TtsError>,
    {
        let limit = match budget {
            FrameBudget::UntilEndOfAudio(max_frames) => max_frames,
            FrameBudget::Readiness(steps) => steps,
        };
        let ids = self.contract.encode(text)?;
        let text_out = self
            .text_encoder
            .run(ort::inputs! {
                "text_ids" => tensor(vec![1, ids.len()], ids.clone())?,
                "txt_lengths" => tensor(vec![1], vec![ids.len() as i64])?,
            })
            .map_err(contract_error)?;
        if let Some(diagnostics) = diagnostics.as_deref_mut() {
            diagnostics.token_ids = ids.clone();
            diagnostics.text_state_checksum = checksum(&f32_tensor(&text_out["text_states"])?.data);
        }
        let text_valid = bool_tensor(&text_out["text_valid"])?;
        let soa = f32_tensor(&text_out["soa_embed"])?;
        let cross = f32_tensor(&text_out["cross_kv"])?;
        drop(text_out);
        let mut state = self.cold_prefix_step(voice, &soa, &cross, &text_valid)?;
        drop(soa);
        let mut eoa = None;
        let mut tail_remaining = None;
        for frame_index in 0..limit {
            // The trailing frame after EOA is still decoded, but must not be able to end again.
            let forbid_eoa = frame_index < MIN_FRAMES || tail_remaining.is_some();
            let (codes, is_eoa) = self.local_decode_frame(&mut state, frame_index, forbid_eoa)?;
            if let Some(diagnostics) = diagnostics.as_deref_mut() {
                diagnostics.frames.push(codes.clone());
                diagnostics
                    .frame_positions
                    .push(self.contract.frame_position(frame_index));
                diagnostics
                    .seen_code_counts
                    .push(state.seen.iter().filter(|value| **value).count());
                if is_eoa && diagnostics.eoa.is_none() {
                    diagnostics.eoa = Some(frame_index);
                }
            }
            on_frame(&codes, frame_index, is_eoa)?;
            match budget {
                // Readiness exercises the frame step as well, then stops without decoding speech.
                FrameBudget::Readiness(steps) => {
                    self.prefix_frame_step(&mut state, &codes, frame_index, &cross, &text_valid)?;
                    if frame_index + 1 == steps {
                        break;
                    }
                }
                FrameBudget::UntilEndOfAudio(_) => {
                    if let Some(remaining) = tail_remaining.as_mut() {
                        *remaining -= 1;
                        if *remaining == 0 {
                            break;
                        }
                    } else if is_eoa {
                        eoa = Some(frame_index);
                        tail_remaining = Some(EOA_EXTRA_FRAMES);
                    }
                    self.prefix_frame_step(&mut state, &codes, frame_index, &cross, &text_valid)?;
                }
            }
        }
        Ok(
            if matches!(budget, FrameBudget::UntilEndOfAudio(_))
                && tail_remaining.is_none_or(|remaining| remaining == 0)
            {
                eoa
            } else {
                None
            },
        )
    }
}

#[derive(Default)]
pub struct CodeFrames {
    pub token_ids: Vec<i64>,
    pub text_state_checksum: f64,
    pub frames: Vec<Vec<i32>>,
    pub frame_positions: Vec<i64>,
    pub seen_code_counts: Vec<usize>,
    pub eoa: Option<usize>,
}

fn checksum(values: &[f32]) -> f64 {
    values
        .iter()
        .enumerate()
        .map(|(i, value)| f64::from(*value) * (i + 1) as f64)
        .sum()
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
