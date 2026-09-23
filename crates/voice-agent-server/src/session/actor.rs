use crate::session::{
    ActiveTurnLimiter, SessionPhase,
    event::SessionEvent,
    speech_output::{SpeechOutput, SpeechOutputEvent},
    turn::DialogueHistory,
};
use crate::{
    audio::{
        CaptureOutcome, DecodeOutcome, ManualCapture, PcmF32Mono, UplinkOpusDecoder, VadBoundary,
        VadSegmenter, VadSegmenterConfig,
    },
    protocol::{ClientMessage, ListenCommand, ListenMode},
    providers::{ProviderSet, llm::UnavailableLlm, tts::UnavailableTts},
    workers::{
        AsrCommand, AsrStreamLease, AsrWorkerEvent, AsrWorkerRuntime, LlmRuntime, LlmRuntimeEvent,
        TtsWorkerRuntime, VadCommand, VadWorkerEvent, VadWorkerLease, VadWorkerRuntime,
        WorkerIdentity, WorkerRuntimeConfig,
    },
};
use std::collections::HashSet;

use tokio::sync::mpsc;
use tracing::{info, warn};

const MAX_DETECT_TEXT_SCALARS: usize = 4_096;

/// The only mutable owner of an accepted Voice Session's phase.
pub struct SessionActor {
    session_id: String,
    phase: SessionPhase,
    accepted_binary_frames: u64,
    uplink_decoder: UplinkOpusDecoder,
    manual_capture: ManualCapture,
    asr_runtime: std::sync::Arc<AsrWorkerRuntime>,
    asr_events: mpsc::Receiver<AsrWorkerEvent>,
    asr_stream: Option<(AsrStreamLease, WorkerIdentity)>,
    asr_cleanup_pending: HashSet<WorkerIdentity>,
    vad_runtime: std::sync::Arc<VadWorkerRuntime>,
    vad_events: mpsc::Receiver<VadWorkerEvent>,
    vad_session: Option<(VadWorkerLease, WorkerIdentity)>,
    listening_mode: Option<ListenMode>,
    auto_speech_active: bool,
    auto_reset_pending: bool,
    vad_segmenter: VadSegmenter,
    auto_retention: AutoPcmRetention,
    pre_roll_samples: u64,
    active_turn_limiter: std::sync::Arc<ActiveTurnLimiter>,
    has_active_turn_permit: bool,
    generation: u64,
    dialogue_history: DialogueHistory,
    llm_runtime: std::sync::Arc<LlmRuntime>,
    llm_events: mpsc::Receiver<LlmRuntimeEvent>,
    llm_operation: Option<WorkerIdentity>,
    generated_response: String,
    tts_runtime: std::sync::Arc<TtsWorkerRuntime>,
    speech_output: SpeechOutput,
    tts_started: bool,
    control_tx: mpsc::Sender<OutboundMessage>,
    audio_tx: mpsc::Sender<OutboundMessage>,
    /// A packet removed from SpeechOutput but not yet admitted by the bounded writer queue.
    /// It must be retried before polling another packet: dropping it creates audible gaps.
    pending_audio: Option<OutboundMessage>,
}

/// Application-owned runtimes shared by every voice session.
pub struct SessionRuntimes {
    pub asr: std::sync::Arc<AsrWorkerRuntime>,
    pub vad: std::sync::Arc<VadWorkerRuntime>,
    pub llm: std::sync::Arc<LlmRuntime>,
    pub tts: std::sync::Arc<TtsWorkerRuntime>,
    pub active_turn_limiter: std::sync::Arc<ActiveTurnLimiter>,
    pub vad_segmenter_config: VadSegmenterConfig,
    pub pre_roll_samples: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OutboundMessage {
    Text(String),
    Binary { generation: u64, packet: Vec<u8> },
    InvalidateAudio(u64),
    Close(u16),
}

impl OutboundMessage {
    pub fn as_text(&self) -> Option<&str> {
        match self {
            Self::Text(text) => Some(text),
            Self::Binary { .. } | Self::InvalidateAudio(_) | Self::Close(_) => None,
        }
    }
}

impl SessionActor {
    pub fn new(
        session_id: String,
        control_tx: mpsc::Sender<OutboundMessage>,
        audio_tx: mpsc::Sender<OutboundMessage>,
        max_capture_frames: usize,
        providers: std::sync::Arc<ProviderSet>,
    ) -> Result<Self, crate::audio::AudioError> {
        Self::new_with_history(
            session_id,
            control_tx,
            audio_tx,
            max_capture_frames,
            providers,
            20,
        )
    }

