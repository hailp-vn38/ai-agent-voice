use super::{super::*, TouchedEntityIds};
use sqlx::{Row, Sqlite, Transaction};
use std::collections::{HashMap, HashSet};

#[allow(clippy::too_many_arguments)]
pub(super) async fn apply_replace_deletions(
    tx: &mut Transaction<'_, Sqlite>,
    bundle: &DatabaseBackupBundle,
    effective_scope: BackupScope,
    agent_map: &HashMap<String, i64>,
    server_map: &HashMap<String, i64>,
    speaker_map: &HashMap<String, i64>,
    template_map: &HashMap<String, i64>,
    now: i64,
    touched: &mut TouchedEntityIds,
) -> Result<(), BackupError> {
    if effective_scope == BackupScope::Full {
        prune_runtime_tables(tx, bundle, touched).await?;
    }

    prune_speakers_and_subtables(tx, bundle, agent_map, speaker_map, template_map, touched).await?;
    prune_physical_devices(tx, bundle).await?;
    prune_mcp_servers_and_subtables(tx, bundle, agent_map, server_map, touched).await?;
    prune_agents_templates_and_providers(tx, bundle, agent_map, template_map, now, touched).await?;

    Ok(())
}

async fn prune_runtime_tables(
    tx: &mut Transaction<'_, Sqlite>,
    bundle: &DatabaseBackupBundle,
    touched: &mut TouchedEntityIds,
) -> Result<(), BackupError> {
    let keep_inbox_ids: HashSet<&str> = bundle
        .notifications_inbox
        .iter()
        .map(|i| i.id.as_str())
        .collect();
    let keep_inbox_dedups: HashSet<&str> = bundle
        .notifications_inbox
        .iter()
        .filter_map(|i| i.dedup_key.as_deref())
        .collect();
    let existing_inbox = sqlx::query("SELECT id, dedup_key FROM notifications_inbox")
        .fetch_all(&mut **tx)
        .await?;
    for row in existing_inbox {
        let id: String = row.get("id");
        let dedup: Option<String> = row.get("dedup_key");
        let keep = keep_inbox_ids.contains(id.as_str())
            || dedup
                .as_deref()
                .is_some_and(|dk| keep_inbox_dedups.contains(dk));
        if !keep {
            sqlx::query("DELETE FROM notifications_inbox WHERE id = ?")
                .bind(&id)
                .execute(&mut **tx)
                .await?;
        }
    }

    let keep_history: HashSet<(&str, i64)> = bundle
        .history_messages
        .iter()
        .map(|h| (h.session_id.as_str(), h.sequence))
        .collect();
    let existing_history = sqlx::query("SELECT id, session_id, sequence FROM history_messages")
        .fetch_all(&mut **tx)
        .await?;
    for row in existing_history {
        let id: i64 = row.get("id");
        let sid: String = row.get("session_id");
        let seq: i64 = row.get("sequence");
        if !keep_history.contains(&(sid.as_str(), seq)) {
            sqlx::query("DELETE FROM history_messages WHERE id = ?")
                .bind(id)
                .execute(&mut **tx)
                .await?;
        }
    }

    let keep_reminders: HashSet<&str> = bundle
        .device_reminders
        .iter()
        .map(|r| r.id.as_str())
        .collect();
    let existing_reminders = sqlx::query("SELECT id, device_row_id FROM device_reminders")
        .fetch_all(&mut **tx)
        .await?;
    for row in existing_reminders {
        let id: String = row.get("id");
        let dev_row_id: i64 = row.get("device_row_id");
        if !keep_reminders.contains(id.as_str()) {
            touched.reminder_device_ids.insert(dev_row_id);
            sqlx::query("DELETE FROM device_reminders WHERE id = ?")
                .bind(&id)
                .execute(&mut **tx)
                .await?;
        }
    }

    Ok(())
}

