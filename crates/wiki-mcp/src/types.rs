use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::time::Duration;
use thiserror::Error;
use tokio_util::sync::CancellationToken;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Access {
    ReadOnly,
    ReadWrite,
}

#[derive(Debug, Clone)]
pub struct CallCtx {
    pub access: Access,
    pub cancel: CancellationToken,
    pub deadline: tokio::time::Instant,
}

#[derive(Debug, Clone, Copy)]
pub struct Policy {
    pub allow_write: bool,
    pub max_concurrency: usize,
    pub call_timeout: Duration,
    pub max_result_chars: usize,
}

impl Default for Policy {
    fn default() -> Self {
        Self {
            allow_write: true,
            max_concurrency: 16,
            call_timeout: Duration::from_secs(30),
            max_result_chars: 12_000,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolSpec {
    pub name: String,
    pub description: String,
    pub parameters: Value,
}

#[derive(Debug, PartialEq, Eq, Error)]
pub enum ToolError {
    #[error("knowledge_tool_denied")]
    Denied,
    #[error("invalid_arguments: {0}")]
    InvalidArguments(String),
    #[error("knowledge_tool_error: {0}")]
    ExecutionFailed(String),
    #[error("knowledge_tool_unavailable")]
    Unavailable,
    #[error("knowledge_tool_cancelled")]
    Cancelled,
    #[error("knowledge_tool_timeout")]
    Timeout,
    #[error("knowledge vault is read-only")]
    ReadOnly,
}

impl From<wiki_core::VaultError> for ToolError {
    fn from(error: wiki_core::VaultError) -> Self {
        match error {
            wiki_core::VaultError::ReadOnly => Self::ReadOnly,
            wiki_core::VaultError::InvalidPath(message) => Self::InvalidArguments(message),
            other => Self::ExecutionFailed(other.to_string()),
        }
    }
}
