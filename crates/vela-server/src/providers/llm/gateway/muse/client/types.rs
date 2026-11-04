use serde_json::Value;

use super::super::pool::UpstreamFailureKind;
use super::super::workflow::WorkflowFailure;

pub(in crate::providers::llm::gateway::muse) const ISOLATION_POLICY_MARKDOWN: &str = "\
# Session Isolation Policy (MANDATORY)\n\n\
This environment serves multiple independent API users concurrently. Each chat session belongs to a separate user.\n\n\
## Rules\n\
1. **Never reference other sessions**: Do not read, mention, or use information from any session other than the current one.\n\
2. **Ignore shared workspace artifacts from other tasks**: Only access files explicitly created or referenced within the current conversation turn.\n\
3. **Treat each session as a clean slate**: Only use the conversation history within the current session.\n";

#[derive(Debug, Clone)]
pub(crate) enum VmError {
    AuthInvalid(String),
    QuotaExceeded(String),
    Timeout(String),
    Transport(String),
}

impl std::fmt::Display for VmError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            VmError::AuthInvalid(msg) => write!(f, "vm auth invalid: {msg}"),
            VmError::QuotaExceeded(msg) => write!(f, "vm quota exceeded: {msg}"),
            VmError::Timeout(msg) => write!(f, "vm timeout: {msg}"),
            VmError::Transport(msg) => write!(f, "vm transport error: {msg}"),
        }
    }
}

impl From<VmError> for WorkflowFailure {
    fn from(err: VmError) -> Self {
        match err {
            VmError::AuthInvalid(msg) => {
                WorkflowFailure::Upstream(UpstreamFailureKind::AuthInvalid, msg)
            }
            VmError::QuotaExceeded(msg) => {
                WorkflowFailure::Upstream(UpstreamFailureKind::QuotaExceeded, msg)
            }
            VmError::Timeout(msg) => WorkflowFailure::Timeout(msg),
            VmError::Transport(msg) => {
                WorkflowFailure::Upstream(UpstreamFailureKind::Transient, msg)
            }
        }
    }
}

/// Real-time event emitted on the `/chat/subscribe` stream with schema-aware accessors.
#[derive(Debug, Clone)]
pub(crate) struct VmStreamEvent {
    pub raw: Value,
}

impl VmStreamEvent {
    pub fn event_name(&self) -> &str {
        self.raw
            .get("event")
            .or_else(|| self.raw.get("event_name"))
            .or_else(|| self.raw.get("type"))
            .and_then(Value::as_str)
            .unwrap_or("")
    }

    pub fn payload(&self) -> &Value {
        self.raw.get("payload").unwrap_or(&self.raw)
    }

    pub fn session_id(&self) -> Option<&str> {
        let p = self.payload();
        p.get("session_id")
            .or_else(|| self.raw.get("session_id"))
            .or_else(|| {
                p.get("chat_context")
                    .and_then(|c| c.get("chat_id").or_else(|| c.get("session_id")))
            })
            .or_else(|| {
                self.raw
                    .get("chat_context")
                    .and_then(|c| c.get("chat_id").or_else(|| c.get("session_id")))
            })
            .or_else(|| p.get("chat_id"))
            .or_else(|| self.raw.get("chat_id"))
            .or_else(|| {
                p.get("session")
                    .and_then(|s| s.get("session_id").or_else(|| s.get("id")))
            })
            .and_then(Value::as_str)
    }

    pub fn activity_code(&self) -> Option<&str> {
        if self.event_name() != "agent.status" {
            return None;
        }
        self.payload().get("activity_code").and_then(Value::as_str)
    }

    /// Inspects `agent.status` (`payload.activity_code`) per `src/core/client.js:1264-1276`.
    pub fn agent_activity_state(&self) -> Option<bool> {
        match self.activity_code()? {
            "working" | "making_something" | "responding" | "thinking" | "running" => Some(true),
            "online" | "idle" => Some(false),
            _ => None,
        }
    }

    pub fn session_update_status(&self, target_session_id: &str) -> Option<&str> {
        if self.event_name() != "sessions.updated" {
            return None;
        }
        let p = self.payload();
        let change = p.get("change").and_then(Value::as_str);
        let sess = p.get("session")?;
        let id = sess
            .get("session_id")
            .or_else(|| sess.get("id"))
            .and_then(Value::as_str)
            .or_else(|| self.session_id())?;
        if id != target_session_id {
            return None;
        }
        let status = sess.get("status").and_then(Value::as_str)?;
        if status == "completed" && change == Some("created") {
            return None;
        }
        Some(status)
    }

    /// Inspects `sessions.updated` (`payload.change != "created"` && `payload.session.status == "completed"`).
    pub fn is_session_completed_for(&self, target_session_id: &str) -> bool {
        self.session_update_status(target_session_id) == Some("completed")
    }
}

#[derive(Debug, Clone)]
pub(crate) struct RpcResponse {
    pub status: u32,
    pub body: Vec<u8>,
}

impl RpcResponse {
    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.body).into_owned()
    }

    pub fn json(&self) -> Result<Value, VmError> {
        serde_json::from_slice(&self.body)
            .map_err(|e| VmError::Transport(format!("invalid RPC JSON: {e}")))
    }
}