    pub fn new_with_history(
        session_id: String,
        control_tx: mpsc::Sender<OutboundMessage>,
        audio_tx: mpsc::Sender<OutboundMessage>,
        max_capture_frames: usize,
        providers: std::sync::Arc<ProviderSet>,
        max_history_messages: usize,
    ) -> Result<Self, crate::audio::AudioError> {
        let asr_runtime = std::sync::Arc::new(AsrWorkerRuntime::new(
            providers.asr_provider(),
            WorkerRuntimeConfig::default(),
        ));
        let vad_runtime = std::sync::Arc::new(VadWorkerRuntime::new(
            providers.vad_provider(),
            WorkerRuntimeConfig::default(),
        ));
        let llm_runtime = std::sync::Arc::new(LlmRuntime::new(
            providers.llm_provider(),
            1,
            std::time::Duration::from_secs(60),
        ));
        Self::new_with_runtimes_and_limiter(
            session_id,
            control_tx,
            audio_tx,
            max_capture_frames,
            max_history_messages,
            SessionRuntimes {
                asr: asr_runtime,
                vad: vad_runtime,
                llm: llm_runtime,
                tts: std::sync::Arc::new(TtsWorkerRuntime::new(
                    providers.tts_provider(),
                    WorkerRuntimeConfig::default(),
                )),
                active_turn_limiter: std::sync::Arc::new(ActiveTurnLimiter::new(8)),
                vad_segmenter_config: VadSegmenterConfig::default(),
                pre_roll_samples: 4_800,
            },
        )
        .and_then(|actor| actor.with_delivery_providers(&providers))
    }

    /// Production passes an application-owned runtime so ASR worker capacity is global.
    pub fn new_with_history_and_runtime(
        session_id: String,
        control_tx: mpsc::Sender<OutboundMessage>,
        audio_tx: mpsc::Sender<OutboundMessage>,
        max_capture_frames: usize,
        max_history_messages: usize,
        asr_runtime: std::sync::Arc<AsrWorkerRuntime>,
    ) -> Result<Self, crate::audio::AudioError> {
        // Kept for the Manual-only public seam used by earlier phases.
        let vad_runtime = std::sync::Arc::new(VadWorkerRuntime::new(
            std::sync::Arc::new(crate::providers::vad::UnavailableVad),
            WorkerRuntimeConfig::default(),
        ));
        Self::new_with_runtimes(
            session_id,
            control_tx,
            audio_tx,
            max_capture_frames,
            max_history_messages,
            asr_runtime,
            vad_runtime,
        )
    }

    /// Production passes both application-owned runtimes so capacity is global across sessions.
    pub fn new_with_runtimes(
        session_id: String,
        control_tx: mpsc::Sender<OutboundMessage>,
        audio_tx: mpsc::Sender<OutboundMessage>,
        max_capture_frames: usize,
        max_history_messages: usize,
        asr_runtime: std::sync::Arc<AsrWorkerRuntime>,
        vad_runtime: std::sync::Arc<VadWorkerRuntime>,
    ) -> Result<Self, crate::audio::AudioError> {
        Self::new_with_runtimes_and_limiter(
            session_id,
            control_tx,
            audio_tx,
            max_capture_frames,
            max_history_messages,
            SessionRuntimes {
                asr: asr_runtime,
                vad: vad_runtime,
                llm: std::sync::Arc::new(LlmRuntime::new(
                    std::sync::Arc::new(UnavailableLlm),
                    1,
                    std::time::Duration::from_secs(60),
                )),
                tts: std::sync::Arc::new(TtsWorkerRuntime::new(
                    std::sync::Arc::new(UnavailableTts),
                    WorkerRuntimeConfig::default(),
                )),
                active_turn_limiter: std::sync::Arc::new(ActiveTurnLimiter::new(8)),
                vad_segmenter_config: VadSegmenterConfig::default(),
                pre_roll_samples: 4_800,
            },
        )
    }

    pub fn new_with_runtimes_and_limiter(
        session_id: String,
        control_tx: mpsc::Sender<OutboundMessage>,
        audio_tx: mpsc::Sender<OutboundMessage>,
        max_capture_frames: usize,
        max_history_messages: usize,
        runtimes: SessionRuntimes,
    ) -> Result<Self, crate::audio::AudioError> {
        let retention_capacity = auto_retention_capacity(
            runtimes.vad.runtime_config().command_capacity,
            runtimes.vad_segmenter_config.min_speech_samples,
            runtimes.pre_roll_samples,
        );
        let asr_events = runtimes.asr.register_session(&session_id);
        let vad_events = runtimes.vad.register_session(&session_id);
        let llm_events = runtimes.llm.register_session(&session_id, 64);
        Ok(Self {
            session_id,
            phase: SessionPhase::Ready,
            accepted_binary_frames: 0,
            uplink_decoder: UplinkOpusDecoder::new()?,
            manual_capture: ManualCapture::new(max_capture_frames)?,
            asr_runtime: runtimes.asr,
            asr_events,
            asr_stream: None,
            asr_cleanup_pending: HashSet::new(),
            vad_runtime: runtimes.vad,
            vad_events,
            vad_session: None,
            listening_mode: None,
            auto_speech_active: false,
            auto_reset_pending: false,
            vad_segmenter: VadSegmenter::new(runtimes.vad_segmenter_config),
            auto_retention: AutoPcmRetention::new(retention_capacity),
            pre_roll_samples: runtimes.pre_roll_samples,
            active_turn_limiter: runtimes.active_turn_limiter,
            has_active_turn_permit: false,
            generation: 0,
            dialogue_history: DialogueHistory::new(max_history_messages),
            llm_runtime: runtimes.llm,
            llm_events,
            llm_operation: None,
            generated_response: String::new(),
            tts_runtime: std::sync::Arc::clone(&runtimes.tts),
            speech_output: SpeechOutput::with_worker(
                std::sync::Arc::new(UnavailableTts),
                runtimes.tts,
                crate::config::SpeechOutputConfig::default(),
            )?,
            tts_started: false,
            control_tx,
            audio_tx,
            pending_audio: None,
        })
    }

