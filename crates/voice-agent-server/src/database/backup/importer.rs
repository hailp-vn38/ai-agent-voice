mod replace;
mod upsert_core;
mod upsert_extensions;

use super::{inspector::inspect_bundle, *};
use crate::database::{
    Database,
    audit::{AuditOutcome, audit},
    secrets::SecretResolver,
};
use replace::apply_replace_deletions;
use sqlx::{Sqlite, Transaction};
use std::collections::{HashMap, HashSet};
use upsert_core::{
    upsert_agents_and_assignments, upsert_mcp_and_devices, upsert_providers_and_templates,
};
use upsert_extensions::{
    upsert_full_scope_runtime_tables, upsert_gateway_and_system_settings,
    upsert_speakers_and_catalog,
};

#[derive(Default)]
pub(super) struct TouchedEntityIds {
    pub agent_ids: HashSet<i64>,
    pub server_ids: HashSet<i64>,
    pub speaker_ids: HashSet<i64>,
    pub template_ids: HashSet<i64>,
    pub reminder_device_ids: HashSet<i64>,
}

pub(super) async fn load_scoped_id_map<'a>(
    tx: &mut Transaction<'_, Sqlite>,
    sql: &str,
    mode: ImportMode,
    backup_keys: impl IntoIterator<Item = &'a str>,
) -> Result<HashMap<String, i64>, BackupError> {
    let rows: Vec<(String, i64)> = sqlx::query_as(sql).fetch_all(&mut **tx).await?;
    match mode {
        ImportMode::Merge => Ok(rows.into_iter().collect()),
        ImportMode::Replace => {
            let keep: HashSet<&str> = backup_keys.into_iter().collect();
            Ok(rows
                .into_iter()
                .filter(|(k, _)| keep.contains(k.as_str()))
                .collect())
        }
    }
}