async fn prune_speakers_and_subtables(
    tx: &mut Transaction<'_, Sqlite>,
    bundle: &DatabaseBackupBundle,
    agent_map: &HashMap<String, i64>,
    speaker_map: &HashMap<String, i64>,
    template_map: &HashMap<String, i64>,
    touched: &mut TouchedEntityIds,
) -> Result<(), BackupError> {
    let keep_voiceprints: HashSet<(i64, &str)> = bundle
        .speaker_voiceprints
        .iter()
        .filter_map(|vp| {
            speaker_map
                .get(&vp.speaker_key)
                .map(|&sp_id| (sp_id, vp.embedding_space.as_str()))
        })
        .collect();
    let existing_vps = sqlx::query("SELECT speaker_id, embedding_space FROM speaker_voiceprints")
        .fetch_all(&mut **tx)
        .await?;
    for row in existing_vps {
        let sp_id: i64 = row.get("speaker_id");
        let space: String = row.get("embedding_space");
        if !keep_voiceprints.contains(&(sp_id, space.as_str())) {
            touched.speaker_ids.insert(sp_id);
            sqlx::query(
                "DELETE FROM speaker_voiceprints WHERE speaker_id = ? AND embedding_space = ?",
            )
            .bind(sp_id)
            .bind(&space)
            .execute(&mut **tx)
            .await?;
        }
    }

    let keep_candidates: HashSet<(i64, i64)> = bundle
        .agent_speaker_candidates
        .iter()
        .filter_map(|c| {
            Some((
                *agent_map.get(&c.agent_key)?,
                *speaker_map.get(&c.speaker_key)?,
            ))
        })
        .collect();
    let existing_candidates =
        sqlx::query("SELECT agent_id, speaker_id FROM agent_speaker_candidates")
            .fetch_all(&mut **tx)
            .await?;
    for row in existing_candidates {
        let aid: i64 = row.get("agent_id");
        let sp_id: i64 = row.get("speaker_id");
        if !keep_candidates.contains(&(aid, sp_id)) {
            touched.agent_ids.insert(aid);
            touched.speaker_ids.insert(sp_id);
            sqlx::query(
                "DELETE FROM agent_speaker_candidates WHERE agent_id = ? AND speaker_id = ?",
            )
            .bind(aid)
            .bind(sp_id)
            .execute(&mut **tx)
            .await?;
        }
    }

    let keep_grants: HashSet<(i64, i64, i64)> = bundle
        .agent_speaker_template_grants
        .iter()
        .filter_map(|g| {
            Some((
                *agent_map.get(&g.agent_key)?,
                *speaker_map.get(&g.speaker_key)?,
                *template_map.get(&g.template_key)?,
            ))
        })
        .collect();
    let existing_grants =
        sqlx::query("SELECT agent_id, speaker_id, template_id FROM agent_speaker_template_grants")
            .fetch_all(&mut **tx)
            .await?;
    for row in existing_grants {
        let aid: i64 = row.get("agent_id");
        let sp_id: i64 = row.get("speaker_id");
        let tid: i64 = row.get("template_id");
        if !keep_grants.contains(&(aid, sp_id, tid)) {
            touched.agent_ids.insert(aid);
            touched.speaker_ids.insert(sp_id);
            touched.template_ids.insert(tid);
            sqlx::query(
                "DELETE FROM agent_speaker_template_grants WHERE agent_id = ? AND speaker_id = ? AND template_id = ?",
            )
            .bind(aid)
            .bind(sp_id)
            .bind(tid)
            .execute(&mut **tx)
            .await?;
        }
    }

    let keep_policies: HashSet<i64> = bundle
        .agent_speaker_policies
        .iter()
        .filter_map(|p| agent_map.get(&p.agent_key).copied())
        .collect();
    let existing_policies: Vec<i64> =
        sqlx::query_scalar("SELECT agent_id FROM agent_speaker_policies")
            .fetch_all(&mut **tx)
            .await?;
    for aid in existing_policies {
        if !keep_policies.contains(&aid) {
            touched.agent_ids.insert(aid);
            sqlx::query("DELETE FROM agent_speaker_policies WHERE agent_id = ?")
                .bind(aid)
                .execute(&mut **tx)
                .await?;
        }
    }

    if bundle.speaker_calibration_catalog.is_none() {
        sqlx::query("DELETE FROM speaker_calibration_catalog")
            .execute(&mut **tx)
            .await?;
    }
    let keep_evidence: HashSet<(&str, &str)> = bundle
        .speaker_calibration_evidence
        .iter()
        .map(|e| {
            (
                e.calibration_revision.as_str(),
                e.candidate_set_digest.as_str(),
            )
        })
        .collect();
    let existing_evidence = sqlx::query(
        "SELECT calibration_revision, candidate_set_digest FROM speaker_calibration_evidence",
    )
    .fetch_all(&mut **tx)
    .await?;
    for row in existing_evidence {
        let rev: String = row.get("calibration_revision");
        let dig: String = row.get("candidate_set_digest");
        if !keep_evidence.contains(&(rev.as_str(), dig.as_str())) {
            sqlx::query(
                "DELETE FROM speaker_calibration_evidence WHERE calibration_revision = ? AND candidate_set_digest = ?",
            )
            .bind(&rev)
            .bind(&dig)
            .execute(&mut **tx)
            .await?;
        }
    }

    let keep_speakers: HashSet<&str> = bundle.speakers.iter().map(|s| s.key.as_str()).collect();
    let existing_speakers = sqlx::query("SELECT id, key FROM speakers")
        .fetch_all(&mut **tx)
        .await?;
    for row in existing_speakers {
        let sp_id: i64 = row.get("id");
        let key: String = row.get("key");
        if !keep_speakers.contains(key.as_str()) {
            touched.speaker_ids.insert(sp_id);
            sqlx::query("DELETE FROM speaker_quick_captures WHERE speaker_id = ?")
                .bind(sp_id)
                .execute(&mut **tx)
                .await?;
            sqlx::query(
                "DELETE FROM speaker_enrollment_drafts WHERE speaker_id = ? OR committed_speaker_id = ?",
            )
            .bind(sp_id)
            .bind(sp_id)
            .execute(&mut **tx)
            .await?;
            sqlx::query("DELETE FROM speakers WHERE id = ?")
                .bind(sp_id)
                .execute(&mut **tx)
                .await?;
        }
    }

    Ok(())
}