    /// Completes construction with the injected delivery providers at the application seam.
    pub fn with_delivery_providers(
        mut self,
        providers: &ProviderSet,
    ) -> Result<Self, crate::audio::AudioError> {
        let tts_runtime = std::sync::Arc::clone(&self.tts_runtime);
        self.speech_output = SpeechOutput::with_worker(
            providers.tts_provider(),
            tts_runtime,
            crate::config::SpeechOutputConfig::default(),
        )?;
        Ok(self)
    }

    pub fn with_delivery_providers_config(
        mut self,
        providers: &ProviderSet,
        config: crate::config::SpeechOutputConfig,
    ) -> Result<Self, crate::audio::AudioError> {
        let tts_runtime = std::sync::Arc::clone(&self.tts_runtime);
        self.speech_output =
            SpeechOutput::with_worker(providers.tts_provider(), tts_runtime, config)?;
        Ok(self)
    }

    pub fn session_id(&self) -> &str {
        &self.session_id
    }
    pub fn phase(&self) -> SessionPhase {
        self.phase
    }
    pub fn accepted_binary_frames(&self) -> u64 {
        self.accepted_binary_frames
    }
    pub fn dialogue_history(&self) -> &[String] {
        self.dialogue_history.messages()
    }

    /// Deterministic test helper: advances the runtime router then drains this session mailbox.
    /// Production routing is advanced only by `WorkerSupervisor`.
    pub fn pump_workers(&mut self) {
        self.asr_runtime.supervise_pending();
        self.vad_runtime.supervise_pending();
        self.drain_worker_events();
        self.drain_speech_output();
    }

    fn drain_worker_events(&mut self) {
        while let Ok(event) = self.asr_events.try_recv() {
            self.on_asr_event(event);
        }
        while let Ok(event) = self.vad_events.try_recv() {
            self.on_vad_event(event);
        }
        while let Ok(event) = self.llm_events.try_recv() {
            self.on_llm_event(event);
            self.drain_speech_output();
        }
    }

    pub fn send_control(
        &self,
        text: String,
    ) -> Result<(), mpsc::error::TrySendError<OutboundMessage>> {
        self.control_tx.try_send(OutboundMessage::Text(text))
    }

    pub async fn run(mut self, mut ingress: mpsc::Receiver<SessionEvent>) {
        let mut worker_tick = tokio::time::interval(std::time::Duration::from_millis(1));
        loop {
            tokio::select! {
                _ = worker_tick.tick() => {
                    self.drain_worker_events();
                    self.drain_speech_output();
                }
                event = ingress.recv() => match event {
                    Some(SessionEvent::ClientMessage(message)) => self.on_client_message(message),
                    Some(SessionEvent::ClientAudio(payload)) => { self.on_binary(payload); }
                    None => break,
                }
            }
        }
        self.phase = SessionPhase::Closed;
    }

    pub fn on_client_message(&mut self, message: ClientMessage) {
        match message {
            ClientMessage::Listen {
                session_id,
                command,
            } => {
                if self.inbound_session_matches(session_id.as_deref()) {
                    self.on_listen_command(command);
                }
            }
            ClientMessage::Abort { session_id } => {
                if self.inbound_session_matches(session_id.as_deref()) {
                    self.abort_current_turn();
                }
            }
            ClientMessage::Hello(_) | ClientMessage::Unknown => {}
        }
    }

    fn inbound_session_matches(&self, session_id: Option<&str>) -> bool {
        matches!(session_id, None | Some("")) || session_id == Some(&self.session_id)
    }

    fn on_listen_command(&mut self, command: ListenCommand) {
        match command {
            ListenCommand::Start { mode } => self.start_listening(mode),
            ListenCommand::Detect { text } => self.accept_detect(text),
            ListenCommand::Stop => {
                if self.listening_mode == Some(ListenMode::Manual)
                    && self.phase == SessionPhase::Listening
                {
                    let outcome = self.manual_capture.stop();
                    if let CaptureOutcome::Utterance(_) = outcome {
                        self.phase = SessionPhase::Processing;
                        self.finish_manual();
                    } else {
                        self.cancel_asr();
                        self.phase = SessionPhase::Ready;
                    }
                }
            }
        }
    }

