mod queries_core;
mod queries_extensions;

use super::*;
use queries_core::*;
use queries_extensions::*;
use sha2::{Digest, Sha256};
use sqlx::{
    SqlitePool,
    sqlite::{SqliteConnectOptions, SqlitePoolOptions},
};
use std::{path::PathBuf, str::FromStr, time::Duration};

const SQLITE_SNAPSHOT_CONFIG_ONLY_USER_VERSION: i64 = 1;

struct TempFileGuard {
    path: PathBuf,
}

impl TempFileGuard {
    fn new(extension: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "voice-agent-backup-{}.{extension}",
            uuid::Uuid::new_v4()
        ));
        Self { path }
    }
}

impl Drop for TempFileGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
        let wal = self.path.with_extension("db-wal");
        let shm = self.path.with_extension("db-shm");
        let _ = std::fs::remove_file(wal);
        let _ = std::fs::remove_file(shm);
    }
}

pub(super) fn compute_bundle_checksum(
    bundle: &DatabaseBackupBundle,
) -> Result<String, BackupError> {
    let mut clone = bundle.clone();
    clone.checksum_sha256 = None;
    let canonical = serde_json::to_vec(&clone).map_err(|_| BackupError::InvalidFormat)?;
    let mut hasher = Sha256::new();
    hasher.update(&canonical);
    let digest = hasher.finalize();
    Ok(digest.iter().map(|b| format!("{b:02x}")).collect())
}

pub(super) fn encode_bundle_json(
    mut bundle: DatabaseBackupBundle,
) -> Result<Vec<u8>, BackupError> {
    bundle.checksum_sha256 = Some(compute_bundle_checksum(&bundle)?);
    serde_json::to_vec_pretty(&bundle).map_err(|_| BackupError::InvalidFormat)
}

pub(super) async fn decode_backup_payload(
    payload: &[u8],
) -> Result<(BackupFormat, DatabaseBackupBundle), BackupError> {
    if payload.is_empty() {
        return Err(BackupError::EmptyPayload);
    }
    if payload.len() > MAX_BACKUP_BODY_BYTES {
        return Err(BackupError::PayloadTooLarge);
    }
    if payload.starts_with(SQLITE_MAGIC_HEADER) {
        let bundle = read_bundle_from_sqlite_bytes(payload).await?;
        Ok((BackupFormat::Sqlite, bundle))
    } else {
        let bundle: DatabaseBackupBundle =
            serde_json::from_slice(payload).map_err(|_| BackupError::InvalidFormat)?;
        if bundle.schema_version != BUNDLE_SCHEMA_VERSION {
            return Err(BackupError::UnsupportedVersion);
        }
        if let Some(ref expected) = bundle.checksum_sha256 {
            let actual = compute_bundle_checksum(&bundle)?;
            if &actual != expected {
                return Err(BackupError::ChecksumMismatch);
            }
        }
        Ok((BackupFormat::Json, bundle))
    }
}

pub(super) async fn export_sqlite_snapshot(
    pool: &SqlitePool,
    scope: BackupScope,
) -> Result<Vec<u8>, BackupError> {
    let guard = TempFileGuard::new("db");
    let escaped_path = guard
        .path
        .to_str()
        .ok_or(BackupError::Unavailable)?
        .replace('\'', "''");
    let vacuum_sql = format!("VACUUM INTO '{escaped_path}'");
    sqlx::query(&vacuum_sql).execute(pool).await?;

    if scope == BackupScope::ConfigOnly {
        let opts = SqliteConnectOptions::from_str(
            guard.path.to_str().ok_or(BackupError::Unavailable)?,
        )
        .map_err(|_| BackupError::Unavailable)?
        .create_if_missing(false)
        .foreign_keys(false)
        .busy_timeout(Duration::from_secs(5));
        let temp_pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(opts)
            .await
            .map_err(|_| BackupError::Unavailable)?;

        for table in [
            "notification_outbox",
            "notifications_inbox",
            "reminder_delivery_attempts",
            "reminder_mutation_keys",
            "device_reminder_cached_items",
            "device_reminders",
            "device_reminder_sync",
            "history_messages",
            "device_enrollments",
            "speaker_enrollment_samples",
            "speaker_enrollment_drafts",
            "speaker_quick_captures",
            "admin_audit_events",
            "system_mcp_token_revocations",
        ] {
            let exists: i64 = sqlx::query_scalar(
                "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?)",
            )
            .bind(table)
            .fetch_one(&temp_pool)
            .await?;
            if exists != 0 {
                let delete_sql = format!("DELETE FROM {table}");
                sqlx::query(&delete_sql).execute(&temp_pool).await?;
            }
        }
        let set_version_sql =
            format!("PRAGMA user_version = {SQLITE_SNAPSHOT_CONFIG_ONLY_USER_VERSION}");
        sqlx::query(&set_version_sql).execute(&temp_pool).await?;
        sqlx::query("VACUUM").execute(&temp_pool).await?;
        temp_pool.close().await;
    }

    tokio::fs::read(&guard.path)
        .await
        .map_err(|_| BackupError::Unavailable)
}

