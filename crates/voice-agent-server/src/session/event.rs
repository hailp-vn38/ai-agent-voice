use crate::protocol::ClientMessage;

/// Reader tasks enqueue events; only SessionActor mutates Voice Session state.
#[derive(Debug)]
pub enum SessionEvent {
    ClientMessage(ClientMessage),
    ClientAudio(Vec<u8>),
}