    fn start_listening(&mut self, mode: ListenMode) {
        if mode == ListenMode::Auto
            && self.listening_mode == Some(ListenMode::Auto)
            && self.vad_session.is_some()
        {
            self.restart_existing_auto_cycle();
            return;
        }
        self.replace_listening_mode(mode);
    }

    /// Enters a new listening mode. Leaving Auto is a real VAD lifecycle boundary; the
    /// acknowledgement-driven Close path owns release of its worker capacity.
    fn replace_listening_mode(&mut self, mode: ListenMode) {
        self.cancel_speech_delivery();
        self.generation += 1;
        self.cancel_llm();
        self.cancel_asr();
        self.release_active_turn();
        self.close_vad();
        self.auto_speech_active = false;
        self.auto_reset_pending = false;
        self.auto_retention.reset();
        self.vad_segmenter.reset();
        self.listening_mode = Some(mode.clone());
        let identity =
            WorkerIdentity::new(self.session_id.clone(), self.generation, self.generation);
        match mode {
            ListenMode::Manual => match self.asr_runtime.open(identity.clone()) {
                Ok(lease) => {
                    self.manual_capture.restart();
                    self.asr_stream = Some((lease, identity));
                    self.phase = SessionPhase::Listening;
                }
                Err(error) => {
                    warn!(?error, "manual ASR start rejected");
                    self.phase = SessionPhase::Ready;
                }
            },
            ListenMode::Auto => match self.vad_runtime.open(identity.clone()) {
                Ok(lease) => {
                    self.vad_session = Some((lease, identity));
                    self.phase = SessionPhase::Listening;
                }
                Err(error) => {
                    warn!(?error, phase = ?self.phase, generation = self.generation, "auto VAD worker open failed");
                    self.phase = SessionPhase::Closed;
                    let _ = self.control_tx.try_send(OutboundMessage::Close(1013));
                }
            },
            ListenMode::Realtime => self.phase = SessionPhase::Ready,
        }
    }

    /// Restarts capture inside an Auto Listening cycle without replacing its pinned VAD worker.
    fn restart_existing_auto_cycle(&mut self) {
        let Some((lease, _)) = self.vad_session else {
            return;
        };
        if self.auto_reset_pending {
            info!(
                generation = self.generation,
                "duplicate auto listen:start accepted while VAD reset is pending"
            );
            return;
        }

        self.cancel_speech_delivery();
        self.generation += 1;
        self.cancel_llm();
        self.cancel_asr();
        self.release_active_turn();
        self.auto_speech_active = false;
        self.auto_retention.reset();
        self.vad_segmenter.reset();
        match self.vad_runtime.send(lease, VadCommand::Reset) {
            Ok(()) => {
                self.auto_reset_pending = true;
                self.phase = SessionPhase::Processing;
                info!(generation = self.generation, "reusing Auto VAD cycle");
            }
            Err(error) => {
                warn!(?error, "failed to reset existing Auto VAD cycle");
                self.fail_closed();
            }
        }
    }

    fn abort_current_turn(&mut self) {
        if self.phase == SessionPhase::Closed {
            return;
        }
        self.cancel_speech_delivery();
        self.generation += 1;
        self.cancel_llm();
        self.manual_capture.abort();
        self.cancel_asr();
        self.release_active_turn();
        if self.listening_mode == Some(ListenMode::Auto) && self.vad_session.is_some() {
            self.abort_auto_turn();
            return;
        }
        self.close_vad();
        self.listening_mode = None;
        self.auto_reset_pending = false;
        self.auto_retention.reset();
        self.phase = SessionPhase::Ready;
    }

    fn abort_auto_turn(&mut self) {
        self.auto_speech_active = false;
        self.auto_retention.reset();
        self.vad_segmenter.reset();
        if self.auto_reset_pending {
            return;
        }
        let Some((lease, _)) = self.vad_session else {
            self.phase = SessionPhase::Ready;
            return;
        };
        match self.vad_runtime.send(lease, VadCommand::Reset) {
            Ok(()) => {
                self.auto_reset_pending = true;
                self.phase = SessionPhase::Processing;
            }
            Err(error) => {
                warn!(?error, "Auto VAD reset failed after abort");
                self.fail_closed();
            }
        }
    }

