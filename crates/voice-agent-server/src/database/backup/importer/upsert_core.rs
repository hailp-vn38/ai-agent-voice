use super::{
    super::{inspector::scrub_or_keep_optional_credential, *},
    TouchedEntityIds, load_scoped_id_map,
};
use crate::database::secrets::SecretResolver;
use sqlx::{Sqlite, Transaction};
use std::collections::{HashMap, HashSet};

pub(super) async fn upsert_providers_and_templates(
    tx: &mut Transaction<'_, Sqlite>,
    bundle: &DatabaseBackupBundle,
    mode: ImportMode,
    secret_resolver: &dyn SecretResolver,
    now: i64,
    touched: &mut TouchedEntityIds,
) -> Result<(HashMap<String, i64>, HashMap<String, i64>), BackupError> {
    for p in &bundle.providers {
        let scope = format!("provider:{}", p.key);
        let cred = scrub_or_keep_optional_credential(
            secret_resolver,
            &scope,
            p.credential_json.as_deref(),
        );
        let existing: Option<(i64, i64)> =
            sqlx::query_as("SELECT id, revision FROM providers WHERE key = ?")
                .bind(&p.key)
                .fetch_optional(&mut **tx)
                .await?;
        if let Some((id, rev)) = existing {
            sqlx::query(
                "UPDATE providers
                 SET name = ?, type = ?, adapter = ?, config_json = ?, credential_json = ?,
                     enabled = ?, revision = ?, updated_at = ?
                 WHERE id = ?",
            )
            .bind(&p.name)
            .bind(&p.provider_type)
            .bind(&p.adapter)
            .bind(&p.config_json)
            .bind(cred)
            .bind(p.enabled)
            .bind(rev + 1)
            .bind(now)
            .bind(id)
            .execute(&mut **tx)
            .await?;
        } else {
            sqlx::query(
                "INSERT INTO providers (key, name, type, adapter, config_json, credential_json, enabled, revision, created_at, updated_at)
                 VALUES (?, ?, ?, ?, ?, ?, ?, 1, ?, ?)",
            )
            .bind(&p.key)
            .bind(&p.name)
            .bind(&p.provider_type)
            .bind(&p.adapter)
            .bind(&p.config_json)
            .bind(cred)
            .bind(p.enabled)
            .bind(p.created_at.max(1))
            .bind(now)
            .execute(&mut **tx)
            .await?;
        }
    }

    let provider_map = load_scoped_id_map(
        tx,
        "SELECT key, id FROM providers",
        mode,
        bundle.providers.iter().map(|p| p.key.as_str()),
    )
    .await?;

    for t in &bundle.agent_templates {
        let existing: Option<(i64, i64)> =
            sqlx::query_as("SELECT id, revision FROM agent_templates WHERE key = ?")
                .bind(&t.key)
                .fetch_optional(&mut **tx)
                .await?;
        let tid = if let Some((id, rev)) = existing {
            sqlx::query(
                "UPDATE agent_templates
                 SET name = ?, description = ?, language = ?, prompt = ?, enabled = ?,
                     revision = ?, updated_at = ?
                 WHERE id = ?",
            )
            .bind(&t.name)
            .bind(&t.description)
            .bind(&t.language)
            .bind(&t.prompt)
            .bind(t.enabled)
            .bind(rev + 1)
            .bind(now)
            .bind(id)
            .execute(&mut **tx)
            .await?;
            id
        } else {
            sqlx::query(
                "INSERT INTO agent_templates (key, name, description, language, prompt, enabled, revision, created_at, updated_at)
                 VALUES (?, ?, ?, ?, ?, ?, 1, ?, ?)",
            )
            .bind(&t.key)
            .bind(&t.name)
            .bind(&t.description)
            .bind(&t.language)
            .bind(&t.prompt)
            .bind(t.enabled)
            .bind(t.created_at.max(1))
            .bind(now)
            .execute(&mut **tx)
            .await?
            .last_insert_rowid()
        };
        touched.template_ids.insert(tid);
    }

    let template_map = load_scoped_id_map(
        tx,
        "SELECT key, id FROM agent_templates",
        mode,
        bundle.agent_templates.iter().map(|t| t.key.as_str()),
    )
    .await?;

    for t in &bundle.agent_templates {
        if let Some(&tid) = template_map.get(&t.key) {
            sqlx::query("DELETE FROM template_provider_bindings WHERE template_id = ?")
                .bind(tid)
                .execute(&mut **tx)
                .await?;
        }
    }
    for b in &bundle.template_provider_bindings {
        let Some(&tid) = template_map.get(&b.template_key) else {
            return Err(BackupError::InvalidReference(format!(
                "template:{}",
                b.template_key
            )));
        };
        let (provider_id, config_key) = if b.provider_source == "database" {
            let pkey = b.provider_key.as_deref().ok_or_else(|| {
                BackupError::InvalidReference(format!(
                    "template_provider_binding:{}:{}",
                    b.template_key, b.provider_type
                ))
            })?;
            let &pid = provider_map.get(pkey).ok_or_else(|| {
                BackupError::InvalidReference(format!("provider:{pkey}"))
            })?;
            (Some(pid), None)
        } else {
            (None, b.config_provider_key.clone())
        };
        sqlx::query(
            "INSERT INTO template_provider_bindings
             (template_id, provider_type, provider_source, provider_id, config_provider_key, created_at, updated_at)
             VALUES (?, ?, ?, ?, ?, ?, ?)
             ON CONFLICT(template_id, provider_type) DO UPDATE SET
                 provider_source = excluded.provider_source,
                 provider_id = excluded.provider_id,
                 config_provider_key = excluded.config_provider_key,
                 updated_at = excluded.updated_at",
        )
        .bind(tid)
        .bind(&b.provider_type)
        .bind(&b.provider_source)
        .bind(provider_id)
        .bind(config_key)
        .bind(b.created_at.max(1))
        .bind(now)
        .execute(&mut **tx)
        .await?;
    }

    Ok((provider_map, template_map))
}

