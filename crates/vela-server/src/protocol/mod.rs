mod client;
mod server;

pub use client::{
    AudioParams, ClientFeatures, ClientHello, ClientMessage, ListenCommand, ListenMode,
    ProtocolError, ReminderEvent, parse_client_message,
};
pub use server::{Firmware, OtaActivation, OtaResponse, OtaWebsocket, ServerHello, ServerTime};
