mod graphs;
mod heads;
mod nano;
mod sampling;

use std::{
    collections::VecDeque,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

use ndarray::{Array1, Axis};
use ort::{session::Session, value::DynValue};

use super::{
    assets::{VieneuModelAssets, VieneuNativeAssets},
    config::VieneuNativeConfig,
    contract::VieneuContract,
    text::{VieneuTextFrontend, frame_limit},
};
use crate::{
    audio::PcmF32Mono,
    providers::tts::{
        TtsBinding, TtsError, TtsSynthesisRequest, TtsWorker, warmup_retained_worker,
        zerotts::runtime::codec::ZeroTtsCodecReplica as MossCodec,
    },
};
use graphs::{hidden, take_cache, tensor, with_cache};
use heads::Heads;
use nano::{NanoNativeState, NanoWorker};

fn failure(error: impl std::fmt::Display) -> TtsError {
    TtsError::IncompatibleContract(format!("VieNeu native: {error}"))
}

pub(super) struct TurboNativeState {
    contract: VieneuContract,
    heads: Heads,
    frontend: VieneuTextFrontend,
}

impl TurboNativeState {
    pub fn load(assets: &VieneuNativeAssets) -> Result<Self, TtsError> {
        let contract = VieneuContract::load(assets)?;
        let heads = Heads::load(&assets.heads, &contract.config)?;
        let frontend = VieneuTextFrontend::load(&assets.g2p_dictionary)?;
        Ok(Self {
            contract,
            heads,
            frontend,
        })
    }
}

pub(super) enum NativeState {
    Turbo(Arc<TurboNativeState>),
    Nano(Arc<NanoNativeState>),
}

impl NativeState {
    pub fn load(assets: &VieneuModelAssets) -> Result<Self, TtsError> {
        match assets {
            VieneuModelAssets::Turbo(turbo) => {
                Ok(Self::Turbo(Arc::new(TurboNativeState::load(turbo)?)))
            }
            VieneuModelAssets::Nano(nano) => Ok(Self::Nano(Arc::new(NanoNativeState::load(nano)?))),
        }
    }

    pub const fn sample_rate_hz(&self) -> u32 {
        match self {
            Self::Turbo(_) => 48_000,
            Self::Nano(_) => 24_000,
        }
    }

    pub fn supports_voice(&self, voice_id: &str) -> bool {
        match self {
            Self::Turbo(state) => state.contract.voices.presets.contains_key(voice_id),
            Self::Nano(state) => state.contract.voices.presets.contains_key(voice_id),
        }
    }
}

pub(super) struct TurboWorker {
    state: Arc<TurboNativeState>,
    options: VieneuNativeConfig,
    prefill: Session,
    decode: Session,
    acoustic: Session,
    codec: MossCodec,
}

pub(super) enum VieneuWorker {
    Turbo(TurboWorker),
    Nano(NanoWorker),
}

impl VieneuWorker {
    pub fn open(
        state: &NativeState,
        assets: &VieneuModelAssets,
        options: &VieneuNativeConfig,
    ) -> Result<Self, TtsError> {
        match (state, assets) {
            (NativeState::Turbo(state), VieneuModelAssets::Turbo(assets)) => Ok(Self::Turbo(
                TurboWorker::open(Arc::clone(state), assets, options)?,
            )),
            (NativeState::Nano(state), VieneuModelAssets::Nano(assets)) => Ok(Self::Nano(
                NanoWorker::open(Arc::clone(state), assets, options)?,
            )),
            _ => Err(TtsError::Failed),
        }
    }
}

impl TurboWorker {
    pub fn open(
        state: Arc<TurboNativeState>,
        assets: &VieneuNativeAssets,
        options: &VieneuNativeConfig,
    ) -> Result<Self, TtsError> {
        let config = &state.contract.config;
        let threads = usize::try_from(options.num_threads).map_err(failure)?;
        Ok(Self {
            prefill: graphs::load(
                &assets.prefill,
                threads,
                config.num_hidden_layers,
                false,
                true,
            )?,
            decode: graphs::load(
                &assets.decode_step,
                threads,
                config.num_hidden_layers,
                false,
                false,
            )?,
            acoustic: graphs::load(
                &assets.acoustic,
                threads,
                config.local_num_hidden_layers,
                true,
                false,
            )?,
            codec: MossCodec::from_assets(
                &assets.codec_metadata,
                &assets.codec_step,
                &assets.codec_data,
                config.n_vq,
                threads,
            )?,
            state,
            options: options.clone(),
        })
    }

    fn acoustic_frame(
        &mut self,
        conditioning: &[f32],
        histories: &mut [VecDeque<i32>],
        cancelled: &AtomicBool,
    ) -> Result<(Vec<i32>, bool), TtsError> {
        let config = &self.state.contract.config;
        let width = config.hidden_size;
        let mut embeddings = conditioning.to_vec();
        embeddings.extend(
            self.state
                .heads
                .text
                .row(config.speech_generation_start_token_id),
        );
        let mut cache: Vec<(String, DynValue)> = Vec::new();
        for kind in ["k", "v"] {
            for layer in 0..config.local_num_hidden_layers {
                cache.push((
                    format!("past_{kind}_{layer}"),
                    tensor(
                        vec![
                            1,
                            config.local_num_attention_heads,
                            0,
                            width / config.local_num_attention_heads,
                        ],
                        Vec::<f32>::new(),
                    )?
                    .into(),
                ));
            }
        }
        let mut codes = Vec::with_capacity(config.n_vq);
        let mut eos = false;
        for (channel, history) in histories.iter_mut().enumerate() {
            check_cancelled(cancelled)?;
            let positions = if channel == 0 {
                vec![0_i64, 1]
            } else {
                vec![(channel + 1) as i64]
            };
            let sequence = positions.len();
            let inputs = vec![
                (
                    "token_emb".into(),
                    tensor(vec![1, sequence, width], embeddings)?.into(),
                ),
                (
                    "position_ids".into(),
                    tensor(vec![1, sequence], positions)?.into(),
                ),
            ];
            let mut output = self
                .acoustic
                .run(with_cache(inputs, &cache))
                .map_err(failure)?;
            let hidden = hidden(&output, width)?;
            if channel == 0 {
                let text_logits = self
                    .state
                    .heads
                    .text
                    .dot(&Array1::from_vec(hidden[..width].to_vec()));
                let text_id = text_logits
                    .iter()
                    .enumerate()
                    .max_by(|a, b| a.1.total_cmp(b.1))
                    .ok_or(TtsError::Failed)?
                    .0;
                eos = text_id == config.speech_generation_end_token_id;
            }
            let last = Array1::from_vec(hidden[hidden.len() - width..].to_vec());
            let logits = self
                .state
                .heads
                .audio
                .index_axis(Axis(0), channel)
                .dot(&last);
            let code = sampling::sample(logits.as_slice().ok_or(TtsError::Failed)?, history)?;
            sampling::remember(history, code);
            codes.push(code);
            let next = take_cache(&mut output, config.local_num_hidden_layers)?;
            drop(output);
            cache = next;
            embeddings = self
                .state
                .heads
                .audio
                .index_axis(Axis(0), channel)
                .row(code as usize)
                .to_vec();
        }
        Ok((codes, eos))
    }

    fn generate(
        &mut self,
        request: &TtsSynthesisRequest,
        cancelled: &AtomicBool,
        on_pcm: &mut dyn FnMut(PcmF32Mono) -> Result<(), TtsError>,
    ) -> Result<(), TtsError> {
        check_cancelled(cancelled)?;
        let selection = if request.selection == TtsBinding::readiness() {
            TtsBinding {
                voice: self.options.voice.clone(),
                language: self.options.language.clone(),
            }
        } else {
            request.selection.clone()
        };
        if selection.language != "vi-VN" {
            return Err(TtsError::UnsupportedLanguage(selection.language));
        }
        let voice = self.state.contract.voices.get(&selection.voice)?;
        let phonemes = self.state.frontend.phonemize(&request.text)?;
        let config = &self.state.contract.config;
        let encoding = self
            .state
            .contract
            .tokenizer
            .encode(phonemes.as_str(), false)
            .map_err(failure)?;
        let anchor = self.state.heads.anchor(&voice.speaker_embedding)?;
        let mut text_ids = vec![
            config.default_style_token_id,
            config.text_prompt_start_token_id,
        ];
        text_ids.extend(encoding.get_ids().iter().map(|id| *id as usize));
        text_ids.push(config.text_prompt_end_token_id);
        let prompt_length = text_ids.len() + voice.reference_codes.len();
        let frame_limit = frame_limit(&phonemes);
        if prompt_length + frame_limit > 2048 {
            return Err(TtsError::Failed);
        }
        let width = config.hidden_size;
        let layers = config.num_hidden_layers;
        let speech_token = config.speech_generation_start_token_id;
        let mut embeddings = Vec::with_capacity(prompt_length * width);
        for id in text_ids {
            embeddings.extend(self.state.heads.embed(id, &[], &anchor)?);
        }
        for codes in &voice.reference_codes {
            embeddings.extend(self.state.heads.embed(
                config.audio_ref_slot_token_id,
                codes,
                &anchor,
            )?);
        }
        check_cancelled(cancelled)?;
        let mut output = self
            .prefill
            .run(ort::inputs! { "inputs_embeds" => tensor(vec![1, prompt_length, width], embeddings)? })
            .map_err(failure)?;
        let values = hidden(&output, width)?;
        let mut conditioning = values[values.len() - width..].to_vec();
        let mut cache = take_cache(&mut output, layers)?;
        drop(output);
        let mut histories = vec![VecDeque::new(); self.state.contract.config.n_vq];
        let mut frames = Vec::with_capacity(4);
        for frame in 0..frame_limit {
            let (codes, eos) = self.acoustic_frame(&conditioning, &mut histories, cancelled)?;
            frames.push(codes.clone());
            if frames.len() == 4 || eos || frame + 1 == frame_limit {
                check_cancelled(cancelled)?;
                let pcm = self.codec.decode_step(&frames)?;
                check_cancelled(cancelled)?;
                on_pcm(pcm)?;
                frames.clear();
            }
            if eos || frame + 1 == frame_limit {
                return Ok(());
            }
            let embedded = self.state.heads.embed(speech_token, &codes, &anchor)?;
            let inputs = vec![
                (
                    "inputs_embeds".into(),
                    tensor(vec![1, 1, width], embedded)?.into(),
                ),
                (
                    "position_ids".into(),
                    tensor(vec![1, 1], vec![(prompt_length + frame) as i64])?.into(),
                ),
            ];
            check_cancelled(cancelled)?;
            let mut output = self
                .decode
                .run(with_cache(inputs, &cache))
                .map_err(failure)?;
            conditioning = hidden(&output, width)?;
            let next = take_cache(&mut output, layers)?;
            drop(output);
            cache = next;
        }
        Err(TtsError::Failed)
    }
}

pub(super) fn check_cancelled(cancelled: &AtomicBool) -> Result<(), TtsError> {
    if cancelled.load(Ordering::Acquire) {
        Err(TtsError::Failed)
    } else {
        Ok(())
    }
}

impl TtsWorker for VieneuWorker {
    fn warmup(&mut self) -> Result<(), TtsError> {
        match self {
            Self::Turbo(_) => warmup_retained_worker(self, "Xin chào.", 48_000),
            Self::Nano(worker) => worker.warmup(),
        }
    }

    fn synthesize(
        &mut self,
        request: &TtsSynthesisRequest,
        cancelled: &AtomicBool,
        on_pcm: &mut dyn FnMut(PcmF32Mono) -> Result<(), TtsError>,
    ) -> Result<(), TtsError> {
        match self {
            Self::Turbo(worker) => {
                worker.codec.reset()?;
                let result = worker.generate(request, cancelled, on_pcm);
                let reset = worker.codec.reset();
                result.and(reset)
            }
            Self::Nano(worker) => worker.synthesize(request, cancelled, on_pcm),
        }
    }

    fn reset(&mut self) -> Result<(), TtsError> {
        match self {
            Self::Turbo(worker) => worker.codec.reset(),
            Self::Nano(_) => Ok(()),
        }
    }
}
