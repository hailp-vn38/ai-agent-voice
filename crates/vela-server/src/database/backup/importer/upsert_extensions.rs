use super::{
    super::{
        inspector::{scrub_or_keep_gateway_credential, scrub_or_keep_optional_credential},
        *,
    },
    TouchedEntityIds, load_scoped_id_map,
};
use crate::database::secrets::SecretResolver;
use base64::{Engine, engine::general_purpose::STANDARD as BASE64};
use sqlx::{Sqlite, Transaction};
use std::collections::HashMap;

#[allow(clippy::too_many_arguments)]
pub(super) async fn upsert_speakers_and_catalog(
    tx: &mut Transaction<'_, Sqlite>,
    bundle: &DatabaseBackupBundle,
    mode: ImportMode,
    provider_map: &HashMap<String, i64>,
    agent_map: &HashMap<String, i64>,
    template_map: &HashMap<String, i64>,
    now: i64,
    touched: &mut TouchedEntityIds,
) -> Result<(HashMap<String, i64>, bool), BackupError> {
    let speaker_catalog_touched = !bundle.speakers.is_empty()
        || !bundle.speaker_voiceprints.is_empty()
        || !bundle.agent_speaker_policies.is_empty()
        || !bundle.agent_speaker_candidates.is_empty()
        || !bundle.agent_speaker_template_grants.is_empty()
        || bundle.speaker_calibration_catalog.is_some();

    for sp in &bundle.speakers {
        let existing: Option<(i64, i64)> =
            sqlx::query_as("SELECT id, revision FROM speakers WHERE key = ?")
                .bind(&sp.key)
                .fetch_optional(&mut **tx)
                .await?;
        let sp_id = if let Some((id, rev)) = existing {
            sqlx::query(
                "UPDATE speakers
                 SET name = ?, description = ?, enabled = ?, revision = ?, updated_at = ?
                 WHERE id = ?",
            )
            .bind(&sp.name)
            .bind(&sp.description)
            .bind(sp.enabled)
            .bind(rev + 1)
            .bind(now)
            .bind(id)
            .execute(&mut **tx)
            .await?;
            id
        } else {
            sqlx::query(
                "INSERT INTO speakers (key, name, description, enabled, revision, created_at, updated_at)
                 VALUES (?, ?, ?, ?, 1, ?, ?)",
            )
            .bind(&sp.key)
            .bind(&sp.name)
            .bind(&sp.description)
            .bind(sp.enabled)
            .bind(sp.created_at.max(1))
            .bind(now)
            .execute(&mut **tx)
            .await?
            .last_insert_rowid()
        };
        touched.speaker_ids.insert(sp_id);
    }

    let speaker_map = load_scoped_id_map(
        tx,
        "SELECT key, id FROM speakers",
        mode,
        bundle.speakers.iter().map(|s| s.key.as_str()),
    )
    .await?;

    for vp in &bundle.speaker_voiceprints {
        let Some(&sp_id) = speaker_map.get(&vp.speaker_key) else {
            return Err(BackupError::InvalidReference(format!(
                "speaker:{}",
                vp.speaker_key
            )));
        };
        let vector = BASE64
            .decode(vp.vector_base64.as_bytes())
            .map_err(|_| BackupError::InvalidFormat)?;
        let provider_id = provider_map.get(&vp.provider_key).copied();
        sqlx::query(
            "INSERT INTO speaker_voiceprints
             (speaker_id, embedding_space, provider_id, provider_key, provider_revision, dims,
              vector, revision, sample_count, browser_validation_status, enrolled_at, updated_at, calibration_revision)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
             ON CONFLICT(speaker_id, embedding_space) DO UPDATE SET
                 provider_id = excluded.provider_id,
                 provider_key = excluded.provider_key,
                 provider_revision = excluded.provider_revision,
                 dims = excluded.dims,
                 vector = excluded.vector,
                 revision = MAX(excluded.revision, speaker_voiceprints.revision + 1),
                 sample_count = excluded.sample_count,
                 browser_validation_status = excluded.browser_validation_status,
                 updated_at = excluded.updated_at,
                 calibration_revision = excluded.calibration_revision",
        )
        .bind(sp_id)
        .bind(&vp.embedding_space)
        .bind(provider_id)
        .bind(&vp.provider_key)
        .bind(vp.provider_revision.max(1))
        .bind(vp.dims)
        .bind(vector)
        .bind(vp.revision.max(1))
        .bind(vp.sample_count.max(1))
        .bind(&vp.browser_validation_status)
        .bind(vp.enrolled_at.max(1))
        .bind(now)
        .bind(&vp.calibration_revision)
        .execute(&mut **tx)
        .await?;
    }

    for pol in &bundle.agent_speaker_policies {
        let Some(&aid) = agent_map.get(&pol.agent_key) else {
            return Err(BackupError::InvalidReference(format!(
                "agent:{}",
                pol.agent_key
            )));
        };
        sqlx::query(
            "INSERT INTO agent_speaker_policies (agent_id, mode, revision)
             VALUES (?, ?, ?)
             ON CONFLICT(agent_id) DO UPDATE SET
                 mode = excluded.mode,
                 revision = MAX(excluded.revision, agent_speaker_policies.revision + 1)",
        )
        .bind(aid)
        .bind(&pol.mode)
        .bind(pol.revision.max(1))
        .execute(&mut **tx)
        .await?;
    }

    for cand in &bundle.agent_speaker_candidates {
        let Some(&aid) = agent_map.get(&cand.agent_key) else {
            return Err(BackupError::InvalidReference(format!(
                "agent:{}",
                cand.agent_key
            )));
        };
        let Some(&sp_id) = speaker_map.get(&cand.speaker_key) else {
            return Err(BackupError::InvalidReference(format!(
                "speaker:{}",
                cand.speaker_key
            )));
        };
        sqlx::query(
            "INSERT INTO agent_speaker_candidates (agent_id, speaker_id, created_at)
             VALUES (?, ?, ?)
             ON CONFLICT(agent_id, speaker_id) DO NOTHING",
        )
        .bind(aid)
        .bind(sp_id)
        .bind(cand.created_at.max(1))
        .execute(&mut **tx)
        .await?;
    }

    for grant in &bundle.agent_speaker_template_grants {
        let Some(&aid) = agent_map.get(&grant.agent_key) else {
            return Err(BackupError::InvalidReference(format!(
                "agent:{}",
                grant.agent_key
            )));
        };
        let Some(&sp_id) = speaker_map.get(&grant.speaker_key) else {
            return Err(BackupError::InvalidReference(format!(
                "speaker:{}",
                grant.speaker_key
            )));
        };
        let Some(&tid) = template_map.get(&grant.template_key) else {
            return Err(BackupError::InvalidReference(format!(
                "template:{}",
                grant.template_key
            )));
        };
        sqlx::query(
            "INSERT INTO agent_speaker_template_grants (agent_id, speaker_id, template_id, created_at)
             VALUES (?, ?, ?, ?)
             ON CONFLICT(agent_id, speaker_id, template_id) DO NOTHING",
        )
        .bind(aid)
        .bind(sp_id)
        .bind(tid)
        .bind(grant.created_at.max(1))
        .execute(&mut **tx)
        .await?;
    }

    if let Some(ref cat) = bundle.speaker_calibration_catalog {
        sqlx::query(
            "INSERT INTO speaker_calibration_catalog (id, calibration_revision, source_digest, profiles_json, published_at)
             VALUES (1, ?, ?, ?, ?)
             ON CONFLICT(id) DO UPDATE SET
                 calibration_revision = excluded.calibration_revision,
                 source_digest = excluded.source_digest,
                 profiles_json = excluded.profiles_json,
                 published_at = excluded.published_at",
        )
        .bind(&cat.calibration_revision)
        .bind(&cat.source_digest)
        .bind(&cat.profiles_json)
        .bind(cat.published_at)
        .execute(&mut **tx)
        .await?;
    }

    for ev in &bundle.speaker_calibration_evidence {
        sqlx::query(
            "INSERT INTO speaker_calibration_evidence
             (calibration_revision, candidate_set_digest, candidate_set_json, report_ref, created_at)
             VALUES (?, ?, ?, ?, ?)
             ON CONFLICT(calibration_revision, candidate_set_digest) DO UPDATE SET
                 candidate_set_json = excluded.candidate_set_json,
                 report_ref = excluded.report_ref",
        )
        .bind(&ev.calibration_revision)
        .bind(&ev.candidate_set_digest)
        .bind(&ev.candidate_set_json)
        .bind(&ev.report_ref)
        .bind(ev.created_at)
        .execute(&mut **tx)
        .await?;
    }

    Ok((speaker_map, speaker_catalog_touched))
}