async fn prune_physical_devices(
    tx: &mut Transaction<'_, Sqlite>,
    bundle: &DatabaseBackupBundle,
) -> Result<(), BackupError> {
    let keep_physical_devices: HashSet<&str> = bundle
        .devices
        .iter()
        .filter(|d| d.kind != "web_assistant")
        .map(|d| d.device_id.as_str())
        .collect();
    let existing_devices =
        sqlx::query("SELECT id, device_id FROM devices WHERE kind = 'physical'")
            .fetch_all(&mut **tx)
            .await?;
    for row in existing_devices {
        let id: i64 = row.get("id");
        let dev_id: String = row.get("device_id");
        if !keep_physical_devices.contains(dev_id.as_str()) {
            sqlx::query("DELETE FROM devices WHERE id = ?")
                .bind(id)
                .execute(&mut **tx)
                .await?;
        }
    }
    Ok(())
}

async fn prune_mcp_servers_and_subtables(
    tx: &mut Transaction<'_, Sqlite>,
    bundle: &DatabaseBackupBundle,
    agent_map: &HashMap<String, i64>,
    server_map: &HashMap<String, i64>,
    touched: &mut TouchedEntityIds,
) -> Result<(), BackupError> {
    let keep_allowlist: HashSet<(i64, i64, &str)> = bundle
        .agent_external_tool_allowlist
        .iter()
        .filter_map(|al| {
            Some((
                *agent_map.get(&al.agent_key)?,
                *server_map.get(&al.server_key)?,
                al.original_name.as_str(),
            ))
        })
        .collect();
    let existing_allowlist =
        sqlx::query("SELECT agent_id, server_id, original_name FROM agent_external_tool_allowlist")
            .fetch_all(&mut **tx)
            .await?;
    for row in existing_allowlist {
        let aid: i64 = row.get("agent_id");
        let sid: i64 = row.get("server_id");
        let name: String = row.get("original_name");
        if !keep_allowlist.contains(&(aid, sid, name.as_str())) {
            touched.agent_ids.insert(aid);
            touched.server_ids.insert(sid);
            sqlx::query(
                "DELETE FROM agent_external_tool_allowlist WHERE agent_id = ? AND server_id = ? AND original_name = ?",
            )
            .bind(aid)
            .bind(sid)
            .bind(&name)
            .execute(&mut **tx)
            .await?;
        }
    }

    let keep_observations: HashSet<(i64, &str)> = bundle
        .external_tool_observations
        .iter()
        .filter_map(|obs| {
            Some((
                *server_map.get(&obs.server_key)?,
                obs.original_name.as_str(),
            ))
        })
        .collect();
    let existing_observations =
        sqlx::query("SELECT server_id, original_name FROM external_tool_observations")
            .fetch_all(&mut **tx)
            .await?;
    for row in existing_observations {
        let sid: i64 = row.get("server_id");
        let name: String = row.get("original_name");
        if !keep_observations.contains(&(sid, name.as_str())) {
            touched.server_ids.insert(sid);
            sqlx::query(
                "DELETE FROM external_tool_observations WHERE server_id = ? AND original_name = ?",
            )
            .bind(sid)
            .bind(&name)
            .execute(&mut **tx)
            .await?;
        }
    }

    let keep_mcp_bindings: HashSet<(i64, i64)> = bundle
        .agent_mcp_bindings
        .iter()
        .filter_map(|b| {
            Some((
                *agent_map.get(&b.agent_key)?,
                *server_map.get(&b.server_key)?,
            ))
        })
        .collect();
    let existing_mcp_bindings =
        sqlx::query("SELECT agent_id, mcp_server_id FROM agent_mcp_bindings")
            .fetch_all(&mut **tx)
            .await?;
    for row in existing_mcp_bindings {
        let aid: i64 = row.get("agent_id");
        let sid: i64 = row.get("mcp_server_id");
        if !keep_mcp_bindings.contains(&(aid, sid)) {
            touched.agent_ids.insert(aid);
            touched.server_ids.insert(sid);
            sqlx::query(
                "DELETE FROM agent_mcp_bindings WHERE agent_id = ? AND mcp_server_id = ?",
            )
            .bind(aid)
            .bind(sid)
            .execute(&mut **tx)
            .await?;
        }
    }

    let keep_servers: HashSet<&str> = bundle
        .mcp_servers
        .iter()
        .map(|s| s.key.as_str())
        .collect();
    let existing_servers = sqlx::query("SELECT id, key FROM mcp_servers")
        .fetch_all(&mut **tx)
        .await?;
    for row in existing_servers {
        let sid: i64 = row.get("id");
        let key: String = row.get("key");
        if !keep_servers.contains(key.as_str()) {
            touched.server_ids.insert(sid);
            sqlx::query("DELETE FROM mcp_servers WHERE id = ?")
                .bind(sid)
                .execute(&mut **tx)
                .await?;
        }
    }

    Ok(())
}

