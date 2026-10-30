//! Online Database Backup Export, Dry-run Inspection, and Key-Mapped Transactional Restore (ADR 0131).

mod bundle;
mod codec;
mod importer;
mod inspector;
#[cfg(test)]
mod tests;

pub(crate) use bundle::*;

use crate::database::{Database, secrets::SecretResolver};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;

pub const MAX_BACKUP_BODY_BYTES: usize = 64 * 1024 * 1024;
pub(crate) const SQLITE_MAGIC_HEADER: &[u8; 16] = b"SQLite format 3\0";
pub(crate) const BUNDLE_SCHEMA_VERSION: u32 = 1;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BackupFormat {
    #[default]
    Json,
    Sqlite,
}

impl BackupFormat {
    pub fn parse(raw: &str) -> Option<Self> {
        match raw.trim() {
            "json" => Some(Self::Json),
            "sqlite" | "db" => Some(Self::Sqlite),
            _ => None,
        }
    }

    pub fn extension(self) -> &'static str {
        match self {
            Self::Json => "json",
            Self::Sqlite => "db",
        }
    }

    pub fn content_type(self) -> &'static str {
        match self {
            Self::Json => "application/json",
            Self::Sqlite => "application/vnd.sqlite3",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BackupScope {
    #[default]
    Full,
    ConfigOnly,
}

impl BackupScope {
    pub fn parse(raw: &str) -> Option<Self> {
        match raw.trim() {
            "full" => Some(Self::Full),
            "config_only" => Some(Self::ConfigOnly),
            _ => None,
        }
    }

    pub fn resolve_effective(self, bundle_scope: Self) -> Self {
        match (self, bundle_scope) {
            (Self::Full, Self::Full) => Self::Full,
            _ => Self::ConfigOnly,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ImportMode {
    #[default]
    Merge,
    Replace,
}

impl ImportMode {
    pub fn parse(raw: &str) -> Option<Self> {
        match raw.trim() {
            "merge" => Some(Self::Merge),
            "replace" => Some(Self::Replace),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct BackupCompanionSettings {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub background_settings: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub knowledge_mcp_settings: Option<Value>,
}

#[derive(Clone, Debug, Default)]
pub struct ExportBackupRequest {
    pub format: BackupFormat,
    pub scope: BackupScope,
    pub companion_settings: BackupCompanionSettings,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BackupImportOptions {
    pub mode: ImportMode,
    pub scope: BackupScope,
    pub clear_unreadable_credentials: bool,
}

impl Default for BackupImportOptions {
    fn default() -> Self {
        Self {
            mode: ImportMode::Merge,
            scope: BackupScope::Full,
            clear_unreadable_credentials: true,
        }
    }
}

pub struct InspectBackupRequest<'a> {
    pub payload: &'a [u8],
    pub options: BackupImportOptions,
    pub secret_resolver: &'a dyn SecretResolver,
}

pub struct ImportBackupRequest<'a> {
    pub request_id: &'a str,
    pub payload: &'a [u8],
    pub options: BackupImportOptions,
    pub secret_resolver: &'a dyn SecretResolver,
}

#[derive(Clone, Debug)]
pub struct ExportedBackup {
    pub format: BackupFormat,
    pub scope: BackupScope,
    pub filename: String,
    pub content_type: &'static str,
    pub bytes: Vec<u8>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct BackupTotals {
    pub to_create: u64,
    pub to_update: u64,
    pub to_delete: u64,
    pub unchanged: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BackupResourceCount {
    pub resource: String,
    pub in_backup: u64,
    pub to_create: u64,
    pub to_update: u64,
    pub to_delete: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct UnreadableCredentialItem {
    pub resource_type: String,
    pub resource_key: String,
    pub action: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct BackupCredentialSummary {
    pub total_encrypted: u64,
    pub decryptable: u64,
    pub unreadable: u64,
    pub unreadable_items: Vec<UnreadableCredentialItem>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BackupInspectionReport {
    pub detected_format: BackupFormat,
    pub bundle_scope: BackupScope,
    pub effective_scope: BackupScope,
    pub mode: ImportMode,
    pub created_at: i64,
    pub server_version: String,
    pub includes_background_settings: bool,
    pub includes_knowledge_mcp_settings: bool,
    pub totals: BackupTotals,
    pub resources: Vec<BackupResourceCount>,
    pub credentials: BackupCredentialSummary,
    pub warnings: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BackupImportReport {
    pub detected_format: BackupFormat,
    pub effective_scope: BackupScope,
    pub mode: ImportMode,
    pub created_at: i64,
    pub server_version: String,
    pub applied_background_settings: bool,
    pub applied_knowledge_mcp_settings: bool,
    pub totals: BackupTotals,
    pub resources: Vec<BackupResourceCount>,
    pub credentials: BackupCredentialSummary,
    pub warnings: Vec<String>,
}

#[derive(Clone, Debug)]
pub struct BackupImportOutcome {
    pub report: BackupImportReport,
    pub effective_scope: BackupScope,
    pub companion_settings: BackupCompanionSettings,
    pub touched_agent_ids: Vec<i64>,
    pub touched_server_ids: Vec<i64>,
    pub touched_speaker_ids: Vec<i64>,
    pub touched_template_ids: Vec<i64>,
}

#[derive(Debug, Error)]
pub enum BackupError {
    #[error("backup_payload_empty")]
    EmptyPayload,
    #[error("backup_payload_too_large")]
    PayloadTooLarge,
    #[error("backup_invalid_format")]
    InvalidFormat,
    #[error("backup_checksum_mismatch")]
    ChecksumMismatch,
    #[error("backup_unsupported_version")]
    UnsupportedVersion,
    #[error("backup_unreadable_credentials")]
    UnreadableCredentials,
    #[error("backup_reference_invalid: {0}")]
    InvalidReference(String),
    #[error("database_busy")]
    Busy,
    #[error("database_unavailable")]
    Unavailable,
}

impl From<sqlx::Error> for BackupError {
    fn from(error: sqlx::Error) -> Self {
        if crate::database::is_busy(&error) {
            Self::Busy
        } else {
            Self::Unavailable
        }
    }
}

impl Database {
    pub async fn export_backup(
        &self,
        req: ExportBackupRequest,
    ) -> Result<ExportedBackup, BackupError> {
        let now = chrono::Utc::now();
        let timestamp = now.format("%Y%m%d-%H%M%S").to_string();
        let filename = format!(
            "voice-agent-backup-{timestamp}.{}",
            req.format.extension()
        );
        let bytes = match req.format {
            BackupFormat::Json => {
                let bundle = codec::export_bundle_from_pool(
                    &self.pool,
                    req.scope,
                    req.companion_settings,
                )
                .await?;
                codec::encode_bundle_json(bundle)?
            }
            BackupFormat::Sqlite => codec::export_sqlite_snapshot(&self.pool, req.scope).await?,
        };
        Ok(ExportedBackup {
            format: req.format,
            scope: req.scope,
            filename,
            content_type: req.format.content_type(),
            bytes,
        })
    }

    pub async fn inspect_backup(
        &self,
        req: InspectBackupRequest<'_>,
    ) -> Result<BackupInspectionReport, BackupError> {
        let (detected_format, bundle) = codec::decode_backup_payload(req.payload).await?;
        inspector::inspect_bundle(
            &self.pool,
            detected_format,
            &bundle,
            req.options,
            req.secret_resolver,
        )
        .await
    }

    pub async fn import_backup(
        &self,
        req: ImportBackupRequest<'_>,
    ) -> Result<BackupImportOutcome, BackupError> {
        let (detected_format, bundle) = codec::decode_backup_payload(req.payload).await?;
        let outcome = importer::import_bundle(
            self,
            req.request_id,
            detected_format,
            bundle,
            req.options,
            req.secret_resolver,
        )
        .await?;
        self.invalidate_tool_security_after_import(&outcome);
        Ok(outcome)
    }

    fn invalidate_tool_security_after_import(&self, outcome: &BackupImportOutcome) {
        self.tool_security.invalidate_system_mcp();
        for &agent_id in &outcome.touched_agent_ids {
            self.tool_security.invalidate_agent(agent_id);
            self.tool_security.invalidate_agent_speakers(agent_id);
        }
        for &server_id in &outcome.touched_server_ids {
            self.tool_security.invalidate_server(server_id);
        }
        for &speaker_id in &outcome.touched_speaker_ids {
            self.tool_security.invalidate_speaker(speaker_id);
        }
        for &template_id in &outcome.touched_template_ids {
            self.tool_security.invalidate_template_speakers(template_id);
        }
    }
}
