use crate::session::SessionPhase;
use crate::{
    audio::{CaptureOutcome, DecodeOutcome, ManualCapture, UplinkOpusDecoder},
    protocol::{ClientMessage, ListenCommand, ListenMode},
};
use tokio::sync::mpsc;

/// The only mutable owner of an accepted Voice Session's phase.
pub struct SessionActor {
    session_id: String,
    phase: SessionPhase,
    accepted_binary_frames: u64,
    uplink_decoder: UplinkOpusDecoder,
    manual_capture: ManualCapture,
    _control_tx: mpsc::Sender<OutboundMessage>,
    _audio_tx: mpsc::Sender<OutboundMessage>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OutboundMessage {
    Text(String),
    Binary(Vec<u8>),
    Close(u16),
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
    ) -> Result<Self, crate::audio::AudioError> {
        Ok(Self {
            session_id,
            phase: SessionPhase::Ready,
            accepted_binary_frames: 0,
            uplink_decoder: UplinkOpusDecoder::new()?,
            manual_capture: ManualCapture::new(max_capture_frames)?,
            _control_tx: control_tx,
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

    pub fn send_control(
        &self,
        text: String,
    ) -> Result<(), mpsc::error::TrySendError<OutboundMessage>> {
        self._control_tx.try_send(OutboundMessage::Text(text))
    }

    pub async fn run(mut self, mut ingress: mpsc::Receiver<SessionEvent>) {
        while let Some(event) = ingress.recv().await {
            match event {
                SessionEvent::ClientMessage(message) => self.on_client_message(message),
                SessionEvent::ClientAudio(payload) => {
                    self.on_binary(payload);
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
                self.manual_capture.restart();
                self.phase = SessionPhase::Listening;
            }
            ClientMessage::Listen(ListenCommand::Start { .. })
            | ClientMessage::Listen(ListenCommand::Detect { .. }) => {}
            ClientMessage::Listen(ListenCommand::Stop) => {
                if self.phase == SessionPhase::Listening {
                    let outcome = self.manual_capture.stop();
                    if let CaptureOutcome::Utterance(utterance) = outcome {
                        tracing::debug!(
                            event = "manual_capture_completed",
                            session_id = %self.session_id,
                            frames = utterance.samples().len() / 960,
                            "manual capture completed"
                        );
                        self.manual_capture.recycle(utterance);
                    }
                    self.phase = SessionPhase::Ready;
                }
            }
            ClientMessage::Abort => {
                if self.phase == SessionPhase::Listening {
                    self.manual_capture.abort();
                    self.phase = SessionPhase::Ready;
                }
            }
            ClientMessage::Hello(_) | ClientMessage::Unknown => {}
        }
    }

    pub fn on_binary(&mut self, payload: Vec<u8>) -> bool {
        if self.phase == SessionPhase::Listening {
            match self.uplink_decoder.decode(&payload) {
                DecodeOutcome::Frame(frame) => {
                    self.manual_capture.push(frame);
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
