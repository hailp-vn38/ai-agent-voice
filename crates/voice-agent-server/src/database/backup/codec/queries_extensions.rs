use super::super::*;
use base64::{Engine, engine::general_purpose::STANDARD as BASE64};
use sqlx::{Row, SqlitePool};

pub(super) async fn fetch_speakers(pool: &SqlitePool) -> Result<Vec<BackupSpeakerRow>, BackupError> {
    let rows = sqlx::query(
        "SELECT key, name, description, enabled, created_at, updated_at
         FROM speakers ORDER BY key ASC",
    )
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|r| BackupSpeakerRow {
            key: r.get("key"),
            name: r.get("name"),
            description: r.get("description"),
            enabled: r.get("enabled"),
            created_at: r.get("created_at"),
            updated_at: r.get("updated_at"),
        })
        .collect())
}

pub(super) async fn fetch_speaker_voiceprints(
    pool: &SqlitePool,
) -> Result<Vec<BackupSpeakerVoiceprintRow>, BackupError> {
    let rows = sqlx::query(
        "SELECT sp.key AS speaker_key, v.embedding_space, v.provider_key, v.provider_revision,
                v.dims, v.vector, v.revision, v.sample_count, v.browser_validation_status,
                v.enrolled_at, v.updated_at, v.calibration_revision
         FROM speaker_voiceprints v
         JOIN speakers sp ON sp.id = v.speaker_id
         ORDER BY sp.key ASC, v.embedding_space ASC",
    )
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|r| {
            let vector: Vec<u8> = r.get("vector");
            BackupSpeakerVoiceprintRow {
                speaker_key: r.get("speaker_key"),
                embedding_space: r.get("embedding_space"),
                provider_key: r.get("provider_key"),
                provider_revision: r.get("provider_revision"),
                dims: r.get("dims"),
                vector_base64: BASE64.encode(&vector),
                revision: r.get("revision"),
                sample_count: r.get("sample_count"),
                browser_validation_status: r.get("browser_validation_status"),
                enrolled_at: r.get("enrolled_at"),
                updated_at: r.get("updated_at"),
                calibration_revision: r.get("calibration_revision"),
            }
        })
        .collect())
}

pub(super) async fn fetch_agent_speaker_policies(
    pool: &SqlitePool,
) -> Result<Vec<BackupAgentSpeakerPolicyRow>, BackupError> {
    let rows = sqlx::query(
        "SELECT a.key AS agent_key, p.mode, p.revision
         FROM agent_speaker_policies p
         JOIN agents a ON a.id = p.agent_id
         ORDER BY a.key ASC",
    )
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|r| BackupAgentSpeakerPolicyRow {
            agent_key: r.get("agent_key"),
            mode: r.get("mode"),
            revision: r.get("revision"),
        })
        .collect())
}

pub(super) async fn fetch_agent_speaker_candidates(
    pool: &SqlitePool,
) -> Result<Vec<BackupAgentSpeakerCandidateRow>, BackupError> {
    let rows = sqlx::query(
        "SELECT a.key AS agent_key, sp.key AS speaker_key, c.created_at
         FROM agent_speaker_candidates c
         JOIN agents a ON a.id = c.agent_id
         JOIN speakers sp ON sp.id = c.speaker_id
         ORDER BY a.key ASC, sp.key ASC",
    )
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|r| BackupAgentSpeakerCandidateRow {
            agent_key: r.get("agent_key"),
            speaker_key: r.get("speaker_key"),
            created_at: r.get("created_at"),
        })
        .collect())
}

pub(super) async fn fetch_agent_speaker_template_grants(
    pool: &SqlitePool,
) -> Result<Vec<BackupAgentSpeakerTemplateGrantRow>, BackupError> {
    let rows = sqlx::query(
        "SELECT a.key AS agent_key, sp.key AS speaker_key, t.key AS template_key, g.created_at
         FROM agent_speaker_template_grants g
         JOIN agents a ON a.id = g.agent_id
         JOIN speakers sp ON sp.id = g.speaker_id
         JOIN agent_templates t ON t.id = g.template_id
         ORDER BY a.key ASC, sp.key ASC, t.key ASC",
    )
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|r| BackupAgentSpeakerTemplateGrantRow {
            agent_key: r.get("agent_key"),
            speaker_key: r.get("speaker_key"),
            template_key: r.get("template_key"),
            created_at: r.get("created_at"),
        })
        .collect())
}