    fn accept_detect(&mut self, input: String) {
        let auto_rearming = self.listening_mode == Some(ListenMode::Auto)
            && self.auto_reset_pending
            && self.phase == SessionPhase::Processing;
        if self.phase != SessionPhase::Listening && !auto_rearming {
            return;
        }
        let Some(text) = normalize_detect_text(input) else {
            return;
        };
        info!(mode = ?self.listening_mode, "typed detect accepted");
        self.manual_capture.abort();
        match self.listening_mode {
            Some(ListenMode::Manual) => {
                if !self.detach_asr_for_detect() {
                    return;
                }
            }
            // `digital-human` starts an Auto cycle, then sends typed text through detect.
            // Revoke ASR semantic ownership but retain the VAD lease. Drained will reset it
            // and return this client to Listening for its next typed or voice turn.
            Some(ListenMode::Auto) => {
                self.auto_speech_active = false;
                self.auto_retention.reset();
                if self.asr_stream.is_some() && !self.detach_asr_for_detect() {
                    return;
                }
            }
            Some(ListenMode::Realtime) | None => return,
        }
        if !self.active_turn_limiter.try_acquire() {
            self.phase = SessionPhase::Ready;
            return;
        }
        self.has_active_turn_permit = true;
        self.phase = SessionPhase::Processing;
        if let Some(text) = self.commit_user_text(text) {
            self.begin_speech_delivery(text);
        } else {
            self.release_active_turn();
            self.phase = SessionPhase::Ready;
        }
    }

    fn detach_asr_for_detect(&mut self) -> bool {
        let Some((lease, identity)) = self.asr_stream.take() else {
            self.fail_closed();
            return false;
        };
        if self.asr_runtime.send(lease, AsrCommand::Cancel).is_err() {
            self.fail_closed();
            return false;
        }
        self.asr_cleanup_pending.insert(identity);
        true
    }

    fn finish_manual(&mut self) {
        if !self.active_turn_limiter.try_acquire() {
            self.cancel_asr();
            self.complete_recognition();
            return;
        }
        self.has_active_turn_permit = true;
        let Some((lease, _)) = self.asr_stream else {
            self.release_active_turn();
            return;
        };
        if self.asr_runtime.send(lease, AsrCommand::Finish).is_err() {
            self.cancel_asr();
            self.release_active_turn();
            self.phase = SessionPhase::Ready;
        }
    }

    fn on_asr_event(&mut self, event: AsrWorkerEvent) {
        let identity = match &event {
            AsrWorkerEvent::Opened { identity }
            | AsrWorkerEvent::Final { identity, .. }
            | AsrWorkerEvent::Failed { identity }
            | AsrWorkerEvent::Cancelled { identity }
            | AsrWorkerEvent::FinalTimedOut { identity }
            | AsrWorkerEvent::CleanupTimedOut { identity } => identity,
        };
        if self.asr_cleanup_pending.contains(identity) {
            match event {
                AsrWorkerEvent::Final { .. }
                | AsrWorkerEvent::Failed { .. }
                | AsrWorkerEvent::Cancelled { .. } => {
                    self.asr_cleanup_pending.remove(identity);
                }
                AsrWorkerEvent::FinalTimedOut { .. } | AsrWorkerEvent::CleanupTimedOut { .. } => {
                    self.fail_closed();
                }
                AsrWorkerEvent::Opened { .. } => {}
            }
            return;
        }
        let current = self
            .asr_stream
            .as_ref()
            .is_some_and(|(_, current)| current == identity)
            && identity.generation() == self.generation;
        match event {
            AsrWorkerEvent::Final { text, .. } if current => {
                self.asr_stream = None;
                if let Some(final_text) = self.commit_user_text(text) {
                    self.begin_speech_delivery(final_text);
                } else {
                    self.complete_recognition();
                }
            }
            AsrWorkerEvent::Failed { .. } if current => {
                self.asr_stream = None;
                self.complete_recognition();
            }
            AsrWorkerEvent::FinalTimedOut { .. } | AsrWorkerEvent::CleanupTimedOut { .. }
                if current =>
            {
                self.fail_closed()
            }
            AsrWorkerEvent::Cancelled { .. } if current => self.asr_stream = None,
            _ => {}
        }
    }

