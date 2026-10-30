use super::super::*;
use sqlx::{Row, SqlitePool};

pub(super) async fn fetch_providers(pool: &SqlitePool) -> Result<Vec<BackupProviderRow>, BackupError> {
    let rows = sqlx::query(
        "SELECT key, name, type, adapter, config_json, credential_json, enabled, created_at, updated_at
         FROM providers ORDER BY key ASC",
    )
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|r| BackupProviderRow {
            key: r.get("key"),
            name: r.get("name"),
            provider_type: r.get("type"),
            adapter: r.get("adapter"),
            config_json: r.get("config_json"),
            credential_json: r.get("credential_json"),
            enabled: r.get("enabled"),
            created_at: r.get("created_at"),
            updated_at: r.get("updated_at"),
        })
        .collect())
}

pub(super) async fn fetch_templates(pool: &SqlitePool) -> Result<Vec<BackupTemplateRow>, BackupError> {
    let rows = sqlx::query(
        "SELECT key, name, description, language, prompt, enabled, created_at, updated_at
         FROM agent_templates ORDER BY key ASC",
    )
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|r| BackupTemplateRow {
            key: r.get("key"),
            name: r.get("name"),
            description: r.get("description"),
            language: r.get("language"),
            prompt: r.get("prompt"),
            enabled: r.get("enabled"),
            created_at: r.get("created_at"),
            updated_at: r.get("updated_at"),
        })
        .collect())
}

pub(super) async fn fetch_template_provider_bindings(
    pool: &SqlitePool,
) -> Result<Vec<BackupTemplateProviderBindingRow>, BackupError> {
    let rows = sqlx::query(
        "SELECT t.key AS template_key, b.provider_type, b.provider_source, p.key AS provider_key,
                b.config_provider_key, b.created_at, b.updated_at
         FROM template_provider_bindings b
         JOIN agent_templates t ON t.id = b.template_id
         LEFT JOIN providers p ON p.id = b.provider_id
         ORDER BY t.key ASC, b.provider_type ASC",
    )
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|r| BackupTemplateProviderBindingRow {
            template_key: r.get("template_key"),
            provider_type: r.get("provider_type"),
            provider_source: r.get("provider_source"),
            provider_key: r.get("provider_key"),
            config_provider_key: r.get("config_provider_key"),
            created_at: r.get("created_at"),
            updated_at: r.get("updated_at"),
        })
        .collect())
}

pub(super) async fn fetch_agents(pool: &SqlitePool) -> Result<Vec<BackupAgentRow>, BackupError> {
    let rows = sqlx::query(
        "SELECT key, name, description, enabled, reminders_enabled, created_at, updated_at
         FROM agents ORDER BY key ASC",
    )
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|r| BackupAgentRow {
            key: r.get("key"),
            name: r.get("name"),
            description: r.get("description"),
            enabled: r.get("enabled"),
            reminders_enabled: r.get("reminders_enabled"),
            created_at: r.get("created_at"),
            updated_at: r.get("updated_at"),
        })
        .collect())
}

pub(super) async fn fetch_agent_template_assignments(
    pool: &SqlitePool,
) -> Result<Vec<BackupAgentTemplateAssignmentRow>, BackupError> {
    let rows = sqlx::query(
        "SELECT a.key AS agent_key, t.key AS template_key, ata.is_default, ata.enabled, ata.created_at
         FROM agent_template_assignments ata
         JOIN agents a ON a.id = ata.agent_id
         JOIN agent_templates t ON t.id = ata.template_id
         ORDER BY a.key ASC, t.key ASC",
    )
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|r| BackupAgentTemplateAssignmentRow {
            agent_key: r.get("agent_key"),
            template_key: r.get("template_key"),
            is_default: r.get("is_default"),
            enabled: r.get("enabled"),
            created_at: r.get("created_at"),
        })
        .collect())
}

