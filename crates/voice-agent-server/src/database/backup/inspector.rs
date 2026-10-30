use super::*;
use crate::database::{
    credentials,
    secrets::{SecretRef, SecretResolver},
};
use sqlx::{Row, SqlitePool};
use std::collections::HashSet;

pub(super) fn can_decrypt_credential(
    secret_resolver: &dyn SecretResolver,
    scope: &str,
    record: &str,
) -> bool {
    let Some(ref_str) = credentials::reference(scope, Some(record), None) else {
        return false;
    };
    let Ok(secret_ref) = SecretRef::parse(ref_str) else {
        return false;
    };
    secret_resolver.resolve(&secret_ref).is_ok()
}

pub(super) fn scrub_or_keep_optional_credential(
    secret_resolver: &dyn SecretResolver,
    scope: &str,
    raw: Option<&str>,
) -> Option<String> {
    match raw {
        Some(cred) if !cred.trim().is_empty() => {
            if can_decrypt_credential(secret_resolver, scope, cred) {
                Some(cred.to_owned())
            } else {
                None
            }
        }
        _ => None,
    }
}

pub(super) fn scrub_or_keep_gateway_credential(
    secret_resolver: &dyn SecretResolver,
    account_key: &str,
    raw_cred: &str,
    raw_status: &str,
) -> (String, String) {
    let trimmed = raw_cred.trim();
    if !trimmed.is_empty() && trimmed != "{}" {
        let scope = format!("gateway_account:{account_key}");
        if can_decrypt_credential(secret_resolver, &scope, trimmed) {
            (raw_cred.to_owned(), raw_status.to_owned())
        } else {
            ("{}".to_owned(), "reauth_required".to_owned())
        }
    } else {
        ("{}".to_owned(), raw_status.to_owned())
    }
}

fn record_credential_check(
    summary: &mut BackupCredentialSummary,
    secret_resolver: &dyn SecretResolver,
    scope: &str,
    cred: &str,
    resource_type: &str,
    resource_key: &str,
    scrub_action: &str,
    clear_unreadable: bool,
) {
    summary.total_encrypted += 1;
    if can_decrypt_credential(secret_resolver, scope, cred) {
        summary.decryptable += 1;
    } else {
        summary.unreadable += 1;
        summary.unreadable_items.push(UnreadableCredentialItem {
            resource_type: resource_type.to_owned(),
            resource_key: resource_key.to_owned(),
            action: if clear_unreadable {
                scrub_action.to_owned()
            } else {
                "reject".to_owned()
            },
        });
    }
}

pub(super) fn verify_bundle_credentials(
    bundle: &DatabaseBackupBundle,
    clear_unreadable: bool,
    secret_resolver: &dyn SecretResolver,
) -> (BackupCredentialSummary, Vec<String>) {
    let mut summary = BackupCredentialSummary::default();
    let mut warnings = Vec::new();

    for p in &bundle.providers {
        if let Some(ref cred) = p.credential_json
            && !cred.trim().is_empty()
        {
            let scope = format!("provider:{}", p.key);
            record_credential_check(
                &mut summary,
                secret_resolver,
                &scope,
                cred,
                "provider",
                &p.key,
                "clear_to_null",
                clear_unreadable,
            );
        }
    }

    for s in &bundle.mcp_servers {
        if let Some(ref cred) = s.credential_json
            && !cred.trim().is_empty()
        {
            let scope = format!("mcp:{}", s.key);
            record_credential_check(
                &mut summary,
                secret_resolver,
                &scope,
                cred,
                "mcp_server",
                &s.key,
                "clear_to_null",
                clear_unreadable,
            );
        }
    }

    for acc in &bundle.llm_gateway_accounts {
        let cred = acc.credential_json.trim();
        if !cred.is_empty() && cred != "{}" {
            let scope = format!("gateway_account:{}", acc.account_key);
            record_credential_check(
                &mut summary,
                secret_resolver,
                &scope,
                cred,
                "llm_gateway_account",
                &acc.account_key,
                "set_reauth_required",
                clear_unreadable,
            );
        }
    }

    for ch in &bundle.notification_channel_configs {
        if let Some(ref cred) = ch.encrypted_credential
            && !cred.trim().is_empty()
        {
            let scope = format!("notification_channel:{}", ch.channel_kind);
            record_credential_check(
                &mut summary,
                secret_resolver,
                &scope,
                cred,
                "notification_channel",
                &ch.channel_kind,
                "clear_to_null",
                clear_unreadable,
            );
        }
    }

    if summary.unreadable > 0 {
        if clear_unreadable {
            warnings.push(format!(
                "{} encrypted credential(s) cannot be decrypted with this server's key and will be cleared (or marked reauth_required for gateway accounts).",
                summary.unreadable
            ));
        } else {
            warnings.push(format!(
                "{} encrypted credential(s) cannot be decrypted with this server's key; enable clear_unreadable_credentials to import without failing.",
                summary.unreadable
            ));
        }
    }

    (summary, warnings)
}