    fn on_vad_event(&mut self, event: VadWorkerEvent) {
        let identity = match &event {
            VadWorkerEvent::Opened { identity }
            | VadWorkerEvent::Probability { identity, .. }
            | VadWorkerEvent::SpeechStart { identity, .. }
            | VadWorkerEvent::SpeechEnd { identity, .. }
            | VadWorkerEvent::ResetDone { identity }
            | VadWorkerEvent::Closed { identity }
            | VadWorkerEvent::Failed { identity }
            | VadWorkerEvent::ResetTimedOut { identity }
            | VadWorkerEvent::CleanupTimedOut { identity } => identity,
        };
        let current = self
            .vad_session
            .as_ref()
            .is_some_and(|(_, active)| active == identity);
        match event {
            VadWorkerEvent::Probability { probability, .. } if current => {
                match self.vad_segmenter.observe(probability) {
                    Ok(Some(VadBoundary::SpeechStart { start_sample })) => {
                        self.on_vad_event(VadWorkerEvent::SpeechStart {
                            identity: identity.clone(),
                            start_sample,
                        })
                    }
                    Ok(Some(VadBoundary::SpeechEnd { end_sample })) => {
                        self.on_vad_event(VadWorkerEvent::SpeechEnd {
                            identity: identity.clone(),
                            end_sample,
                        })
                    }
                    Ok(None) => {}
                    Err(_) => {
                        warn!("VAD stream integrity failure");
                        self.fail_closed();
                    }
                }
            }
            VadWorkerEvent::SpeechStart { start_sample, .. }
                if current && !self.auto_speech_active =>
            {
                self.auto_speech_active = true;
                let identity =
                    WorkerIdentity::new(self.session_id.clone(), self.generation, self.generation);
                match self.asr_runtime.open(identity.clone()) {
                    Ok(lease) => {
                        self.asr_stream = Some((lease, identity));
                        let feed_start = start_sample.saturating_sub(self.pre_roll_samples);
                        let Some(retained) = self.auto_retention.range(feed_start) else {
                            self.fail_closed();
                            return;
                        };
                        if self.push_asr(retained).is_err() {
                            self.cancel_asr();
                            self.asr_stream = None;
                        }
                    }
                    Err(_) => self.auto_retention.reset(),
                }
            }
            VadWorkerEvent::SpeechEnd { .. } if current && self.auto_speech_active => {
                self.auto_speech_active = false;
                self.phase = SessionPhase::Processing;
                if self.asr_stream.is_some() {
                    self.finish_manual();
                } else {
                    // ASR capacity/queue denial has no transcript, but the VAD cycle remains
                    // usable after its reset acknowledgement.
                    self.complete_recognition();
                }
            }
            VadWorkerEvent::ResetDone { .. } if current => {
                self.auto_reset_pending = false;
                self.auto_retention.reset();
                self.vad_segmenter.reset();
                if self.llm_operation.is_none() && !self.tts_started {
                    self.phase = SessionPhase::Listening;
                }
            }
            VadWorkerEvent::Closed { .. } if current => self.vad_session = None,
            VadWorkerEvent::Failed { .. }
            | VadWorkerEvent::ResetTimedOut { .. }
            | VadWorkerEvent::CleanupTimedOut { .. }
                if current =>
            {
                warn!("VAD worker failed or cleanup timed out");
                self.fail_closed()
            }
            _ => {}
        }
    }

    fn complete_recognition(&mut self) {
        self.release_active_turn();
        if self.listening_mode == Some(ListenMode::Auto) && self.vad_session.is_some() {
            if self.auto_reset_pending {
                return;
            }
            if let Some((lease, _)) = self.vad_session {
                if self.vad_runtime.send(lease, VadCommand::Reset).is_err() {
                    self.fail_closed();
                } else {
                    self.auto_reset_pending = true;
                }
            }
        } else {
            self.phase = SessionPhase::Ready;
        }
    }

    fn fail_closed(&mut self) {
        if self.phase == SessionPhase::Closed {
            return;
        }
        warn!(phase = ?self.phase, "voice session failed closed");
        self.cancel_speech_delivery();
        self.generation += 1;
        self.cancel_llm();
        self.cancel_asr();
        self.release_active_turn();
        self.close_vad();
        self.auto_reset_pending = false;
        self.asr_stream = None;
        self.phase = SessionPhase::Closed;
        let _ = self.control_tx.try_send(OutboundMessage::Close(1011));
    }

    fn commit_user_text(&mut self, final_text: String) -> Option<String> {
        let text = final_text.trim();
        if text.is_empty() {
            return None;
        }
        let text = text.to_owned();
        self.dialogue_history.commit_user(text.clone());
        let payload = serde_json::json!({
            "session_id": self.session_id,
            "type": "stt",
            "text": text,
        });
        let Ok(payload) = serde_json::to_string(&payload) else {
            return None;
        };
        let _ = self.send_control(payload);
        Some(text)
    }

    fn begin_speech_delivery(&mut self, user_text: String) {
        let identity =
            WorkerIdentity::new(self.session_id.clone(), self.generation, self.generation);
        self.generated_response.clear();
        if self.llm_runtime.start(identity.clone(), user_text).is_err() {
            warn!("LLM operation could not start");
            self.complete_recognition();
            return;
        }
        info!("LLM operation started");
        self.llm_operation = Some(identity);
    }

    fn on_llm_event(&mut self, event: LlmRuntimeEvent) {
        let identity = match &event {
            LlmRuntimeEvent::TextDelta { identity, .. }
            | LlmRuntimeEvent::UnexpectedToolCall { identity }
            | LlmRuntimeEvent::Finished { identity }
            | LlmRuntimeEvent::Failed { identity }
            | LlmRuntimeEvent::Cancelled { identity } => identity,
        };
        if self.llm_operation.as_ref() != Some(identity) || identity.generation() != self.generation
        {
            return;
        }
        match event {
            LlmRuntimeEvent::TextDelta { text, .. } => {
                self.generated_response.push_str(&text);
                if self.speech_output.push_delta(&text).is_err() {
                    self.fail_speech_delivery();
                }
            }
            LlmRuntimeEvent::Finished { .. } => {
                self.llm_operation = None;
                info!("LLM operation finished");
                if self.generated_response.trim().is_empty()
                    || self.speech_output.finish_input().is_err()
                {
                    self.fail_speech_delivery();
                }
            }
            LlmRuntimeEvent::UnexpectedToolCall { .. }
            | LlmRuntimeEvent::Failed { .. }
            | LlmRuntimeEvent::Cancelled { .. } => {
                self.llm_operation = None;
                warn!("LLM operation ended without a deliverable response");
                self.fail_speech_delivery();
            }
        }
    }

