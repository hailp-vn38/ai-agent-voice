//! Runtime configuration, redacted token credentials, and persistence for Knowledge MCP.

use crate::config::WikiMcpConfig;
use serde::{Deserialize, Serialize};
use std::{
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

pub(crate) const SETTINGS_FILENAME: &str = "knowledge_mcp_settings.json";
pub(crate) const CONCURRENCY_RANGE: std::ops::RangeInclusive<usize> = 1..=64;
pub(crate) const CALL_TIMEOUT_MS_RANGE: std::ops::RangeInclusive<u64> = 1_000..=120_000;
pub(crate) const RESULT_CHARS_RANGE: std::ops::RangeInclusive<usize> = 512..=64_000;
pub(crate) const MAX_TOKENS: usize = 64;

/// Access level resolved for an authenticated Knowledge MCP caller.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum KnowledgeMcpAccess {
    ReadOnly,
    #[default]
    ReadWrite,
}

/// Stored token entry in `knowledge_mcp_settings.json`.
/// `Debug` redacts the raw token string; Admin GET responses use [`KnowledgeMcpTokenSummary`]
/// so existing secret tokens are never re-exposed after creation.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct KnowledgeMcpTokenEntry {
    pub id: String,
    pub name: String,
    pub token: String,
    pub access: KnowledgeMcpAccess,
    pub created_at: i64,
}

impl std::fmt::Debug for KnowledgeMcpTokenEntry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("KnowledgeMcpTokenEntry")
            .field("id", &self.id)
            .field("name", &self.name)
            .field("token", &"[REDACTED]")
            .field("access", &self.access)
            .field("created_at", &self.created_at)
            .finish()
    }
}

impl KnowledgeMcpTokenEntry {
    pub(crate) fn masked_token(&self) -> String {
        let len = self.token.len();
        if len <= 12 {
            return "••••••••".to_string();
        }
        let prefix = self
            .token
            .char_indices()
            .nth(8)
            .map(|(idx, _)| &self.token[..idx])
            .unwrap_or("kmcp_");
        let suffix = self
            .token
            .char_indices()
            .rev()
            .nth(3)
            .map(|(idx, _)| &self.token[idx..])
            .unwrap_or("");
        format!("{prefix}••••••••{suffix}")
    }

    pub(crate) fn to_summary(&self) -> KnowledgeMcpTokenSummary {
        KnowledgeMcpTokenSummary {
            id: self.id.clone(),
            name: self.name.clone(),
            masked_token: self.masked_token(),
            access: self.access,
            created_at: self.created_at,
        }
    }
}

/// Safe, secret-free summary returned on `GET /api/admin/system-mcp/knowledge`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct KnowledgeMcpTokenSummary {
    pub id: String,
    pub name: String,
    pub masked_token: String,
    pub access: KnowledgeMcpAccess,
    pub created_at: i64,
}

/// Persisted Knowledge MCP runtime configuration editable from the Admin Web UI.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct PersistedKnowledgeMcpSettings {
    pub revision: i64,
    pub enabled: bool,
    pub path: String,
    pub allow_write: bool,
    pub max_concurrency: usize,
    pub call_timeout_ms: u64,
    pub max_result_chars: usize,
    pub tokens: Vec<KnowledgeMcpTokenEntry>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct UpdateKnowledgeMcpInput {
    pub enabled: bool,
    pub allow_write: bool,
    pub max_concurrency: Option<usize>,
    pub call_timeout_ms: Option<u64>,
    pub max_result_chars: Option<usize>,
    pub expected_revision: Option<i64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum KnowledgeMcpAdminError {
    ValidationFailed,
    RevisionConflict,
    NotFound,
}

pub(crate) fn persisted_settings_exist(store_dir: &Path) -> bool {
    store_dir.join(SETTINGS_FILENAME).is_file()
}

pub(crate) fn load_or_seed_settings(
    cfg: &WikiMcpConfig,
    store_path: Option<&PathBuf>,
) -> PersistedKnowledgeMcpSettings {
    let mut settings = store_path
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|raw| serde_json::from_str::<PersistedKnowledgeMcpSettings>(&raw).ok())
        .unwrap_or_else(|| seed_settings_from_config(cfg));
    if settings.revision < 1 {
        settings.revision = 1;
    }
    merge_static_tokens(&mut settings.tokens, cfg);
    settings
}

pub(crate) fn persist_settings(
    store_path: Option<&PathBuf>,
    settings: &PersistedKnowledgeMcpSettings,
) {
    let Some(path) = store_path else {
        return;
    };
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(serialized) = serde_json::to_string_pretty(settings) {
        let _ = std::fs::write(path, serialized);
    }
}

pub(crate) fn unix_now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

pub(crate) fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (&x, &y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

fn seed_settings_from_config(cfg: &WikiMcpConfig) -> PersistedKnowledgeMcpSettings {
    let mut tokens = Vec::new();
    merge_static_tokens(&mut tokens, cfg);
    PersistedKnowledgeMcpSettings {
        revision: 1,
        enabled: cfg.enabled,
        path: if cfg.path.trim().is_empty() {
            "/mcp/knowledge".to_string()
        } else {
            cfg.path.trim().to_string()
        },
        allow_write: cfg.allow_write,
        max_concurrency: cfg
            .max_concurrency
            .clamp(*CONCURRENCY_RANGE.start(), *CONCURRENCY_RANGE.end()),
        call_timeout_ms: cfg
            .call_timeout_ms
            .clamp(*CALL_TIMEOUT_MS_RANGE.start(), *CALL_TIMEOUT_MS_RANGE.end()),
        max_result_chars: cfg
            .max_result_chars
            .clamp(*RESULT_CHARS_RANGE.start(), *RESULT_CHARS_RANGE.end()),
        tokens,
    }
}

fn merge_static_tokens(tokens: &mut Vec<KnowledgeMcpTokenEntry>, cfg: &WikiMcpConfig) {
    let now = unix_now();
    let mut push_unique = |id: &str, name: &str, raw: &str, access: KnowledgeMcpAccess| {
        let clean = raw.trim();
        if clean.is_empty() {
            return;
        }
        if !tokens.iter().any(|t| t.token == clean) {
            tokens.push(KnowledgeMcpTokenEntry {
                id: id.to_string(),
                name: name.to_string(),
                token: clean.to_string(),
                access,
                created_at: now,
            });
        }
    };
    push_unique(
        "config-default-token",
        "Default Config Token",
        &cfg.token,
        KnowledgeMcpAccess::ReadWrite,
    );
    push_unique(
        "config-write-token",
        "Config Read-Write Token",
        &cfg.write_token,
        KnowledgeMcpAccess::ReadWrite,
    );
    push_unique(
        "config-read-token",
        "Config Read-Only Token",
        &cfg.read_token,
        KnowledgeMcpAccess::ReadOnly,
    );
}
