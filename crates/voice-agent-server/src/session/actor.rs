use crate::session::{event::SessionEvent, turn::DialogueHistory, ActiveTurnLimiter, SessionPhase};
use crate::{
    audio::{CaptureOutcome, DecodeOutcome, ManualCapture, PcmF32Mono, UplinkOpusDecoder},
    protocol::{ClientMessage, ListenCommand, ListenMode},
    providers::ProviderSet,
    workers::{
        AsrCommand, AsrStreamLease, AsrWorkerEvent, AsrWorkerRuntime, VadCommand, VadWorkerEvent,
        VadWorkerLease, VadWorkerRuntime, WorkerIdentity, WorkerRuntimeConfig,
    },
};
use tokio::sync::mpsc;

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
    vad_runtime: std::sync::Arc<VadWorkerRuntime>,
    vad_events: mpsc::Receiver<VadWorkerEvent>,
    vad_session: Option<(VadWorkerLease, WorkerIdentity)>,
    listening_mode: Option<ListenMode>,
    auto_speech_active: bool,
    auto_preroll: Vec<PcmF32Mono>,
    max_capture_frames: usize,
    active_turn_limiter: std::sync::Arc<ActiveTurnLimiter>,
    has_active_turn_permit: bool,
    generation: u64,
    dialogue_history: DialogueHistory,
    control_tx: mpsc::Sender<OutboundMessage>,
    _audio_tx: mpsc::Sender<OutboundMessage>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OutboundMessage {
    Text(String),
    Binary(Vec<u8>),
    Close(u16),
}

impl OutboundMessage {
    pub fn as_text(&self) -> Option<&str> {
        match self {
            Self::Text(text) => Some(text),
            Self::Binary(_) | Self::Close(_) => None,
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
            asr_runtime,
            vad_runtime,
            std::sync::Arc::new(ActiveTurnLimiter::new(8)),
        )
    }

    pub fn new_with_runtimes_and_limiter(
        session_id: String,
        control_tx: mpsc::Sender<OutboundMessage>,
        audio_tx: mpsc::Sender<OutboundMessage>,
        max_capture_frames: usize,
        max_history_messages: usize,
        asr_runtime: std::sync::Arc<AsrWorkerRuntime>,
        vad_runtime: std::sync::Arc<VadWorkerRuntime>,
        active_turn_limiter: std::sync::Arc<ActiveTurnLimiter>,
    ) -> Result<Self, crate::audio::AudioError> {
        let asr_events = asr_runtime.register_session(&session_id);
        let vad_events = vad_runtime.register_session(&session_id);
        Ok(Self {
            session_id,
            phase: SessionPhase::Ready,
            accepted_binary_frames: 0,
            uplink_decoder: UplinkOpusDecoder::new()?,
            manual_capture: ManualCapture::new(max_capture_frames)?,
            asr_runtime,
            asr_events,
            asr_stream: None,
            vad_runtime,
            vad_events,
            vad_session: None,
            listening_mode: None,
            auto_speech_active: false,
            auto_preroll: Vec::with_capacity(max_capture_frames),
            max_capture_frames,
            active_turn_limiter,
            has_active_turn_permit: false,
            generation: 0,
            dialogue_history: DialogueHistory::new(max_history_messages),
            control_tx,
            _audio_tx: audio_tx,
        })
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
    }