    fn drain_speech_output(&mut self) {
        if !self.flush_pending_audio() {
            return;
        }
        loop {
            let event = match self.speech_output.poll() {
                Ok(Some(event)) => event,
                Ok(None) => break,
                Err(_) => {
                    self.fail_speech_delivery();
                    break;
                }
            };
            match event {
                SpeechOutputEvent::SegmentReady { text } => {
                    let payload = serde_json::json!({
                        "session_id": self.session_id,
                        "type": "llm",
                        "text": &text,
                    });
                    if serde_json::to_string(&payload)
                        .ok()
                        .is_none_or(|payload| self.send_control(payload).is_err())
                    {
                        self.fail_speech_delivery();
                        break;
                    }
                }
                SpeechOutputEvent::Started => {
                    self.tts_started = true;
                    info!("TTS delivery started");
                    let payload = serde_json::json!({
                        "session_id": self.session_id,
                        "type": "tts",
                        "state": "start",
                    });
                    if serde_json::to_string(&payload)
                        .ok()
                        .is_none_or(|payload| self.send_control(payload).is_err())
                    {
                        self.fail_speech_delivery();
                        break;
                    }
                }
                SpeechOutputEvent::AudioPacket(packet) => {
                    let message = OutboundMessage::Binary {
                        generation: self.generation,
                        packet,
                    };
                    match self.audio_tx.try_send(message) {
                        Ok(()) => {}
                        Err(mpsc::error::TrySendError::Full(message)) => {
                            self.pending_audio = Some(message);
                            break;
                        }
                        // A closed queue means the connection writer has already gone away;
                        // session teardown owns the resulting close path.
                        Err(mpsc::error::TrySendError::Closed(_)) => break,
                    }
                }
                SpeechOutputEvent::Drained => {
                    info!("TTS delivery drained");
                    if self.tts_started {
                        let payload = serde_json::json!({
                            "session_id": self.session_id,
                            "type": "tts",
                            "state": "stop",
                        });
                        if let Ok(payload) = serde_json::to_string(&payload) {
                            let _ = self.send_control(payload);
                        }
                    }
                    // A Delivered Assistant Response exists only after all its audio drained.
                    self.dialogue_history
                        .commit_assistant(std::mem::take(&mut self.generated_response));
                    self.tts_started = false;
                    self.complete_recognition();
                }
            }
        }
    }

    fn fail_speech_delivery(&mut self) {
        self.cancel_llm();
        self.cancel_speech_delivery();
        self.complete_recognition();
    }

    fn cancel_speech_delivery(&mut self) {
        self.speech_output.cancel();
        self.pending_audio = None;
        if self.tts_started {
            // The writer receives this gate before a stop and drops queued packets for this turn.
            let _ = self
                .control_tx
                .try_send(OutboundMessage::InvalidateAudio(self.generation));
            let payload = serde_json::json!({
                "session_id": self.session_id,
                "type": "tts",
                "state": "stop",
            });
            if let Ok(payload) = serde_json::to_string(&payload) {
                let _ = self.send_control(payload);
            }
        }
        self.tts_started = false;
    }

    /// Returns false while the writer is backpressured. Keeping exactly one pending packet
    /// bounds memory and, together with SpeechOutput's own queue, preserves packet order.
    fn flush_pending_audio(&mut self) -> bool {
        let Some(message) = self.pending_audio.take() else {
            return true;
        };
        match self.audio_tx.try_send(message) {
            Ok(()) => true,
            Err(mpsc::error::TrySendError::Full(message)) => {
                self.pending_audio = Some(message);
                false
            }
            Err(mpsc::error::TrySendError::Closed(_)) => false,
        }
    }

    fn cancel_asr(&mut self) {
        if let Some((lease, _)) = self.asr_stream {
            let _ = self.asr_runtime.send(lease, AsrCommand::Cancel);
        }
    }

    fn cancel_llm(&mut self) {
        if let Some(identity) = self.llm_operation.take() {
            self.llm_runtime.cancel(&identity);
        }
        self.generated_response.clear();
    }

    fn release_active_turn(&mut self) {
        if self.has_active_turn_permit {
            self.active_turn_limiter.release();
            self.has_active_turn_permit = false;
        }
    }

    fn close_vad(&mut self) {
        if let Some((lease, _)) = self.vad_session {
            let _ = self.vad_runtime.send(lease, VadCommand::Close);
        }
    }