pub(super) async fn upsert_agents_and_assignments(
    tx: &mut Transaction<'_, Sqlite>,
    bundle: &DatabaseBackupBundle,
    mode: ImportMode,
    template_map: &HashMap<String, i64>,
    now: i64,
    touched: &mut TouchedEntityIds,
) -> Result<HashMap<String, i64>, BackupError> {
    for a in &bundle.agents {
        let existing: Option<(i64, i64, i64)> = sqlx::query_as(
            "SELECT id, revision, reminder_policy_revision FROM agents WHERE key = ?",
        )
        .bind(&a.key)
        .fetch_optional(&mut **tx)
        .await?;
        let aid = if let Some((id, rev, rem_rev)) = existing {
            sqlx::query(
                "UPDATE agents
                 SET name = ?, description = ?, enabled = ?, reminders_enabled = ?,
                     revision = ?, reminder_policy_revision = ?, updated_at = ?
                 WHERE id = ?",
            )
            .bind(&a.name)
            .bind(&a.description)
            .bind(a.enabled)
            .bind(a.reminders_enabled)
            .bind(rev + 1)
            .bind(rem_rev + 1)
            .bind(now)
            .bind(id)
            .execute(&mut **tx)
            .await?;
            id
        } else {
            sqlx::query(
                "INSERT INTO agents (key, name, description, enabled, reminders_enabled, revision, reminder_policy_revision, created_at, updated_at)
                 VALUES (?, ?, ?, ?, ?, 1, 1, ?, ?)",
            )
            .bind(&a.key)
            .bind(&a.name)
            .bind(&a.description)
            .bind(a.enabled)
            .bind(a.reminders_enabled)
            .bind(a.created_at.max(1))
            .bind(now)
            .execute(&mut **tx)
            .await?
            .last_insert_rowid()
        };
        touched.agent_ids.insert(aid);
    }

    let agent_map = load_scoped_id_map(
        tx,
        "SELECT key, id FROM agents",
        mode,
        bundle.agents.iter().map(|a| a.key.as_str()),
    )
    .await?;

    for a in &bundle.agents {
        if let Some(&aid) = agent_map.get(&a.key) {
            sqlx::query("DELETE FROM agent_template_assignments WHERE agent_id = ?")
                .bind(aid)
                .execute(&mut **tx)
                .await?;
        }
    }
    let mut seen_default_for_agent = HashSet::new();
    for ata in &bundle.agent_template_assignments {
        let Some(&aid) = agent_map.get(&ata.agent_key) else {
            return Err(BackupError::InvalidReference(format!(
                "agent:{}",
                ata.agent_key
            )));
        };
        let Some(&tid) = template_map.get(&ata.template_key) else {
            return Err(BackupError::InvalidReference(format!(
                "template:{}",
                ata.template_key
            )));
        };
        let is_default = if ata.is_default == 1 && ata.enabled == 1 {
            if seen_default_for_agent.insert(aid) {
                1
            } else {
                0
            }
        } else {
            ata.is_default
        };
        sqlx::query(
            "INSERT INTO agent_template_assignments (agent_id, template_id, is_default, enabled, created_at)
             VALUES (?, ?, ?, ?, ?)
             ON CONFLICT(agent_id, template_id) DO UPDATE SET
                 is_default = excluded.is_default,
                 enabled = excluded.enabled",
        )
        .bind(aid)
        .bind(tid)
        .bind(is_default)
        .bind(ata.enabled)
        .bind(ata.created_at.max(1))
        .execute(&mut **tx)
        .await?;
    }

    Ok(agent_map)
}