pub(super) async fn fetch_mcp_servers(pool: &SqlitePool) -> Result<Vec<BackupMcpServerRow>, BackupError> {
    let rows = sqlx::query(
        "SELECT key, name, transport, url, headers_json, auth_type, auth_header_name,
                connect_timeout_ms, request_timeout_ms, enabled, credential_json, created_at, updated_at
         FROM mcp_servers ORDER BY key ASC",
    )
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|r| BackupMcpServerRow {
            key: r.get("key"),
            name: r.get("name"),
            transport: r.get("transport"),
            url: r.get("url"),
            headers_json: r.get("headers_json"),
            auth_type: r.get("auth_type"),
            auth_header_name: r.get("auth_header_name"),
            connect_timeout_ms: r.get("connect_timeout_ms"),
            request_timeout_ms: r.get("request_timeout_ms"),
            enabled: r.get("enabled"),
            credential_json: r.get("credential_json"),
            created_at: r.get("created_at"),
            updated_at: r.get("updated_at"),
        })
        .collect())
}

pub(super) async fn fetch_agent_mcp_bindings(
    pool: &SqlitePool,
) -> Result<Vec<BackupAgentMcpBindingRow>, BackupError> {
    let rows = sqlx::query(
        "SELECT a.key AS agent_key, s.key AS server_key, b.enabled, b.required, b.created_at
         FROM agent_mcp_bindings b
         JOIN agents a ON a.id = b.agent_id
         JOIN mcp_servers s ON s.id = b.mcp_server_id
         ORDER BY a.key ASC, s.key ASC",
    )
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|r| BackupAgentMcpBindingRow {
            agent_key: r.get("agent_key"),
            server_key: r.get("server_key"),
            enabled: r.get("enabled"),
            required: r.get("required"),
            created_at: r.get("created_at"),
        })
        .collect())
}

pub(super) async fn fetch_external_tool_observations(
    pool: &SqlitePool,
) -> Result<Vec<BackupExternalToolObservationRow>, BackupError> {
    let rows = sqlx::query(
        "SELECT s.key AS server_key, o.original_name, o.description, o.input_schema,
                o.fingerprint, o.server_revision, o.blocked, o.revision, o.observed_at
         FROM external_tool_observations o
         JOIN mcp_servers s ON s.id = o.server_id
         ORDER BY s.key ASC, o.original_name ASC",
    )
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|r| BackupExternalToolObservationRow {
            server_key: r.get("server_key"),
            original_name: r.get("original_name"),
            description: r.get("description"),
            input_schema: r.get("input_schema"),
            fingerprint: r.get("fingerprint"),
            server_revision: r.get("server_revision"),
            blocked: r.get("blocked"),
            revision: r.get("revision"),
            observed_at: r.get("observed_at"),
        })
        .collect())
}

pub(super) async fn fetch_agent_tool_allowlist(
    pool: &SqlitePool,
) -> Result<Vec<BackupAgentToolAllowlistRow>, BackupError> {
    let rows = sqlx::query(
        "SELECT a.key AS agent_key, s.key AS server_key, al.original_name, al.fingerprint,
                al.allowed, al.sensitive, al.revision
         FROM agent_external_tool_allowlist al
         JOIN agents a ON a.id = al.agent_id
         JOIN mcp_servers s ON s.id = al.server_id
         ORDER BY a.key ASC, s.key ASC, al.original_name ASC",
    )
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|r| BackupAgentToolAllowlistRow {
            agent_key: r.get("agent_key"),
            server_key: r.get("server_key"),
            original_name: r.get("original_name"),
            fingerprint: r.get("fingerprint"),
            allowed: r.get("allowed"),
            sensitive: r.get("sensitive"),
            revision: r.get("revision"),
        })
        .collect())
}

pub(super) async fn fetch_devices(pool: &SqlitePool) -> Result<Vec<BackupDeviceRow>, BackupError> {
    let rows = sqlx::query(
        "SELECT d.device_id, d.kind, a.key AS agent_key, t.key AS template_key,
                d.name, d.description, d.enabled, d.metadata_json, d.last_seen_at,
                d.created_at, d.updated_at
         FROM devices d
         JOIN agents a ON a.id = d.agent_id
         LEFT JOIN agent_templates t ON t.id = d.template_id
         ORDER BY d.kind ASC, d.device_id ASC",
    )
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|r| BackupDeviceRow {
            device_id: r.get("device_id"),
            kind: r.get("kind"),
            agent_key: r.get("agent_key"),
            template_key: r.get("template_key"),
            name: r.get("name"),
            description: r.get("description"),
            enabled: r.get("enabled"),
            metadata_json: r.get("metadata_json"),
            last_seen_at: r.get("last_seen_at"),
            created_at: r.get("created_at"),
            updated_at: r.get("updated_at"),
        })
        .collect())
}
