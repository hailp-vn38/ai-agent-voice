use crate::tools::device_mcp::{McpIncoming, parse_incoming};
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
    #[serde(default)]
    pub features: ClientFeatures,
}

/// Optional client capability assertions. Unknown feature keys remain compatible
/// because this type intentionally does not deny unknown fields.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct ClientFeatures {
    #[serde(default)]
    pub reminders: bool,
    #[serde(default)]
    pub direct_tts: bool,
    #[serde(default)]
    pub pipeline_status: bool,
    #[serde(default)]
    pub turn_status: bool,
    #[serde(default)]
    pub tool_status: bool,
    /// Ticket 10: opt-in to bounded `speaker` Observe state frames. Off by default.
    #[serde(default)]
    pub speaker_status: bool,
    #[serde(default)]
    pub aec: bool,
    #[serde(default)]
    pub mcp: bool,
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
    Mcp {
        session_id: Option<String>,
        payload: McpIncoming,
    },
    Reminder {
        session_id: String,
        event: ReminderEvent,
    },
    Unknown,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ReminderEvent {
    SyncRequest {
        known_revision: Option<i64>,
    },
    SyncAck {
        snapshot_id: String,
        revision: i64,
        next_wake_at_ms: Option<i64>,
    },
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
        Some("mcp") => parse_mcp_message(&value),
        Some("reminder") => parse_reminder_message(&value),
        _ => Ok(ClientMessage::Unknown),
    }
}

fn parse_reminder_message(value: &Value) -> Result<ClientMessage, ProtocolError> {
    let Some(Some(session_id)) = parse_session_id(value) else {
        return Ok(ClientMessage::Unknown);
    };
    let event = match value.get("event").and_then(Value::as_str) {
        Some("sync_request") => ReminderEvent::SyncRequest {
            known_revision: value.get("known_revision").and_then(Value::as_i64),
        },
        Some("sync_ack") => {
            let Some(snapshot_id) = value
                .get("snapshot_id")
                .and_then(Value::as_str)
                .filter(|v| !v.is_empty() && v.len() <= 128)
            else {
                return Ok(ClientMessage::Unknown);
            };
            let Some(revision) = value
                .get("revision")
                .and_then(Value::as_i64)
                .filter(|v| *v >= 0)
            else {
                return Ok(ClientMessage::Unknown);
            };
            let next_wake_at_ms = match value.get("next_wake_at_ms") {
                None => None,
                Some(v) => match v.as_i64() {
                    Some(v) => Some(v),
                    None => return Ok(ClientMessage::Unknown),
                },
            };
            ReminderEvent::SyncAck {
                snapshot_id: snapshot_id.into(),
                revision,
                next_wake_at_ms,
            }
        }
        _ => return Ok(ClientMessage::Unknown),
    };
    Ok(ClientMessage::Reminder { session_id, event })
}

fn parse_mcp_message(value: &Value) -> Result<ClientMessage, ProtocolError> {
    let Some(session_id) = parse_session_id(value) else {
        return Ok(ClientMessage::Unknown);
    };
    let Some(payload) = value.get("payload").cloned().and_then(parse_incoming) else {
        return Ok(ClientMessage::Unknown);
    };
    Ok(ClientMessage::Mcp {
        session_id,
        payload,
    })
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

#[cfg(test)]
mod tool_status_feature_tests {
    use super::*;

    #[test]
    fn tool_status_is_explicitly_opted_into_by_voice_clients() {
        let base = serde_json::json!({"type":"hello","version":1,"transport":"websocket"});
        let without = parse_client_message(&base.to_string()).expect("default hello parses");
        let ClientMessage::Hello(without) = without else {
            panic!("hello")
        };
        assert!(!without.features.tool_status);

        let mut enabled = base;
        enabled["features"] = serde_json::json!({"tool_status":true,"future_feature":true});
        let parsed = parse_client_message(&enabled.to_string()).expect("feature hello parses");
        let ClientMessage::Hello(parsed) = parsed else {
            panic!("hello")
        };
        assert!(parsed.features.tool_status);
    }

    #[test]
    fn reminder_controls_require_a_current_session_and_typed_ack_fields() {
        assert!(matches!(
            parse_client_message(r#"{"type":"reminder","event":"sync_request","session_id":"s"}"#),
            Ok(ClientMessage::Reminder { session_id, event: ReminderEvent::SyncRequest { .. } }) if session_id == "s"
        ));
        assert!(matches!(
            parse_client_message(
                r#"{"type":"reminder","event":"sync_ack","session_id":"s","snapshot_id":"snap","revision":2,"next_wake_at_ms":3}"#
            ),
            Ok(ClientMessage::Reminder {
                event: ReminderEvent::SyncAck { revision: 2, .. },
                ..
            })
        ));
        assert!(matches!(
            parse_client_message(
                r#"{"type":"reminder","event":"sync_ack","snapshot_id":"snap","revision":2}"#
            ),
            Ok(ClientMessage::Unknown)
        ));
    }
}