#[allow(clippy::too_many_arguments)]
pub(super) async fn upsert_mcp_and_devices(
    tx: &mut Transaction<'_, Sqlite>,
    bundle: &DatabaseBackupBundle,
    mode: ImportMode,
    agent_map: &HashMap<String, i64>,
    template_map: &HashMap<String, i64>,
    secret_resolver: &dyn SecretResolver,
    now: i64,
    touched: &mut TouchedEntityIds,
) -> Result<HashMap<String, i64>, BackupError> {
    for s in &bundle.mcp_servers {
        let scope = format!("mcp:{}", s.key);
        let cred = scrub_or_keep_optional_credential(
            secret_resolver,
            &scope,
            s.credential_json.as_deref(),
        );
        let existing: Option<(i64, i64)> =
            sqlx::query_as("SELECT id, revision FROM mcp_servers WHERE key = ?")
                .bind(&s.key)
                .fetch_optional(&mut **tx)
                .await?;
        let sid = if let Some((id, rev)) = existing {
            sqlx::query(
                "UPDATE mcp_servers
                 SET name = ?, transport = ?, url = ?, headers_json = ?, auth_type = ?,
                     auth_header_name = ?, connect_timeout_ms = ?, request_timeout_ms = ?,
                     enabled = ?, credential_json = ?, revision = ?, updated_at = ?
                 WHERE id = ?",
            )
            .bind(&s.name)
            .bind(&s.transport)
            .bind(&s.url)
            .bind(&s.headers_json)
            .bind(&s.auth_type)
            .bind(&s.auth_header_name)
            .bind(s.connect_timeout_ms)
            .bind(s.request_timeout_ms)
            .bind(s.enabled)
            .bind(cred)
            .bind(rev + 1)
            .bind(now)
            .bind(id)
            .execute(&mut **tx)
            .await?;
            id
        } else {
            sqlx::query(
                "INSERT INTO mcp_servers
                 (key, name, transport, url, headers_json, auth_type, auth_header_name,
                  connect_timeout_ms, request_timeout_ms, enabled, credential_json, revision, created_at, updated_at)
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, 1, ?, ?)",
            )
            .bind(&s.key)
            .bind(&s.name)
            .bind(&s.transport)
            .bind(&s.url)
            .bind(&s.headers_json)
            .bind(&s.auth_type)
            .bind(&s.auth_header_name)
            .bind(s.connect_timeout_ms)
            .bind(s.request_timeout_ms)
            .bind(s.enabled)
            .bind(cred)
            .bind(s.created_at.max(1))
            .bind(now)
            .execute(&mut **tx)
            .await?
            .last_insert_rowid()
        };
        touched.server_ids.insert(sid);
    }

    let server_map = load_scoped_id_map(
        tx,
        "SELECT key, id FROM mcp_servers",
        mode,
        bundle.mcp_servers.iter().map(|s| s.key.as_str()),
    )
    .await?;

    for b in &bundle.agent_mcp_bindings {
        let Some(&aid) = agent_map.get(&b.agent_key) else {
            return Err(BackupError::InvalidReference(format!(
                "agent:{}",
                b.agent_key
            )));
        };
        let Some(&sid) = server_map.get(&b.server_key) else {
            return Err(BackupError::InvalidReference(format!(
                "mcp_server:{}",
                b.server_key
            )));
        };
        sqlx::query(
            "INSERT INTO agent_mcp_bindings (agent_id, mcp_server_id, enabled, required, created_at)
             VALUES (?, ?, ?, ?, ?)
             ON CONFLICT(agent_id, mcp_server_id) DO UPDATE SET
                 enabled = excluded.enabled,
                 required = excluded.required",
        )
        .bind(aid)
        .bind(sid)
        .bind(b.enabled)
        .bind(b.required)
        .bind(b.created_at.max(1))
        .execute(&mut **tx)
        .await?;
    }

    for obs in &bundle.external_tool_observations {
        let Some(&sid) = server_map.get(&obs.server_key) else {
            return Err(BackupError::InvalidReference(format!(
                "mcp_server:{}",
                obs.server_key
            )));
        };
        sqlx::query(
            "INSERT INTO external_tool_observations
             (server_id, original_name, description, input_schema, fingerprint, server_revision, blocked, revision, observed_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)
             ON CONFLICT(server_id, original_name) DO UPDATE SET
                 description = excluded.description,
                 input_schema = excluded.input_schema,
                 fingerprint = excluded.fingerprint,
                 server_revision = excluded.server_revision,
                 blocked = excluded.blocked,
                 revision = MAX(excluded.revision, external_tool_observations.revision + 1),
                 observed_at = excluded.observed_at",
        )
        .bind(sid)
        .bind(&obs.original_name)
        .bind(&obs.description)
        .bind(&obs.input_schema)
        .bind(&obs.fingerprint)
        .bind(obs.server_revision.max(1))
        .bind(obs.blocked)
        .bind(obs.revision.max(1))
        .bind(obs.observed_at.max(1))
        .execute(&mut **tx)
        .await?;
    }

    for al in &bundle.agent_external_tool_allowlist {
        let Some(&aid) = agent_map.get(&al.agent_key) else {
            return Err(BackupError::InvalidReference(format!(
                "agent:{}",
                al.agent_key
            )));
        };
        let Some(&sid) = server_map.get(&al.server_key) else {
            return Err(BackupError::InvalidReference(format!(
                "mcp_server:{}",
                al.server_key
            )));
        };
        sqlx::query(
            "INSERT INTO agent_external_tool_allowlist
             (agent_id, server_id, original_name, fingerprint, allowed, sensitive, revision)
             VALUES (?, ?, ?, ?, ?, ?, ?)
             ON CONFLICT(agent_id, server_id, original_name) DO UPDATE SET
                 fingerprint = excluded.fingerprint,
                 allowed = excluded.allowed,
                 sensitive = excluded.sensitive,
                 revision = MAX(excluded.revision, agent_external_tool_allowlist.revision + 1)",
        )
        .bind(aid)
        .bind(sid)
        .bind(&al.original_name)
        .bind(&al.fingerprint)
        .bind(al.allowed)
        .bind(al.sensitive)
        .bind(al.revision.max(1))
        .execute(&mut **tx)
        .await?;
    }

    for d in &bundle.devices {
        let Some(&aid) = agent_map.get(&d.agent_key) else {
            return Err(BackupError::InvalidReference(format!(
                "agent:{}",
                d.agent_key
            )));
        };
        let tid = match d.template_key.as_deref() {
            Some(tk) => Some(*template_map.get(tk).ok_or_else(|| {
                BackupError::InvalidReference(format!("template:{tk}"))
            })?),
            None => None,
        };
        upsert_device_row(tx, d, aid, tid, now).await?;
    }

    Ok(server_map)
}

