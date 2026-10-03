use crate::protocol::ClientMessage;

/// Reader tasks enqueue events; only SessionActor mutates Voice Session state.
#[derive(Debug)]
pub enum SessionEvent {
    ClientMessage(ClientMessage),
    ClientAudio(Vec<u8>),
    /// Application lifecycle requests a controlled WebSocket close before the drain deadline.
    Shutdown,
}
