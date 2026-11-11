//! Public configuration, status, and error types for Wiki synchronization.

use crate::DocPath;
use serde::{Deserialize, Serialize};
use std::{
    path::PathBuf,
    time::{Duration, SystemTime},
};
use thiserror::Error;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncConfig {
    pub remote_url: Option<String>,
    pub branch: String,
    pub author: Option<GitAuthor>,
    pub auth: GitAuth,
    pub max_file_bytes: u64,
    #[serde(with = "duration_serde")]
    pub command_timeout: Duration,
}

impl Default for SyncConfig {
    fn default() -> Self {
        Self {
            remote_url: None,
            branch: "main".into(),
            author: None,
            auth: GitAuth::System,
            max_file_bytes: 50 * 1024 * 1024,
            command_timeout: Duration::from_secs(60),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GitAuthor {
    pub name: String,
    pub email: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum GitAuth {
    System,
    SshKey { key: PathBuf, known_hosts: PathBuf },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DisabledReason {
    NoGit,
    NotConfigured,
    ReadOnly,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeferReason {
    IngestRunning,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SyncErrorKind {
    GitMissing,
    ReadOnly,
    Auth,
    Network,
    Timeout,
    FileTooLarge,
    UnsafeRemote,
    Git,
    Vault,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SyncStatus {
    Disabled {
        reason: DisabledReason,
    },
    Idle {
        #[serde(with = "option_system_time")]
        last_sync: Option<SystemTime>,
        ahead: u32,
        behind: u32,
    },
    Syncing,
    Deferred {
        reason: DeferReason,
        #[serde(with = "system_time")]
        retry_at: SystemTime,
    },
    Conflicted {
        copies: Vec<DocPath>,
    },
    Error {
        kind: SyncErrorKind,
        message: String,
    },
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SyncReport {
    pub committed: usize,
    pub pulled: Vec<DocPath>,
    pub conflicts: Vec<DocPath>,
    pub pushed: bool,
}

#[derive(Debug, Error, Clone, Serialize, Deserialize)]
#[serde(tag = "type", content = "detail", rename_all = "snake_case")]
pub enum SyncError {
    #[error("git is not installed or not available on PATH")]
    GitMissing,
    #[error("This process does not hold the vault write lock")]
    ReadOnly,
    #[error("Git authentication failed: {0}")]
    Auth(String),
    #[error("Git network operation failed: {0}")]
    Network(String),
    #[error("Git command timed out: {command}")]
    Timeout { command: String },
    #[error("File is larger than the configured limit ({bytes} bytes): {path}")]
    FileTooLarge { path: String, bytes: u64 },
    #[error("Unsafe remote URL: {0}")]
    UnsafeRemote(String),
    #[error("Git operation failed: {0}")]
    Git(String),
    #[error("Vault operation failed: {0}")]
    Vault(String),
}

impl SyncError {
    pub fn kind(&self) -> SyncErrorKind {
        match self {
            Self::GitMissing => SyncErrorKind::GitMissing,
            Self::ReadOnly => SyncErrorKind::ReadOnly,
            Self::Auth(_) => SyncErrorKind::Auth,
            Self::Network(_) => SyncErrorKind::Network,
            Self::Timeout { .. } => SyncErrorKind::Timeout,
            Self::FileTooLarge { .. } => SyncErrorKind::FileTooLarge,
            Self::UnsafeRemote(_) => SyncErrorKind::UnsafeRemote,
            Self::Git(_) => SyncErrorKind::Git,
            Self::Vault(_) => SyncErrorKind::Vault,
        }
    }
}

mod duration_serde {
    use serde::{Deserialize, Deserializer, Serializer};
    use std::time::Duration;
    pub fn serialize<S: Serializer>(value: &Duration, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_u64(value.as_secs())
    }
    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Duration, D::Error> {
        Ok(Duration::from_secs(u64::deserialize(deserializer)?))
    }
}
mod system_time {
    use serde::{Deserialize, Deserializer, Serializer};
    use std::time::{Duration, SystemTime, UNIX_EPOCH};
    pub fn serialize<S: Serializer>(value: &SystemTime, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_u64(
            value
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs(),
        )
    }
    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<SystemTime, D::Error> {
        Ok(UNIX_EPOCH + Duration::from_secs(u64::deserialize(deserializer)?))
    }
}
mod option_system_time {
    use serde::{Deserialize, Deserializer, Serialize, Serializer};
    use std::time::{Duration, SystemTime, UNIX_EPOCH};
    pub fn serialize<S: Serializer>(
        value: &Option<SystemTime>,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        value
            .map(|time| {
                time.duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs()
            })
            .serialize(serializer)
    }
    pub fn deserialize<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<Option<SystemTime>, D::Error> {
        Ok(Option::<u64>::deserialize(deserializer)?
            .map(|secs| UNIX_EPOCH + Duration::from_secs(secs)))
    }
}
