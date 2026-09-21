use crate::protocol::{ClientMessage, ListenState};
use crate::session::SessionPhase;
use tokio::sync::mpsc;

/// The only mutable owner of an accepted Voice Session's phase.
pub struct SessionActor {
    session_id: String,
    phase: SessionPhase,
    accepted_binary_frames: u64,
    control_tx: mpsc::Sender<OutboundMessage>,
    audio_tx: mpsc::Sender<OutboundMessage>,
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
    ) -> Self {
        Self {
            session_id,
            phase: SessionPhase::Ready,
            accepted_binary_frames: 0,
            control_tx,
            audio_tx,
        }
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

    pub async fn send_control(&self, text: String) {
        let _ = self.control_tx.send(OutboundMessage::Text(text)).await;
    }
    pub async fn close(&self, code: u16) {
        let _ = self.control_tx.send(OutboundMessage::Close(code)).await;
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
            ClientMessage::Listen(ListenState::Start) => self.phase = SessionPhase::Listening,
            ClientMessage::Listen(ListenState::Stop) => {
                if self.phase == SessionPhase::Listening {
                    self.phase = SessionPhase::Ready;
                }
            }
            ClientMessage::Abort => {
                if self.phase == SessionPhase::Listening {
                    self.phase = SessionPhase::Ready;
                }
            }
            ClientMessage::Hello(_) | ClientMessage::Unknown => {}
        }
    }

    /// V1 forwards raw bytes; Opus decode starts in Phase 2.
    pub fn on_binary(&mut self, _payload: Vec<u8>) -> bool {
        if self.phase == SessionPhase::Listening {
            self.accepted_binary_frames += 1;
            true
        } else {
            false
        }
    }

    #[allow(dead_code)]
    pub async fn send_audio(&self, payload: Vec<u8>) {
        let _ = self.audio_tx.send(OutboundMessage::Binary(payload)).await;
    }
}
