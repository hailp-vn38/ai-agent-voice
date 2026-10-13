use crate::protocol::ClientMessage;

/// Reader tasks enqueue events; only SessionActor mutates Voice Session state.
#[derive(Debug)]
pub enum SessionEvent {
    JobSubmitCompleted {
        session_id: String,
        turn_id: super::TurnId,
        generation: u64,
        tool_call_id: String,
        outcome: serde_json::Value,
    },
    ClientMessage(ClientMessage),
    DirectSpeech {
        request: super::DirectSpeechRequest,
        reply: tokio::sync::oneshot::Sender<Result<(), super::DirectSpeechError>>,
    },
    ReminderControl {
        text: String,
    },
    ClientAudio(Vec<u8>),
    /// Application lifecycle requests a controlled WebSocket close before the drain deadline.
    Shutdown,
    SecurityInvalidated,
}