pub(super) async fn fetch_speaker_calibration_catalog(
    pool: &SqlitePool,
) -> Result<Option<BackupSpeakerCalibrationCatalogRow>, BackupError> {
    let row = sqlx::query(
        "SELECT calibration_revision, source_digest, profiles_json, published_at
         FROM speaker_calibration_catalog WHERE id = 1",
    )
    .fetch_optional(pool)
    .await?;
    Ok(row.map(|r| BackupSpeakerCalibrationCatalogRow {
        calibration_revision: r.get("calibration_revision"),
        source_digest: r.get("source_digest"),
        profiles_json: r.get("profiles_json"),
        published_at: r.get("published_at"),
    }))
}

pub(super) async fn fetch_speaker_calibration_evidence(
    pool: &SqlitePool,
) -> Result<Vec<BackupSpeakerCalibrationEvidenceRow>, BackupError> {
    let rows = sqlx::query(
        "SELECT calibration_revision, candidate_set_digest, candidate_set_json, report_ref, created_at
         FROM speaker_calibration_evidence
         ORDER BY calibration_revision ASC, candidate_set_digest ASC",
    )
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|r| BackupSpeakerCalibrationEvidenceRow {
            calibration_revision: r.get("calibration_revision"),
            candidate_set_digest: r.get("candidate_set_digest"),
            candidate_set_json: r.get("candidate_set_json"),
            report_ref: r.get("report_ref"),
            created_at: r.get("created_at"),
        })
        .collect())
}

pub(super) async fn fetch_llm_gateway_accounts(
    pool: &SqlitePool,
) -> Result<Vec<BackupLlmGatewayAccountRow>, BackupError> {
    let rows = sqlx::query(
        "SELECT account_key, vendor, label, account_email, credential_json, expires_at,
                enabled, priority, status, last_error, last_refreshed_at, last_used_at,
                created_at, updated_at
         FROM llm_gateway_accounts ORDER BY account_key ASC",
    )
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|r| BackupLlmGatewayAccountRow {
            account_key: r.get("account_key"),
            vendor: r.get("vendor"),
            label: r.get("label"),
            account_email: r.get("account_email"),
            credential_json: r.get("credential_json"),
            expires_at: r.get("expires_at"),
            enabled: r.get("enabled"),
            priority: r.get("priority"),
            status: r.get("status"),
            last_error: r.get("last_error"),
            last_refreshed_at: r.get("last_refreshed_at"),
            last_used_at: r.get("last_used_at"),
            created_at: r.get("created_at"),
            updated_at: r.get("updated_at"),
        })
        .collect())
}

pub(super) async fn fetch_notification_settings(
    pool: &SqlitePool,
) -> Result<Option<BackupNotificationSettingsRow>, BackupError> {
    let row = sqlx::query(
        "SELECT web_ui_toast_enabled, retention_days, updated_at_ms
         FROM notification_settings_state WHERE id = 1",
    )
    .fetch_optional(pool)
    .await?;
    Ok(row.map(|r| BackupNotificationSettingsRow {
        web_ui_toast_enabled: r.get("web_ui_toast_enabled"),
        retention_days: r.get("retention_days"),
        updated_at_ms: r.get("updated_at_ms"),
    }))
}

pub(super) async fn fetch_notification_channels(
    pool: &SqlitePool,
) -> Result<Vec<BackupNotificationChannelConfigRow>, BackupError> {
    let rows = sqlx::query(
        "SELECT channel_kind, enabled, target_json, subscribed_topics_json,
                encrypted_credential, updated_at_ms
         FROM notification_channel_configs ORDER BY channel_kind ASC",
    )
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|r| BackupNotificationChannelConfigRow {
            channel_kind: r.get("channel_kind"),
            enabled: r.get("enabled"),
            target_json: r.get("target_json"),
            subscribed_topics_json: r.get("subscribed_topics_json"),
            encrypted_credential: r.get("encrypted_credential"),
            updated_at_ms: r.get("updated_at_ms"),
        })
        .collect())
}