pub(super) async fn import_bundle(
    db: &Database,
    request_id: &str,
    detected_format: BackupFormat,
    bundle: DatabaseBackupBundle,
    options: BackupImportOptions,
    secret_resolver: &dyn SecretResolver,
) -> Result<BackupImportOutcome, BackupError> {
    let inspection =
        inspect_bundle(&db.pool, detected_format, &bundle, options, secret_resolver).await?;
    if inspection.credentials.unreadable > 0 && !options.clear_unreadable_credentials {
        return Err(BackupError::UnreadableCredentials);
    }

    let effective_scope = inspection.effective_scope;
    let now = crate::database::unix_seconds().unwrap_or_default();
    let now_ms = chrono::Utc::now().timestamp_millis();

    let mut tx = db.pool.begin_with("BEGIN IMMEDIATE").await?;
    let mut touched = TouchedEntityIds::default();

    let (provider_map, template_map) = upsert_providers_and_templates(
        &mut tx,
        &bundle,
        options.mode,
        secret_resolver,
        now,
        &mut touched,
    )
    .await?;

    let agent_map = upsert_agents_and_assignments(
        &mut tx,
        &bundle,
        options.mode,
        &template_map,
        now,
        &mut touched,
    )
    .await?;

    let server_map = upsert_mcp_and_devices(
        &mut tx,
        &bundle,
        options.mode,
        &agent_map,
        &template_map,
        secret_resolver,
        now,
        &mut touched,
    )
    .await?;

    let (speaker_map, mut speaker_catalog_touched) = upsert_speakers_and_catalog(
        &mut tx,
        &bundle,
        options.mode,
        &provider_map,
        &agent_map,
        &template_map,
        now,
        &mut touched,
    )
    .await?;

    upsert_gateway_and_system_settings(&mut tx, &bundle, secret_resolver, now, now_ms).await?;

    if effective_scope == BackupScope::Full {
        upsert_full_scope_runtime_tables(
            &mut tx,
            &bundle,
            options.mode,
            &agent_map,
            &template_map,
            &mut touched,
        )
        .await?;
    }

    if options.mode == ImportMode::Replace {
        apply_replace_deletions(
            &mut tx,
            &bundle,
            effective_scope,
            &agent_map,
            &server_map,
            &speaker_map,
            &template_map,
            now,
            &mut touched,
        )
        .await?;
    }

    if !touched.speaker_ids.is_empty() {
        speaker_catalog_touched = true;
    }
    if speaker_catalog_touched {
        sqlx::query(
            "INSERT INTO speaker_catalog (id, revision) VALUES (1, 1)
             ON CONFLICT(id) DO UPDATE SET revision = speaker_catalog.revision + 1",
        )
        .execute(&mut *tx)
        .await?;
    }

    reconcile_touched_device_reminders(&mut tx, &touched.reminder_device_ids).await?;

    let affected_rows =
        inspection.totals.to_create + inspection.totals.to_update + inspection.totals.to_delete;
    audit(
        &mut *tx,
        request_id,
        "database_backup",
        None,
        "import",
        None,
        None,
        AuditOutcome::Success,
        affected_rows,
    )
    .await?;

    tx.commit().await?;

    let applied_background_settings = effective_scope == BackupScope::Full
        && bundle
            .companion_settings
            .background_settings
            .as_ref()
            .is_some_and(|v| {
                serde_json::from_value::<crate::services::task_flow::PersistedBackgroundSettings>(
                    v.clone(),
                )
                .is_ok()
            });
    let applied_knowledge_mcp_settings = effective_scope == BackupScope::Full
        && bundle
            .companion_settings
            .knowledge_mcp_settings
            .as_ref()
            .is_some_and(|v| v.is_object());

    let report = BackupImportReport {
        detected_format,
        effective_scope,
        mode: options.mode,
        created_at: bundle.created_at,
        server_version: bundle.server_version,
        applied_background_settings,
        applied_knowledge_mcp_settings,
        totals: inspection.totals,
        resources: inspection.resources,
        credentials: inspection.credentials,
        warnings: inspection.warnings,
    };

    Ok(BackupImportOutcome {
        report,
        effective_scope,
        companion_settings: if effective_scope == BackupScope::Full {
            bundle.companion_settings
        } else {
            BackupCompanionSettings::default()
        },
        touched_agent_ids: touched.agent_ids.into_iter().collect(),
        touched_server_ids: touched.server_ids.into_iter().collect(),
        touched_speaker_ids: touched.speaker_ids.into_iter().collect(),
        touched_template_ids: touched.template_ids.into_iter().collect(),
    })
}

async fn reconcile_touched_device_reminders(
    tx: &mut Transaction<'_, Sqlite>,
    reminder_device_ids: &HashSet<i64>,
) -> Result<(), BackupError> {
    for &dev_row_id in reminder_device_ids {
        let dev_still_exists: i64 =
            sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM devices WHERE id = ?)")
                .bind(dev_row_id)
                .fetch_one(&mut **tx)
                .await?;
        if dev_still_exists == 0 {
            continue;
        }
        sqlx::query("DELETE FROM device_reminder_cached_items WHERE device_row_id = ?")
            .bind(dev_row_id)
            .execute(&mut **tx)
            .await?;
        let max_sched_rev: i64 = sqlx::query_scalar(
            "SELECT COALESCE(MAX(schedule_revision), 0) FROM device_reminders WHERE device_row_id = ?",
        )
        .bind(dev_row_id)
        .fetch_one(&mut **tx)
        .await?;
        sqlx::query(
            "INSERT INTO device_reminder_sync (device_row_id, schedule_revision, last_acked_revision)
             VALUES (?, ?, 0)
             ON CONFLICT(device_row_id) DO UPDATE SET
                 schedule_revision = MAX(device_reminder_sync.schedule_revision + 1, excluded.schedule_revision)",
        )
        .bind(dev_row_id)
        .bind(max_sched_rev.max(1))
        .execute(&mut **tx)
        .await?;
    }
    Ok(())
}
