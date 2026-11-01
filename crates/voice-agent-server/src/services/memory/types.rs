//! Public data types of the `MemoryCore` interface.

use chrono::{DateTime, Utc};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::sync::Arc;
use tokio::sync::watch;

pub(super) const VIRTUAL_DEVICE_DIR: &str = "web-assistant";

/// Which vault a call touches. Only the server can build one: no tool takes a scope from the model.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct MemoryScope(pub(super) ScopeTarget);

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(super) enum ScopeTarget {
    Device { dir: String },
    Deployment,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeviceKind {
    Physical,
    Virtual,
}

impl DeviceKind {
    /// Maps `devices.kind`, which `memory_binding` returns as stored.
    pub fn from_stored(kind: &str) -> Self {
        if kind == "web_assistant" {
            Self::Virtual
        } else {
            Self::Physical
        }
    }
}

impl From<crate::database::devices::MemoryMode> for MemoryMode {
    fn from(mode: crate::database::devices::MemoryMode) -> Self {
        use crate::database::devices::MemoryMode as Stored;
        match mode {
            Stored::Off => Self::Off,
            Stored::Read => Self::Read,
            Stored::ReadWrite => Self::ReadWrite,
        }
    }
}

impl MemoryScope {
    /// The raw `device_id` is client-supplied, so it never reaches a path or this value (ADR 0024).
    pub fn for_device(device_id: &str, kind: DeviceKind) -> Self {
        let dir = match kind {
            DeviceKind::Virtual => VIRTUAL_DEVICE_DIR.to_string(),
            DeviceKind::Physical => {
                let digest = Sha256::digest(device_id.as_bytes());
                digest.iter().take(8).map(|b| format!("{b:02x}")).collect()
            }
        };
        Self(ScopeTarget::Device { dir })
    }

    /// Accepts only `SetProfile` and `inspect`.
    pub fn deployment() -> Self {
        Self(ScopeTarget::Deployment)
    }
}

/// Stable identity of a fact, `fact_<uuid v4 simple>`. Anything else, such as a Wiki Page id, does
/// not parse, so it cannot reach a Memory operation.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize)]
#[serde(transparent)]
pub struct FactId(String);

impl FactId {
    pub fn parse(value: &str) -> Option<Self> {
        let hex = value.strip_prefix("fact_")?;
        (hex.len() == 32
            && hex
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)))
        .then(|| Self(value.to_string()))
    }

    pub(super) fn generate() -> Self {
        Self(format!("fact_{}", uuid::Uuid::new_v4().simple()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// The part after `fact_`, which disambiguates colliding slugs.
    pub(super) fn slug_suffix(&self) -> &str {
        &self.0["fact_".len().."fact_".len() + 6]
    }
}

macro_rules! text_enum {
    ($(#[$meta:meta])* $name:ident { $($variant:ident => $text:literal),+ $(,)? }) => {
        $(#[$meta])*
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
        pub enum $name { $(#[serde(rename = $text)] $variant),+ }

        impl $name {
            pub fn as_str(self) -> &'static str {
                match self { $(Self::$variant => $text),+ }
            }

            pub fn parse(value: &str) -> Option<Self> {
                match value { $($text => Some(Self::$variant),)+ _ => None }
            }
        }
    };
}

text_enum!(FactKind {
    User => "user",
    Preference => "preference",
    Business => "business",
    Decision => "decision",
    Reference => "reference",
});

text_enum!(Provenance {
    User => "user",
    Source => "source",
    Assistant => "assistant",
});

text_enum!(FactOrigin {
    Voice => "voice",
    Admin => "admin",
    Learned => "learned",
    OwnerEdited => "owner-edited",
});

text_enum!(FactStatus {
    Confirmed => "confirmed",
    Superseded => "superseded",
});

text_enum!(
    /// `devices.memory_mode`, fixed for a Voice Session at admission.
    MemoryMode {
        Off => "off",
        Read => "read",
        ReadWrite => "read_write",
    }
);

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct MemoryFact {
    pub id: FactId,
    pub revision: u64,
    pub title: String,
    pub hook: String,
    pub body: String,
    pub kind: FactKind,
    pub provenance: Provenance,
    pub origin: FactOrigin,
    pub status: FactStatus,
    pub superseded_by: Option<String>,
    pub pinned: bool,
    pub source_ref: Option<String>,
    #[serde(serialize_with = "serialize_time")]
    pub created_at: DateTime<Utc>,
    #[serde(serialize_with = "serialize_time")]
    pub updated_at: DateTime<Utc>,
}

pub enum MemoryChange {
    Remember {
        title: String,
        hook: String,
        body: String,
        kind: FactKind,
        source_ref: Option<String>,
    },
    Correct {
        id: FactId,
        expected_revision: u64,
        title: Option<String>,
        hook: Option<String>,
        body: String,
    },
    /// Undo: removes the fact and leaves no Forget Tombstone.
    Discard {
        id: FactId,
        expected_revision: u64,
    },
    /// Removes the fact and leaves a Forget Tombstone.
    Forget {
        id: FactId,
        expected_revision: u64,
    },
    /// Does not bump `revision`.
    SetPinned {
        id: FactId,
        pinned: bool,
    },
    /// Admin on the deployment scope only.
    SetProfile {
        body: String,
        expected_revision: u64,
    },
    PurgeScope,
}

#[derive(Clone, Copy, Debug)]
pub enum MemoryActor {
    Admin,
    /// `mode` is the value admission fixed for the session, which is what keeps `apply` the single
    /// place permissions are checked without giving `MemoryCore` a database.
    VoiceUser {
        device_db_id: i64,
        mode: MemoryMode,
        intent: ExplicitIntent,
    },
}

/// One flag per action: a "nhớ giúp X" turn must not unlock Forget or Correct.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ExplicitIntent {
    pub remember: bool,
    pub correct: bool,
    pub forget: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ReceiptAction {
    Remember,
    Correct,
    Discard,
    Forget,
    SetPinned,
    SetProfile,
    PurgeScope,
}

/// Proof of a write: a returned value, never inferred from disk.
#[derive(Clone, Debug, Serialize)]
pub struct MemoryReceipt {
    pub action: ReceiptAction,
    pub fact_id: Option<FactId>,
    pub title: Option<String>,
    pub revision: Option<u64>,
    #[serde(serialize_with = "serialize_time")]
    pub at: DateTime<Utc>,
    /// Committed, but some cleanup did not finish; `reconcile` completes it.
    pub cleanup_pending: bool,
}

/// What a Voice turn sees. Fact text is untrusted data: callers must place it in a data block.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct MemoryContext {
    /// JSON lines, most important first, so cutting at a line boundary stays valid.
    pub rows: Vec<String>,
    pub hidden: usize,
}

/// Reads the latest context without I/O, safe on the actor's hot path.
#[derive(Clone, Debug)]
pub struct MemoryContextHandle(pub(super) watch::Receiver<Arc<MemoryContext>>);

impl MemoryContextHandle {
    pub fn current(&self) -> Arc<MemoryContext> {
        self.0.borrow().clone()
    }
}

#[derive(Clone, Debug)]
pub struct RecallQuery {
    pub text: String,
    pub limit: usize,
}

#[derive(Clone, Debug, Serialize)]
pub struct RecallResult {
    /// Untrusted data.
    pub items: Vec<MemoryFact>,
    /// Matching facts left out by `limit`, as far as the over-fetch could see.
    pub hidden: usize,
}

#[derive(Clone, Debug, Default)]
pub struct InspectFilter {
    pub id: Option<FactId>,
}

#[derive(Clone, Debug, Serialize)]
pub struct MemoryProfile {
    pub body: String,
    pub revision: u64,
    #[serde(serialize_with = "serialize_optional_time")]
    pub updated_at: Option<DateTime<Utc>>,
}

#[derive(Clone, Debug, Serialize)]
pub struct MemoryOverview {
    pub facts: Vec<MemoryFact>,
    pub quarantined: usize,
    /// File names inside `facts/`, so an Admin can find and repair them.
    pub quarantined_files: Vec<String>,
    /// Only on the deployment scope.
    pub profile: Option<MemoryProfile>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
pub struct ReconcileReport {
    pub facts: usize,
    pub quarantined: usize,
    pub tombstones_applied: usize,
    pub owner_edited: usize,
    pub temp_files_removed: usize,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct ReconcileAllReport {
    pub reconciled: usize,
    pub failed: usize,
    /// Directories under `devices/` outside `known`. Reported, never deleted: one wrong import must
    /// stay recoverable.
    pub orphans: Vec<String>,
}

/// Messages never carry fact content (ADR 0024).
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum MemoryError {
    #[error("Memory is disabled")]
    Disabled,
    #[error("Memory is read-only on this Device")]
    ReadOnly,
    #[error("the user did not explicitly request this Memory change in this turn")]
    RequiresExplicitRequest,
    #[error("the fact changed since its revision was read")]
    RevisionConflict,
    #[error("no such fact")]
    NotFound,
    #[error("the vault reached its fact limit")]
    LimitReached,
    #[error("a field exceeds its size limit")]
    TooLarge,
    #[error("the content looks like a secret")]
    RejectedSensitive,
    #[error("this actor or scope may not perform this operation")]
    ScopeDenied,
    #[error("Memory storage is unavailable")]
    Unavailable,
}

impl MemoryError {
    /// Shared by tool results and HTTP responses.
    pub fn code(self) -> &'static str {
        match self {
            Self::Disabled => "memory_disabled",
            Self::ReadOnly => "memory_read_only",
            Self::RequiresExplicitRequest => "memory_requires_explicit_request",
            Self::RevisionConflict => "memory_revision_conflict",
            Self::NotFound => "memory_not_found",
            Self::LimitReached => "memory_limit_reached",
            Self::TooLarge => "memory_too_large",
            Self::RejectedSensitive => "memory_rejected_sensitive",
            Self::ScopeDenied => "memory_scope_denied",
            Self::Unavailable => "memory_unavailable",
        }
    }

    pub fn http_status(self) -> http::StatusCode {
        use http::StatusCode;
        match self {
            Self::Disabled | Self::Unavailable => StatusCode::SERVICE_UNAVAILABLE,
            Self::ReadOnly | Self::RequiresExplicitRequest | Self::ScopeDenied => {
                StatusCode::FORBIDDEN
            }
            Self::RevisionConflict | Self::LimitReached => StatusCode::CONFLICT,
            Self::NotFound => StatusCode::NOT_FOUND,
            Self::TooLarge | Self::RejectedSensitive => StatusCode::BAD_REQUEST,
        }
    }
}

pub(super) fn format_time(time: &DateTime<Utc>) -> String {
    time.to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

fn serialize_time<S: serde::Serializer>(
    time: &DateTime<Utc>,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    serializer.serialize_str(&format_time(time))
}

fn serialize_optional_time<S: serde::Serializer>(
    time: &Option<DateTime<Utc>>,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    match time {
        Some(time) => serializer.serialize_str(&format_time(time)),
        None => serializer.serialize_none(),
    }
}