pub(super) async fn fetch_system_mcp_grants(
    pool: &SqlitePool,
) -> Result<Option<BackupSystemMcpGrantsRow>, BackupError> {
    let row = sqlx::query(
        "SELECT system_read, mcp_read, mcp_discover, mcp_manage_request
         FROM system_mcp_grants WHERE id = 1",
    )
    .fetch_optional(pool)
    .await?;
    Ok(row.map(|r| BackupSystemMcpGrantsRow {
        system_read: r.get("system_read"),
        mcp_read: r.get("mcp_read"),
        mcp_discover: r.get("mcp_discover"),
        mcp_manage_request: r.get("mcp_manage_request"),
    }))
}

pub(super) async fn fetch_device_reminders(
    pool: &SqlitePool,
) -> Result<Vec<BackupDeviceReminderRow>, BackupError> {
    let rows = sqlx::query(
        "SELECT r.id, d.device_id, d.kind AS device_kind, r.message, r.due_at_ms, r.timezone,
                r.status, r.row_revision, r.schedule_revision, r.source, r.attempts_used,
                r.next_attempt_at_ms, r.created_at_ms, r.updated_at_ms, r.completed_at_ms,
                r.cancelled_at_ms
         FROM device_reminders r
         JOIN devices d ON d.id = r.device_row_id
         ORDER BY r.created_at_ms ASC, r.id ASC",
    )
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|r| BackupDeviceReminderRow {
            id: r.get("id"),
            device_id: r.get("device_id"),
            device_kind: r.get("device_kind"),
            message: r.get("message"),
            due_at_ms: r.get("due_at_ms"),
            timezone: r.get("timezone"),
            status: r.get("status"),
            row_revision: r.get("row_revision"),
            schedule_revision: r.get("schedule_revision"),
            source: r.get("source"),
            attempts_used: r.get("attempts_used"),
            next_attempt_at_ms: r.get("next_attempt_at_ms"),
            created_at_ms: r.get("created_at_ms"),
            updated_at_ms: r.get("updated_at_ms"),
            completed_at_ms: r.get("completed_at_ms"),
            cancelled_at_ms: r.get("cancelled_at_ms"),
        })
        .collect())
}

pub(super) async fn fetch_history_messages(
    pool: &SqlitePool,
) -> Result<Vec<BackupHistoryMessageRow>, BackupError> {
    let rows = sqlx::query(
        "SELECT h.session_id, h.sequence, d.device_id, d.kind AS device_kind,
                a.key AS agent_key, t.key AS template_key, h.turn_id, h.role, h.text, h.created_at
         FROM history_messages h
         JOIN devices d ON d.id = h.device_id
         JOIN agents a ON a.id = h.agent_id
         LEFT JOIN agent_templates t ON t.id = h.template_id
         ORDER BY h.created_at ASC, h.session_id ASC, h.sequence ASC",
    )
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|r| BackupHistoryMessageRow {
            session_id: r.get("session_id"),
            sequence: r.get("sequence"),
            device_id: r.get("device_id"),
            device_kind: r.get("device_kind"),
            agent_key: r.get("agent_key"),
            template_key: r.get("template_key"),
            turn_id: r.get("turn_id"),
            role: r.get("role"),
            text: r.get("text"),
            created_at: r.get("created_at"),
        })
        .collect())
}

pub(super) async fn fetch_notifications_inbox(
    pool: &SqlitePool,
) -> Result<Vec<BackupNotificationInboxRow>, BackupError> {
    let rows = sqlx::query(
        "SELECT id, dedup_key, topic, severity, title, body, source_kind, source_id,
                metadata_json, is_read, read_at_ms, created_at_ms
         FROM notifications_inbox
         ORDER BY created_at_ms ASC, id ASC",
    )
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|r| BackupNotificationInboxRow {
            id: r.get("id"),
            dedup_key: r.get("dedup_key"),
            topic: r.get("topic"),
            severity: r.get("severity"),
            title: r.get("title"),
            body: r.get("body"),
            source_kind: r.get("source_kind"),
            source_id: r.get("source_id"),
            metadata_json: r.get("metadata_json"),
            is_read: r.get("is_read"),
            read_at_ms: r.get("read_at_ms"),
            created_at_ms: r.get("created_at_ms"),
        })
        .collect())
}