    fn push_asr(&mut self, pcm: PcmF32Mono) -> Result<(), ()> {
        let Some((lease, _)) = self.asr_stream else {
            return Err(());
        };
        self.asr_runtime
            .send(lease, AsrCommand::Push(pcm))
            .map_err(|_| ())
    }

    pub fn on_binary(&mut self, payload: Vec<u8>) -> bool {
        if self.phase == SessionPhase::Listening {
            match self.uplink_decoder.decode(&payload) {
                DecodeOutcome::Frame(frame) => {
                    let pcm = PcmF32Mono::from_uplink(&frame);
                    if self.listening_mode == Some(ListenMode::Auto) {
                        if self.auto_reset_pending {
                            return false;
                        }
                        let Some((lease, _)) = self.vad_session else {
                            return false;
                        };
                        self.auto_retention.push(&pcm);
                        match self.vad_runtime.send(lease, VadCommand::Push(pcm.clone())) {
                            Ok(()) => {}
                            Err(crate::workers::VadWorkerError::QueueFull) => {
                                self.fail_closed();
                                return false;
                            }
                            Err(_) => {
                                self.fail_closed();
                                return false;
                            }
                        }
                        if self.auto_speech_active && self.push_asr(pcm).is_err() {
                            self.cancel_asr();
                            // Recognition is terminally invalid once a Push was not admitted;
                            // wait for worker cleanup, but do not attempt Finish at SpeechEnd.
                            self.asr_stream = None;
                        }
                        self.accepted_binary_frames += 1;
                        return true;
                    }
                    if !self.manual_capture.push(frame.clone()) {
                        self.cancel_asr();
                        return false;
                    }
                    let Some((lease, _)) = self.asr_stream else {
                        self.phase = SessionPhase::Ready;
                        return false;
                    };
                    if self.asr_runtime.send(lease, AsrCommand::Push(pcm)).is_err() {
                        self.cancel_asr();
                        self.asr_stream = None;
                        self.manual_capture.abort();
                        self.phase = SessionPhase::Ready;
                        return false;
                    }
                    self.accepted_binary_frames += 1;
                    true
                }
                DecodeOutcome::Dropped(reason) => {
                    tracing::debug!(
                        event = "audio_frame_dropped",
                        ?reason,
                        "audio frame dropped"
                    );
                    false
                }
            }
        } else {
            false
        }
    }
}

/// Actor-owned bounded PCM storage for onset-relative Auto pre-roll.
struct AutoPcmRetention {
    samples: Vec<f32>,
    first_sample: u64,
    cursor: u64,
    capacity: usize,
}

impl AutoPcmRetention {
    fn new(capacity: usize) -> Self {
        Self {
            samples: Vec::with_capacity(capacity),
            first_sample: 0,
            cursor: 0,
            capacity,
        }
    }

    fn push(&mut self, pcm: &PcmF32Mono) {
        debug_assert_eq!(pcm.sample_rate_hz(), 16_000);
        self.samples.extend_from_slice(pcm.samples());
        self.cursor += pcm.samples().len() as u64;
        let overflow = self.samples.len().saturating_sub(self.capacity);
        if overflow > 0 {
            self.samples.drain(..overflow);
            self.first_sample += overflow as u64;
        }
    }

    fn range(&self, start_sample: u64) -> Option<PcmF32Mono> {
        if start_sample < self.first_sample || start_sample > self.cursor {
            return None;
        }
        let offset = (start_sample - self.first_sample) as usize;
        Some(PcmF32Mono::from_samples(self.samples[offset..].to_vec()))
    }

    fn reset(&mut self) {
        self.samples.clear();
        self.first_sample = 0;
        self.cursor = 0;
    }
}

fn auto_retention_capacity(
    vad_command_capacity: usize,
    confirmation_samples: u64,
    pre_roll_samples: u64,
) -> usize {
    const FRAME_SAMPLES: usize = 960;
    const RECHUNK_SLACK_SAMPLES: usize = 512;
    pre_roll_samples as usize
        + confirmation_samples as usize
        + vad_command_capacity.saturating_mul(FRAME_SAMPLES)
        + FRAME_SAMPLES
        + RECHUNK_SLACK_SAMPLES
}

impl Drop for SessionActor {
    fn drop(&mut self) {
        // The application-owned supervisor continues to observe the acknowledgement or timeout
        // after this actor and its WebSocket have gone away.
        self.cancel_asr();
        self.cancel_llm();
        self.speech_output.cancel();
        self.release_active_turn();
        self.close_vad();
        self.asr_runtime.unregister_session(&self.session_id);
        self.vad_runtime.unregister_session(&self.session_id);
        self.llm_runtime.unregister_session(&self.session_id);
    }
}

fn normalize_detect_text(input: String) -> Option<String> {
    let text = input.trim();
    if text.is_empty()
        || text.chars().take(MAX_DETECT_TEXT_SCALARS + 1).count() > MAX_DETECT_TEXT_SCALARS
    {
        return None;
    }
    Some(text.to_owned())
}