async fn read_bundle_from_sqlite_bytes(
    payload: &[u8],
) -> Result<DatabaseBackupBundle, BackupError> {
    let guard = TempFileGuard::new("db");
    tokio::fs::write(&guard.path, payload)
        .await
        .map_err(|_| BackupError::Unavailable)?;

    let opts = SqliteConnectOptions::from_str(
        guard.path.to_str().ok_or(BackupError::InvalidFormat)?,
    )
    .map_err(|_| BackupError::InvalidFormat)?
    .read_only(true)
    .create_if_missing(false)
    .pragma("trusted_schema", "OFF")
    .pragma("cell_size_check", "ON")
    .pragma("query_only", "ON")
    .busy_timeout(Duration::from_secs(5));

    let temp_pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(opts)
        .await
        .map_err(|_| BackupError::InvalidFormat)?;

    let has_providers: i64 = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'providers')",
    )
    .fetch_one(&temp_pool)
    .await
    .map_err(|_| BackupError::InvalidFormat)?;
    if has_providers == 0 {
        temp_pool.close().await;
        return Err(BackupError::InvalidFormat);
    }

    let user_version: i64 = sqlx::query_scalar("PRAGMA user_version")
        .fetch_one(&temp_pool)
        .await
        .unwrap_or(0);
    let snapshot_scope = if user_version == SQLITE_SNAPSHOT_CONFIG_ONLY_USER_VERSION {
        BackupScope::ConfigOnly
    } else {
        BackupScope::Full
    };

    let bundle = export_bundle_from_pool(
        &temp_pool,
        snapshot_scope,
        BackupCompanionSettings::default(),
    )
    .await
    .map_err(|_| BackupError::InvalidFormat)?;
    temp_pool.close().await;
    Ok(bundle)
}

pub(super) async fn export_bundle_from_pool(
    pool: &SqlitePool,
    scope: BackupScope,
    companion_settings: BackupCompanionSettings,
) -> Result<DatabaseBackupBundle, BackupError> {
    let created_at = crate::database::unix_seconds().unwrap_or_default();
    let server_version = env!("CARGO_PKG_VERSION").to_owned();

    let providers = fetch_providers(pool).await?;
    let agent_templates = fetch_templates(pool).await?;
    let template_provider_bindings = fetch_template_provider_bindings(pool).await?;
    let agents = fetch_agents(pool).await?;
    let agent_template_assignments = fetch_agent_template_assignments(pool).await?;
    let mcp_servers = fetch_mcp_servers(pool).await?;
    let agent_mcp_bindings = fetch_agent_mcp_bindings(pool).await?;
    let external_tool_observations = fetch_external_tool_observations(pool).await?;
    let agent_external_tool_allowlist = fetch_agent_tool_allowlist(pool).await?;
    let devices = fetch_devices(pool).await?;
    let speakers = fetch_speakers(pool).await?;
    let speaker_voiceprints = fetch_speaker_voiceprints(pool).await?;
    let agent_speaker_policies = fetch_agent_speaker_policies(pool).await?;
    let agent_speaker_candidates = fetch_agent_speaker_candidates(pool).await?;
    let agent_speaker_template_grants = fetch_agent_speaker_template_grants(pool).await?;
    let speaker_calibration_catalog = fetch_speaker_calibration_catalog(pool).await?;
    let speaker_calibration_evidence = fetch_speaker_calibration_evidence(pool).await?;
    let llm_gateway_accounts = fetch_llm_gateway_accounts(pool).await?;
    let notification_settings_state = fetch_notification_settings(pool).await?;
    let notification_channel_configs = fetch_notification_channels(pool).await?;
    let system_mcp_grants = fetch_system_mcp_grants(pool).await?;

    let (device_reminders, history_messages, notifications_inbox, companion_settings) =
        if scope == BackupScope::Full {
            (
                fetch_device_reminders(pool).await?,
                fetch_history_messages(pool).await?,
                fetch_notifications_inbox(pool).await?,
                companion_settings,
            )
        } else {
            (
                Vec::new(),
                Vec::new(),
                Vec::new(),
                BackupCompanionSettings::default(),
            )
        };

    Ok(DatabaseBackupBundle {
        schema_version: BUNDLE_SCHEMA_VERSION,
        server_version,
        created_at,
        scope,
        checksum_sha256: None,
        providers,
        agent_templates,
        template_provider_bindings,
        agents,
        agent_template_assignments,
        mcp_servers,
        agent_mcp_bindings,
        external_tool_observations,
        agent_external_tool_allowlist,
        devices,
        speakers,
        speaker_voiceprints,
        agent_speaker_policies,
        agent_speaker_candidates,
        agent_speaker_template_grants,
        speaker_calibration_catalog,
        speaker_calibration_evidence,
        llm_gateway_accounts,
        notification_settings_state,
        notification_channel_configs,
        system_mcp_grants,
        device_reminders,
        history_messages,
        notifications_inbox,
        companion_settings,
    })
}
