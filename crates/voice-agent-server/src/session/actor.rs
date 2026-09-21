use crate::session::SessionPhase;
use crate::{
    audio::{CaptureOutcome, DecodeOutcome, ManualCapture, PcmF32Mono, UplinkOpusDecoder},
    protocol::{ClientMessage, ListenCommand, ListenMode},
    providers::ProviderSet,
    workers::{
        AsrCommand, AsrStreamLease, AsrWorkerEvent, AsrWorkerRuntime, WorkerIdentity,
        WorkerRuntimeConfig,
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
    asr_stream: Option<(AsrStreamLease, WorkerIdentity)>,
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

#[derive(Debug)]
struct DialogueHistory {
    messages: Vec<String>,
    max_messages: usize,
}

impl DialogueHistory {
    fn new(max_messages: usize) -> Self {
        Self {
            messages: Vec::new(),
            max_messages,
        }
    }

    fn commit_user(&mut self, text: String) {
        if self.messages.len() == self.max_messages {
            self.messages.remove(0);
        }
        self.messages.push(text);
    }
}

/// Reader tasks only enqueue these events; they never mutate Voice Session state.
#[derive(Debug)]
pub enum SessionEvent {
    ClientMessage(ClientMessage),
    ClientAudio(Vec<u8>),
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
        Self::new_with_history_and_runtime(
            session_id,
            control_tx,
            audio_tx,
            max_capture_frames,
            max_history_messages,
            asr_runtime,
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
        Ok(Self {
            session_id,
            phase: SessionPhase::Ready,
            accepted_binary_frames: 0,
            uplink_decoder: UplinkOpusDecoder::new()?,
            manual_capture: ManualCapture::new(max_capture_frames)?,
            asr_runtime,
            asr_stream: None,
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
        &self.dialogue_history.messages
    }

    /// Applies queued worker events without ever invoking a mutable provider from the actor.
    pub fn pump_workers(&mut self) {
        while let Some(event) = self.asr_runtime.try_recv() {
            self.on_asr_event(event);
        }
        if let Some(event) = self.asr_runtime.reap_timeouts() {
            self.on_asr_event(event);
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
                _ = worker_tick.tick() => self.pump_workers(),
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
            ClientMessage::Listen(ListenCommand::Start {
                mode: ListenMode::Manual,
            }) => {
                self.cancel_asr();
                self.manual_capture.restart();
                self.generation += 1;
                let identity =
                    WorkerIdentity::new(self.session_id.clone(), self.generation, self.generation);
                match self.asr_runtime.open(identity.clone()) {
                    Ok(lease) => {
                        self.asr_stream = Some((lease, identity));
                        self.phase = SessionPhase::Listening;
                    }
                    Err(_) => self.phase = SessionPhase::Ready,
                }
            }
            ClientMessage::Listen(ListenCommand::Start { .. })
            | ClientMessage::Listen(ListenCommand::Detect { .. }) => {}
            ClientMessage::Listen(ListenCommand::Stop) => {
                if self.phase == SessionPhase::Listening {
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
                if self.phase == SessionPhase::Listening {
                    self.manual_capture.abort();
                    self.cancel_asr();
                    self.phase = SessionPhase::Ready;
                }
            }
            ClientMessage::Hello(_) | ClientMessage::Unknown => {}
        }
    }

    fn finish_manual(&mut self) {
        let Some((lease, _)) = self.asr_stream else {
            return;
        };
        if self.asr_runtime.send(lease, AsrCommand::Finish).is_err() {
            self.cancel_asr();
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
                self.phase = SessionPhase::Ready;
            }
            AsrWorkerEvent::Failed { .. }
            | AsrWorkerEvent::FinalTimedOut { .. }
            | AsrWorkerEvent::CleanupTimedOut { .. }
                if current =>
            {
                self.asr_stream = None;
                self.phase = SessionPhase::Ready;
            }
            AsrWorkerEvent::Cancelled { .. } if current => self.asr_stream = None,
            _ => {}
        }
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

    pub fn on_binary(&mut self, payload: Vec<u8>) -> bool {
        if self.phase == SessionPhase::Listening {
            match self.uplink_decoder.decode(&payload) {
                DecodeOutcome::Frame(frame) => {
                    let pcm = PcmF32Mono::from_uplink(&frame);
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