fn diff_key_sets(
    backup_keys: &HashSet<String>,
    existing_keys: &HashSet<String>,
    mode: ImportMode,
) -> (u64, u64, u64, u64) {
    let mut to_create = 0;
    let mut to_update = 0;
    for k in backup_keys {
        if existing_keys.contains(k) {
            to_update += 1;
        } else {
            to_create += 1;
        }
    }
    let unmatched = existing_keys
        .iter()
        .filter(|k| !backup_keys.contains(*k))
        .count() as u64;
    match mode {
        ImportMode::Merge => (to_create, to_update, 0, unmatched),
        ImportMode::Replace => (to_create, to_update, unmatched, 0),
    }
}

async fn fetch_key_set(pool: &SqlitePool, sql: &str) -> Result<HashSet<String>, BackupError> {
    let rows: Vec<String> = sqlx::query_scalar(sql).fetch_all(pool).await?;
    Ok(rows.into_iter().collect())
}

pub(super) async fn inspect_bundle(
    pool: &SqlitePool,
    detected_format: BackupFormat,
    bundle: &DatabaseBackupBundle,
    options: BackupImportOptions,
    secret_resolver: &dyn SecretResolver,
) -> Result<BackupInspectionReport, BackupError> {
    let effective_scope = options.scope.resolve_effective(bundle.scope);
    let (credentials, mut warnings) = verify_bundle_credentials(
        bundle,
        options.clear_unreadable_credentials,
        secret_resolver,
    );

    let mut resources = Vec::new();
    let mut totals = BackupTotals::default();

    let mut add_resource =
        |name: &str, in_backup: u64, (c, u, d, un): (u64, u64, u64, u64)| {
            totals.to_create += c;
            totals.to_update += u;
            totals.to_delete += d;
            totals.unchanged += un;
            resources.push(BackupResourceCount {
                resource: name.to_owned(),
                in_backup,
                to_create: c,
                to_update: u,
                to_delete: d,
            });
        };

    // Core keyed tables
    for (resource, sql, backup_keys) in [
        (
            "providers",
            "SELECT key FROM providers",
            bundle
                .providers
                .iter()
                .map(|r| r.key.clone())
                .collect::<HashSet<String>>(),
        ),
        (
            "agent_templates",
            "SELECT key FROM agent_templates",
            bundle
                .agent_templates
                .iter()
                .map(|r| r.key.clone())
                .collect::<HashSet<String>>(),
        ),
    ] {
        let existing = fetch_key_set(pool, sql).await?;
        add_resource(
            resource,
            backup_keys.len() as u64,
            diff_key_sets(&backup_keys, &existing, options.mode),
        );
    }

    let existing_agents = fetch_key_set(pool, "SELECT key FROM agents").await?;
    let backup_agents: HashSet<String> = bundle.agents.iter().map(|r| r.key.clone()).collect();
    add_resource(
        "agents",
        bundle.agents.len() as u64,
        diff_key_sets(&backup_agents, &existing_agents, options.mode),
    );

    let existing_servers = fetch_key_set(pool, "SELECT key FROM mcp_servers").await?;
    let backup_servers: HashSet<String> =
        bundle.mcp_servers.iter().map(|r| r.key.clone()).collect();
    add_resource(
        "mcp_servers",
        bundle.mcp_servers.len() as u64,
        diff_key_sets(&backup_servers, &existing_servers, options.mode),
    );

    // devices (singleton web_assistant + physical by device_id)
    let existing_physical = fetch_key_set(
        pool,
        "SELECT device_id FROM devices WHERE kind = 'physical'",
    )
    .await?;
    let has_existing_web_assistant: i64 = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM devices WHERE kind = 'web_assistant')",
    )
    .fetch_one(pool)
    .await?;
    let backup_physical: HashSet<String> = bundle
        .devices
        .iter()
        .filter(|d| d.kind != "web_assistant")
        .map(|d| d.device_id.clone())
        .collect();
    let backup_has_web_assistant = bundle.devices.iter().any(|d| d.kind == "web_assistant");
    let (mut dev_c, mut dev_u, dev_d, mut dev_un) =
        diff_key_sets(&backup_physical, &existing_physical, options.mode);
    if backup_has_web_assistant {
        if has_existing_web_assistant != 0 {
            dev_u += 1;
        } else {
            dev_c += 1;
        }
    } else if has_existing_web_assistant != 0 {
        dev_un += 1;
    }
    add_resource(
        "devices",
        bundle.devices.len() as u64,
        (dev_c, dev_u, dev_d, dev_un),
    );

    for (resource, sql, backup_keys) in [
        (
            "speakers",
            "SELECT key FROM speakers",
            bundle
                .speakers
                .iter()
                .map(|r| r.key.clone())
                .collect::<HashSet<String>>(),
        ),
        (
            "llm_gateway_accounts",
            "SELECT account_key FROM llm_gateway_accounts",
            bundle
                .llm_gateway_accounts
                .iter()
                .map(|r| r.account_key.clone())
                .collect::<HashSet<String>>(),
        ),
    ] {
        let existing = fetch_key_set(pool, sql).await?;
        add_resource(
            resource,
            backup_keys.len() as u64,
            diff_key_sets(&backup_keys, &existing, options.mode),
        );
    }

    if effective_scope == BackupScope::Full {
        let existing_reminders = fetch_key_set(pool, "SELECT id FROM device_reminders").await?;
        let backup_reminders: HashSet<String> = bundle
            .device_reminders
            .iter()
            .map(|r| r.id.clone())
            .collect();
        add_resource(
            "device_reminders",
            bundle.device_reminders.len() as u64,
            diff_key_sets(&backup_reminders, &existing_reminders, options.mode),
        );

        let existing_history = fetch_key_set(
            pool,
            "SELECT session_id || ':' || CAST(sequence AS TEXT) FROM history_messages",
        )
        .await?;
        let backup_history: HashSet<String> = bundle
            .history_messages
            .iter()
            .map(|h| format!("{}:{}", h.session_id, h.sequence))
            .collect();
        add_resource(
            "history_messages",
            bundle.history_messages.len() as u64,
            diff_key_sets(&backup_history, &existing_history, options.mode),
        );

        let existing_inbox_rows =
            sqlx::query("SELECT id, dedup_key FROM notifications_inbox")
                .fetch_all(pool)
                .await?;
        let existing_ids: HashSet<String> =
            existing_inbox_rows.iter().map(|r| r.get("id")).collect();
        let existing_dedups: HashSet<String> = existing_inbox_rows
            .iter()
            .filter_map(|r| r.get::<Option<String>, _>("dedup_key"))
            .collect();
        let mut in_c = 0;
        let mut in_u = 0;
        let mut matched_existing = 0usize;
        for item in &bundle.notifications_inbox {
            let exists = existing_ids.contains(&item.id)
                || item
                    .dedup_key
                    .as_ref()
                    .is_some_and(|dk| existing_dedups.contains(dk));
            if exists {
                in_u += 1;
                matched_existing += 1;
            } else {
                in_c += 1;
            }
        }
        let unmatched_existing =
            existing_inbox_rows.len().saturating_sub(matched_existing) as u64;
        let (in_d, in_un) = match options.mode {
            ImportMode::Merge => (0, unmatched_existing),
            ImportMode::Replace => (unmatched_existing, 0),
        };
        add_resource(
            "notifications_inbox",
            bundle.notifications_inbox.len() as u64,
            (in_c, in_u, in_d, in_un),
        );
    } else if options.mode == ImportMode::Replace {
        let removed_agents: HashSet<&str> = existing_agents
            .iter()
            .filter(|k| !backup_agents.contains(*k))
            .map(String::as_str)
            .collect();
        let removed_devices: HashSet<&str> = existing_physical
            .iter()
            .filter(|k| !backup_physical.contains(*k))
            .map(String::as_str)
            .collect();
        if !removed_agents.is_empty() || !removed_devices.is_empty() {
            let history_rows = sqlx::query(
                "SELECT a.key AS agent_key, d.device_id AS device_id, d.kind AS device_kind
                 FROM history_messages h
                 JOIN agents a ON a.id = h.agent_id
                 JOIN devices d ON d.id = h.device_id",
            )
            .fetch_all(pool)
            .await?;
            let cascaded_history = history_rows
                .iter()
                .filter(|r| {
                    let ak: String = r.get("agent_key");
                    let did: String = r.get("device_id");
                    let dkind: String = r.get("device_kind");
                    removed_agents.contains(ak.as_str())
                        || (dkind == "physical" && removed_devices.contains(did.as_str()))
                })
                .count() as i64;

            let reminder_rows = sqlx::query(
                "SELECT d.device_id AS device_id
                 FROM device_reminders r
                 JOIN devices d ON d.id = r.device_row_id
                 WHERE d.kind = 'physical'",
            )
            .fetch_all(pool)
            .await?;
            let cascaded_reminders = reminder_rows
                .iter()
                .filter(|r| {
                    let did: String = r.get("device_id");
                    removed_devices.contains(did.as_str())
                })
                .count() as i64;

            if cascaded_history > 0 || cascaded_reminders > 0 {
                warnings.push(format!(
                    "Replace mode with config_only scope will also remove {} history message(s) and {} device reminder(s) tied to deleted agents or physical devices.",
                    cascaded_history, cascaded_reminders
                ));
            }
        }
    }

    Ok(BackupInspectionReport {
        detected_format,
        bundle_scope: bundle.scope,
        effective_scope,
        mode: options.mode,
        created_at: bundle.created_at,
        server_version: bundle.server_version.clone(),
        includes_background_settings: bundle.companion_settings.background_settings.is_some(),
        includes_knowledge_mcp_settings: bundle.companion_settings.knowledge_mcp_settings.is_some(),
        totals,
        resources,
        credentials,
        warnings,
    })
}