pub(super) async fn upsert_gateway_and_system_settings(
    tx: &mut Transaction<'_, Sqlite>,
    bundle: &DatabaseBackupBundle,
    secret_resolver: &dyn SecretResolver,
    now: i64,
    now_ms: i64,
) -> Result<(), BackupError> {
    for acc in &bundle.llm_gateway_accounts {
        let (cred_json, status) = scrub_or_keep_gateway_credential(
            secret_resolver,
            &acc.account_key,
            &acc.credential_json,
            &acc.status,
        );
        sqlx::query(
            "INSERT INTO llm_gateway_accounts
             (account_key, vendor, label, account_email, credential_json, expires_at,
              enabled, priority, status, last_error, last_refreshed_at, last_used_at, created_at, updated_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
             ON CONFLICT(account_key) DO UPDATE SET
                 vendor = excluded.vendor,
                 label = excluded.label,
                 account_email = excluded.account_email,
                 credential_json = excluded.credential_json,
                 expires_at = excluded.expires_at,
                 enabled = excluded.enabled,
                 priority = excluded.priority,
                 status = excluded.status,
                 last_error = excluded.last_error,
                 last_refreshed_at = excluded.last_refreshed_at,
                 last_used_at = excluded.last_used_at,
                 updated_at = excluded.updated_at",
        )
        .bind(&acc.account_key)
        .bind(&acc.vendor)
        .bind(&acc.label)
        .bind(&acc.account_email)
        .bind(cred_json)
        .bind(acc.expires_at)
        .bind(acc.enabled)
        .bind(acc.priority)
        .bind(status)
        .bind(&acc.last_error)
        .bind(acc.last_refreshed_at)
        .bind(acc.last_used_at)
        .bind(acc.created_at)
        .bind(now)
        .execute(&mut **tx)
        .await?;
    }

    if let Some(ref ns) = bundle.notification_settings_state {
        sqlx::query(
            "INSERT INTO notification_settings_state (id, revision, web_ui_toast_enabled, retention_days, updated_at_ms)
             VALUES (1, 1, ?, ?, ?)
             ON CONFLICT(id) DO UPDATE SET
                 revision = notification_settings_state.revision + 1,
                 web_ui_toast_enabled = excluded.web_ui_toast_enabled,
                 retention_days = excluded.retention_days,
                 updated_at_ms = excluded.updated_at_ms",
        )
        .bind(ns.web_ui_toast_enabled)
        .bind(ns.retention_days)
        .bind(now_ms)
        .execute(&mut **tx)
        .await?;
    }

    for ch in &bundle.notification_channel_configs {
        let scope = format!("notification_channel:{}", ch.channel_kind);
        let cred = scrub_or_keep_optional_credential(
            secret_resolver,
            &scope,
            ch.encrypted_credential.as_deref(),
        );
        sqlx::query(
            "INSERT INTO notification_channel_configs
             (channel_kind, enabled, target_json, subscribed_topics_json, encrypted_credential, updated_at_ms)
             VALUES (?, ?, ?, ?, ?, ?)
             ON CONFLICT(channel_kind) DO UPDATE SET
                 enabled = excluded.enabled,
                 target_json = excluded.target_json,
                 subscribed_topics_json = excluded.subscribed_topics_json,
                 encrypted_credential = excluded.encrypted_credential,
                 updated_at_ms = excluded.updated_at_ms",
        )
        .bind(&ch.channel_kind)
        .bind(ch.enabled)
        .bind(&ch.target_json)
        .bind(&ch.subscribed_topics_json)
        .bind(cred)
        .bind(now_ms)
        .execute(&mut **tx)
        .await?;
    }

    if let Some(ref grants) = bundle.system_mcp_grants {
        sqlx::query(
            "INSERT INTO system_mcp_grants (id, system_read, mcp_read, mcp_discover, mcp_manage_request, revision)
             VALUES (1, ?, ?, ?, ?, 1)
             ON CONFLICT(id) DO UPDATE SET
                 system_read = excluded.system_read,
                 mcp_read = excluded.mcp_read,
                 mcp_discover = excluded.mcp_discover,
                 mcp_manage_request = excluded.mcp_manage_request,
                 revision = system_mcp_grants.revision + 1",
        )
        .bind(grants.system_read)
        .bind(grants.mcp_read)
        .bind(grants.mcp_discover)
        .bind(grants.mcp_manage_request)
        .execute(&mut **tx)
        .await?;
    }

    Ok(())
}

pub(super) async fn upsert_full_scope_runtime_tables(
    tx: &mut Transaction<'_, Sqlite>,
    bundle: &DatabaseBackupBundle,
    mode: ImportMode,
    agent_map: &HashMap<String, i64>,
    template_map: &HashMap<String, i64>,
    touched: &mut TouchedEntityIds,
) -> Result<(), BackupError> {
    let physical_device_map = load_scoped_id_map(
        tx,
        "SELECT device_id, id FROM devices WHERE kind = 'physical'",
        mode,
        bundle
            .devices
            .iter()
            .filter(|d| d.kind != "web_assistant")
            .map(|d| d.device_id.as_str()),
    )
    .await?;
    let web_assistant_row_id: Option<i64> =
        sqlx::query_scalar("SELECT id FROM devices WHERE kind = 'web_assistant'")
            .fetch_optional(&mut **tx)
            .await?;

    let resolve_device_row_id = |device_kind: &str, device_id: &str| -> Result<i64, BackupError> {
        if device_kind == "web_assistant" {
            web_assistant_row_id
                .ok_or_else(|| BackupError::InvalidReference("device:web_assistant".to_owned()))
        } else {
            physical_device_map
                .get(device_id)
                .copied()
                .or_else(|| {
                    if device_id.starts_with("web_assistant_") {
                        web_assistant_row_id
                    } else {
                        None
                    }
                })
                .ok_or_else(|| BackupError::InvalidReference(format!("device:{device_id}")))
        }
    };

    for r in &bundle.device_reminders {
        let dev_row_id = resolve_device_row_id(&r.device_kind, &r.device_id)?;
        touched.reminder_device_ids.insert(dev_row_id);
        sqlx::query(
            "INSERT INTO device_reminders
             (id, device_row_id, message, due_at_ms, timezone, status, row_revision,
              schedule_revision, source, attempts_used, next_attempt_at_ms,
              created_at_ms, updated_at_ms, completed_at_ms, cancelled_at_ms)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
             ON CONFLICT(id) DO UPDATE SET
                 device_row_id = excluded.device_row_id,
                 message = excluded.message,
                 due_at_ms = excluded.due_at_ms,
                 timezone = excluded.timezone,
                 status = excluded.status,
                 row_revision = MAX(device_reminders.row_revision + 1, excluded.row_revision),
                 schedule_revision = MAX(device_reminders.schedule_revision + 1, excluded.schedule_revision),
                 source = excluded.source,
                 attempts_used = excluded.attempts_used,
                 next_attempt_at_ms = excluded.next_attempt_at_ms,
                 updated_at_ms = excluded.updated_at_ms,
                 completed_at_ms = excluded.completed_at_ms,
                 cancelled_at_ms = excluded.cancelled_at_ms",
        )
        .bind(&r.id)
        .bind(dev_row_id)
        .bind(&r.message)
        .bind(r.due_at_ms)
        .bind(&r.timezone)
        .bind(&r.status)
        .bind(r.row_revision.max(1))
        .bind(r.schedule_revision.max(1))
        .bind(&r.source)
        .bind(r.attempts_used.max(0))
        .bind(r.next_attempt_at_ms)
        .bind(r.created_at_ms)
        .bind(r.updated_at_ms)
        .bind(r.completed_at_ms)
        .bind(r.cancelled_at_ms)
        .execute(&mut **tx)
        .await?;
    }

    for h in &bundle.history_messages {
        let dev_row_id = resolve_device_row_id(&h.device_kind, &h.device_id)?;
        let Some(&aid) = agent_map.get(&h.agent_key) else {
            return Err(BackupError::InvalidReference(format!(
                "agent:{}",
                h.agent_key
            )));
        };
        let tid = match h.template_key.as_deref() {
            Some(tk) => template_map.get(tk).copied(),
            None => None,
        };
        sqlx::query(
            "INSERT INTO history_messages
             (session_id, sequence, device_id, agent_id, template_id, turn_id, role, text, created_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)
             ON CONFLICT(session_id, sequence) DO UPDATE SET
                 device_id = excluded.device_id,
                 agent_id = excluded.agent_id,
                 template_id = excluded.template_id,
                 turn_id = excluded.turn_id,
                 role = excluded.role,
                 text = excluded.text,
                 created_at = excluded.created_at",
        )
        .bind(&h.session_id)
        .bind(h.sequence)
        .bind(dev_row_id)
        .bind(aid)
        .bind(tid)
        .bind(&h.turn_id)
        .bind(&h.role)
        .bind(&h.text)
        .bind(h.created_at)
        .execute(&mut **tx)
        .await?;
    }

    for item in &bundle.notifications_inbox {
        let existing_id: Option<String> = sqlx::query_scalar(
            "SELECT id FROM notifications_inbox WHERE id = ? OR (dedup_key IS NOT NULL AND dedup_key = ?)",
        )
        .bind(&item.id)
        .bind(&item.dedup_key)
        .fetch_optional(&mut **tx)
        .await?;

        if let Some(target_id) = existing_id {
            if let Some(ref dk) = item.dedup_key {
                sqlx::query("DELETE FROM notifications_inbox WHERE dedup_key = ? AND id != ?")
                    .bind(dk)
                    .bind(&target_id)
                    .execute(&mut **tx)
                    .await?;
            }
            sqlx::query(
                "UPDATE notifications_inbox
                 SET dedup_key = ?, topic = ?, severity = ?, title = ?, body = ?,
                     source_kind = ?, source_id = ?, metadata_json = ?, is_read = ?,
                     read_at_ms = ?, created_at_ms = ?
                 WHERE id = ?",
            )
            .bind(&item.dedup_key)
            .bind(&item.topic)
            .bind(&item.severity)
            .bind(&item.title)
            .bind(&item.body)
            .bind(&item.source_kind)
            .bind(&item.source_id)
            .bind(&item.metadata_json)
            .bind(item.is_read)
            .bind(item.read_at_ms)
            .bind(item.created_at_ms)
            .bind(&target_id)
            .execute(&mut **tx)
            .await?;
        } else {
            sqlx::query(
                "INSERT INTO notifications_inbox
                 (id, dedup_key, topic, severity, title, body, source_kind, source_id,
                  metadata_json, is_read, read_at_ms, created_at_ms)
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(&item.id)
            .bind(&item.dedup_key)
            .bind(&item.topic)
            .bind(&item.severity)
            .bind(&item.title)
            .bind(&item.body)
            .bind(&item.source_kind)
            .bind(&item.source_id)
            .bind(&item.metadata_json)
            .bind(item.is_read)
            .bind(item.read_at_ms)
            .bind(item.created_at_ms)
            .execute(&mut **tx)
            .await?;
        }
    }

    Ok(())
}