async fn prune_agents_templates_and_providers(
    tx: &mut Transaction<'_, Sqlite>,
    bundle: &DatabaseBackupBundle,
    agent_map: &HashMap<String, i64>,
    template_map: &HashMap<String, i64>,
    now: i64,
    touched: &mut TouchedEntityIds,
) -> Result<(), BackupError> {
    let keep_agents: HashSet<&str> = bundle.agents.iter().map(|a| a.key.as_str()).collect();
    let kept_agent_ids: HashSet<i64> = bundle
        .agents
        .iter()
        .filter_map(|a| agent_map.get(&a.key).copied())
        .collect();

    if let Some((wa_id, wa_agent_id, wa_template_id)) = sqlx::query_as::<_, (i64, i64, Option<i64>)>(
        "SELECT id, agent_id, template_id FROM devices WHERE kind = 'web_assistant'",
    )
    .fetch_optional(&mut **tx)
    .await?
    {
        let keep_template_ids: HashSet<i64> = bundle
            .agent_templates
            .iter()
            .filter_map(|t| template_map.get(&t.key).copied())
            .collect();
        let new_wa_template_id = wa_template_id.filter(|tid| keep_template_ids.contains(tid));

        if !kept_agent_ids.contains(&wa_agent_id) {
            let fallback_aid = bundle
                .agents
                .first()
                .and_then(|a| agent_map.get(&a.key).copied())
                .ok_or_else(|| {
                    BackupError::InvalidReference(
                        "web_assistant requires at least one agent in replace mode".to_owned(),
                    )
                })?;
            sqlx::query(
                "UPDATE devices
                 SET agent_id = ?, template_id = ?, revision = revision + 1, updated_at = ?
                 WHERE id = ?",
            )
            .bind(fallback_aid)
            .bind(new_wa_template_id)
            .bind(now)
            .bind(wa_id)
            .execute(&mut **tx)
            .await?;
        } else if new_wa_template_id != wa_template_id {
            sqlx::query(
                "UPDATE devices
                 SET template_id = ?, revision = revision + 1, updated_at = ?
                 WHERE id = ?",
            )
            .bind(new_wa_template_id)
            .bind(now)
            .bind(wa_id)
            .execute(&mut **tx)
            .await?;
        }
    }

    let existing_agents = sqlx::query("SELECT id, key FROM agents")
        .fetch_all(&mut **tx)
        .await?;
    for row in existing_agents {
        let aid: i64 = row.get("id");
        let key: String = row.get("key");
        if !keep_agents.contains(key.as_str()) {
            touched.agent_ids.insert(aid);
            sqlx::query("DELETE FROM history_messages WHERE agent_id = ?")
                .bind(aid)
                .execute(&mut **tx)
                .await?;
            sqlx::query("DELETE FROM agents WHERE id = ?")
                .bind(aid)
                .execute(&mut **tx)
                .await?;
        }
    }

    let keep_templates: HashSet<&str> = bundle
        .agent_templates
        .iter()
        .map(|t| t.key.as_str())
        .collect();
    let existing_templates = sqlx::query("SELECT id, key FROM agent_templates")
        .fetch_all(&mut **tx)
        .await?;
    for row in existing_templates {
        let tid: i64 = row.get("id");
        let key: String = row.get("key");
        if !keep_templates.contains(key.as_str()) {
            touched.template_ids.insert(tid);
            sqlx::query("DELETE FROM template_provider_bindings WHERE template_id = ?")
                .bind(tid)
                .execute(&mut **tx)
                .await?;
            sqlx::query("DELETE FROM agent_template_assignments WHERE template_id = ?")
                .bind(tid)
                .execute(&mut **tx)
                .await?;
            sqlx::query("DELETE FROM agent_templates WHERE id = ?")
                .bind(tid)
                .execute(&mut **tx)
                .await?;
        }
    }

    let keep_providers: HashSet<&str> = bundle.providers.iter().map(|p| p.key.as_str()).collect();
    let existing_providers = sqlx::query("SELECT id, key FROM providers")
        .fetch_all(&mut **tx)
        .await?;
    for row in existing_providers {
        let pid: i64 = row.get("id");
        let key: String = row.get("key");
        if !keep_providers.contains(key.as_str()) {
            sqlx::query("DELETE FROM providers WHERE id = ?")
                .bind(pid)
                .execute(&mut **tx)
                .await?;
        }
    }

    let keep_gateway: HashSet<&str> = bundle
        .llm_gateway_accounts
        .iter()
        .map(|a| a.account_key.as_str())
        .collect();
    let existing_gateway = sqlx::query("SELECT id, account_key FROM llm_gateway_accounts")
        .fetch_all(&mut **tx)
        .await?;
    for row in existing_gateway {
        let id: i64 = row.get("id");
        let key: String = row.get("account_key");
        if !keep_gateway.contains(key.as_str()) {
            sqlx::query("DELETE FROM llm_gateway_accounts WHERE id = ?")
                .bind(id)
                .execute(&mut **tx)
                .await?;
        }
    }

    let keep_channels: HashSet<&str> = bundle
        .notification_channel_configs
        .iter()
        .map(|c| c.channel_kind.as_str())
        .collect();
    let existing_channels: Vec<String> =
        sqlx::query_scalar("SELECT channel_kind FROM notification_channel_configs")
            .fetch_all(&mut **tx)
            .await?;
    for kind in existing_channels {
        if !keep_channels.contains(kind.as_str()) {
            sqlx::query("DELETE FROM notification_channel_configs WHERE channel_kind = ?")
                .bind(&kind)
                .execute(&mut **tx)
                .await?;
        }
    }

    Ok(())
}
