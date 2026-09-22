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
    #[serde(default = "default_v1_version")]
    pub version: u8,
    #[serde(default = "default_websocket_transport")]
    pub transport: String,
    #[serde(default = "default_uplink_audio_params")]
    pub audio_params: AudioParams,
}

fn default_v1_version() -> u8 {
    1
}

fn default_websocket_transport() -> String {
    "websocket".to_owned()
}

fn default_uplink_audio_params() -> AudioParams {
    AudioParams {
        format: "opus".to_owned(),
        sample_rate: 16_000,
        channels: 1,
        frame_duration: 60,
    }
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
    Listen {
        session_id: Option<String>,
        command: ListenCommand,
    },
    Abort {
        session_id: Option<String>,
    },
    Unknown,
}

impl ClientMessage {
    /// Builds an unscoped V1 command for actor-level tests and compatibility callers.
    pub fn listen(command: ListenCommand) -> Self {
        Self::Listen {
            session_id: None,
            command,
        }
    }
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
        Some("abort") => parse_session_id(&value)
            .map_or(Ok(ClientMessage::Unknown), |session_id| {
                Ok(ClientMessage::Abort { session_id })
            }),
        _ => Ok(ClientMessage::Unknown),
    }
}

fn parse_listen_command(value: &Value) -> Result<ClientMessage, ProtocolError> {
    let Some(session_id) = parse_session_id(value) else {
        return Ok(ClientMessage::Unknown);
    };
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
    Ok(ClientMessage::Listen {
        session_id,
        command,
    })
}

fn parse_session_id(value: &Value) -> Option<Option<String>> {
    match value.get("session_id") {
        None => Some(None),
        Some(Value::String(session_id)) => Some(Some(session_id.clone())),
        Some(_) => None,
    }
}
