use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct AudioParams {
    pub format: String,
    pub sample_rate: u32,
    pub channels: u8,
    pub frame_duration: u16,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct ClientHello {
    #[serde(rename = "type")]
    pub message_type: String,
    pub version: u8,
    pub transport: String,
    pub audio_params: AudioParams,
}

impl ClientHello {
    pub fn validate_v1(&self) -> Result<(), ProtocolError> {
        if self.message_type != "hello" || self.version != 1 || self.transport != "websocket" {
            return Err(ProtocolError::InvalidHello);
        }
        let audio = &self.audio_params;
        if audio.format != "opus"
            || audio.sample_rate != 16_000
            || audio.channels != 1
            || audio.frame_duration != 60
        {
            return Err(ProtocolError::InvalidAudioProfile);
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ListenMode {
    Manual,
    Auto,
    Realtime,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ListenCommand {
    Start { mode: ListenMode },
    Stop,
    Detect { text: String },
}

#[derive(Clone, Debug, PartialEq)]
pub enum ClientMessage {
    Hello(ClientHello),
    Listen(ListenCommand),
    Abort,
    Unknown,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ProtocolError {
    #[error("malformed JSON")]
    MalformedJson,
    #[error("invalid hello")]
    InvalidHello,
    #[error("invalid canonical audio profile")]
    InvalidAudioProfile,
}

pub fn parse_client_message(text: &str) -> Result<ClientMessage, ProtocolError> {
    let value: Value = serde_json::from_str(text).map_err(|_| ProtocolError::MalformedJson)?;
    match value.get("type").and_then(Value::as_str) {
        Some("hello") => serde_json::from_value(value)
            .map(ClientMessage::Hello)
            .map_err(|_| ProtocolError::InvalidHello),
        Some("listen") => parse_listen_command(&value),
        Some("abort") => Ok(ClientMessage::Abort),
        _ => Ok(ClientMessage::Unknown),
    }
}

fn parse_listen_command(value: &Value) -> Result<ClientMessage, ProtocolError> {
    let command = match value.get("state").and_then(Value::as_str) {
        Some("start") => match value.get("mode").and_then(Value::as_str) {
            Some("manual") => ListenCommand::Start {
                mode: ListenMode::Manual,
            },
            Some("auto") => ListenCommand::Start {
                mode: ListenMode::Auto,
            },
            Some("realtime") => ListenCommand::Start {
                mode: ListenMode::Realtime,
            },
            _ => return Ok(ClientMessage::Unknown),
        },
        Some("stop") => ListenCommand::Stop,
        Some("detect") => match value.get("text").and_then(Value::as_str) {
            Some(text) => ListenCommand::Detect {
                text: text.to_owned(),
            },
            None => return Ok(ClientMessage::Unknown),
        },
        _ => return Ok(ClientMessage::Unknown),
    };
    Ok(ClientMessage::Listen(command))
}