    fn drain_worker_events(&mut self) {
        while let Ok(event) = self.asr_events.try_recv() {
            self.on_asr_event(event);
        }
        while let Ok(event) = self.vad_events.try_recv() {
            self.on_vad_event(event);
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
                _ = worker_tick.tick() => self.drain_worker_events(),
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
            ClientMessage::Listen(ListenCommand::Start { mode }) => {
                self.generation += 1;
                self.cancel_asr();
                self.release_active_turn();
                self.close_vad();
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
                        Err(_) => self.phase = SessionPhase::Ready,
                    },
                    ListenMode::Auto => match self.vad_runtime.open(identity.clone()) {
                        Ok(lease) => {
                            self.vad_session = Some((lease, identity));
                            self.auto_speech_active = false;
                            self.auto_preroll.clear();
                            self.phase = SessionPhase::Listening;
                        }
                        Err(_) => self.phase = SessionPhase::Ready,
                    },
                    ListenMode::Realtime => self.phase = SessionPhase::Ready,
                }
            }
            ClientMessage::Listen(ListenCommand::Detect { .. }) => {}
            ClientMessage::Listen(ListenCommand::Stop) => {
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
            ClientMessage::Abort => {
                if self.phase != SessionPhase::Closed {
                    self.generation += 1;
                    self.manual_capture.abort();
                    self.cancel_asr();
                    self.release_active_turn();
                    self.close_vad();
                    self.listening_mode = None;
                    self.auto_preroll.clear();
                    self.phase = SessionPhase::Ready;
                }
            }
            ClientMessage::Hello(_) | ClientMessage::Unknown => {}
        }
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
        let current = self
            .asr_stream
            .as_ref()
            .is_some_and(|(_, current)| current == identity)
            && identity.generation() == self.generation;
        match event {
            AsrWorkerEvent::Final { text, .. } if current => {
                self.asr_stream = None;
                self.commit_final(text);
                self.complete_recognition();
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
            | VadWorkerEvent::SpeechStart { identity }
            | VadWorkerEvent::SpeechEnd { identity }
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
            VadWorkerEvent::SpeechStart { .. } if current && !self.auto_speech_active => {
                self.auto_speech_active = true;
                let identity =
                    WorkerIdentity::new(self.session_id.clone(), self.generation, self.generation);
                match self.asr_runtime.open(identity.clone()) {
                    Ok(lease) => {
                        self.asr_stream = Some((lease, identity));
                        let preroll = std::mem::take(&mut self.auto_preroll);
                        for frame in preroll {
                            if self.push_asr(frame).is_err() {
                                break;
                            }
                        }
                    }
                    Err(_) => self.auto_preroll.clear(),
                }
            }
            VadWorkerEvent::SpeechEnd { .. } if current && self.auto_speech_active => {
                self.auto_speech_active = false;
                self.auto_preroll.clear();
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
                self.auto_preroll.clear();
                self.phase = SessionPhase::Listening;
            }
            VadWorkerEvent::Closed { .. } if current => self.vad_session = None,
            VadWorkerEvent::Failed { .. }
            | VadWorkerEvent::ResetTimedOut { .. }
            | VadWorkerEvent::CleanupTimedOut { .. }
                if current =>
            {
                self.fail_closed()
            }
            _ => {}
        }
    }

    fn complete_recognition(&mut self) {
        self.release_active_turn();
        if self.listening_mode == Some(ListenMode::Auto) && self.vad_session.is_some() {
            if let Some((lease, _)) = self.vad_session {
                if self.vad_runtime.send(lease, VadCommand::Reset).is_err() {
                    self.fail_closed();
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
        self.generation += 1;
        self.cancel_asr();
        self.release_active_turn();
        self.close_vad();
        self.asr_stream = None;
        self.phase = SessionPhase::Closed;
        let _ = self.control_tx.try_send(OutboundMessage::Close(1011));
    }

    fn commit_final(&mut self, final_text: String) {
        let text = final_text.trim();
        if text.is_empty() {
            return;
        }
        let text = text.to_owned();
        self.dialogue_history.commit_user(text.clone());
        let payload = serde_json::json!({
            "session_id": self.session_id,
            "type": "stt",
            "text": text,
        });
        let Ok(payload) = serde_json::to_string(&payload) else {
            return;
        };
        let _ = self.send_control(payload);
    }

    fn cancel_asr(&mut self) {
        if let Some((lease, _)) = self.asr_stream {
            let _ = self.asr_runtime.send(lease, AsrCommand::Cancel);
        }
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
                        let Some((lease, _)) = self.vad_session else {
                            return false;
                        };
                        if !self.auto_speech_active
                            && self.auto_preroll.len() < self.max_capture_frames
                        {
                            self.auto_preroll.push(pcm.clone());
                        }
                        match self.vad_runtime.send(lease, VadCommand::Push(pcm.clone())) {
                            Ok(()) => {}
                            Err(crate::workers::VadWorkerError::QueueFull) => {
                                tracing::debug!(event = "vad_input_dropped", session_id = %self.session_id, "VAD input queue is full");
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
                        session_id = %self.session_id,
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

impl Drop for SessionActor {
    fn drop(&mut self) {
        // The application-owned supervisor continues to observe the acknowledgement or timeout
        // after this actor and its WebSocket have gone away.
        self.cancel_asr();
        self.release_active_turn();
        self.close_vad();
        self.asr_runtime.unregister_session(&self.session_id);
        self.vad_runtime.unregister_session(&self.session_id);
    }
}