async fn upsert_device_row(
    tx: &mut Transaction<'_, Sqlite>,
    d: &BackupDeviceRow,
    aid: i64,
    tid: Option<i64>,
    now: i64,
) -> Result<(), BackupError> {
    let (existing, target_kind): (Option<(i64, i64)>, &str) = if d.kind == "web_assistant" {
        let row = sqlx::query_as("SELECT id, revision FROM devices WHERE kind = 'web_assistant'")
            .fetch_optional(&mut **tx)
            .await?;
        (row, "web_assistant")
    } else {
        let row = sqlx::query_as("SELECT id, revision FROM devices WHERE device_id = ?")
            .bind(&d.device_id)
            .fetch_optional(&mut **tx)
            .await?;
        (row, "physical")
    };

    if let Some((id, rev)) = existing {
        sqlx::query(
            "UPDATE devices
             SET kind = ?, agent_id = ?, template_id = ?, name = ?, description = ?,
                 enabled = ?, metadata_json = ?, revision = ?, updated_at = ?
             WHERE id = ?",
        )
        .bind(target_kind)
        .bind(aid)
        .bind(tid)
        .bind(&d.name)
        .bind(&d.description)
        .bind(d.enabled)
        .bind(&d.metadata_json)
        .bind(rev + 1)
        .bind(now)
        .bind(id)
        .execute(&mut **tx)
        .await?;
    } else {
        sqlx::query(
            "INSERT INTO devices
             (device_id, kind, agent_id, template_id, name, description, enabled, metadata_json, revision, last_seen_at, created_at, updated_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, 1, ?, ?, ?)",
        )
        .bind(&d.device_id)
        .bind(target_kind)
        .bind(aid)
        .bind(tid)
        .bind(&d.name)
        .bind(&d.description)
        .bind(d.enabled)
        .bind(&d.metadata_json)
        .bind(d.last_seen_at)
        .bind(d.created_at.max(1))
        .bind(now)
        .execute(&mut **tx)
        .await?;
    }
    Ok(())
}
