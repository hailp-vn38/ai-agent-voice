use super::types::{
    ChannelDeliveryStatusView, DeleteInboxOutput, ExternalChannelKind, InboxListQuery,
    InboxPageView, MarkReadOutput, NotificationContent, NotificationHubError, NotificationInput,
    NotificationItemView, NotificationSettingsUpdate, NotificationSettingsView,
    NotificationSeverity, NotificationSourceKind, NotificationTopic, OutboxDeliveryStatus,
    TelegramChannelSettingsView, WebUiChannelSettingsView, ZaloChannelSettingsView,
};
use crate::database::{
    Database,
    audit::{AuditOutcome, audit, audit_conflict},
    credentials, is_busy,
    secrets::{
        SecretRef, SecretResolveError, SecretResolver, SecretValue, notification_channel_secret_env,
    },
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sqlx::{FromRow, Row};
use std::collections::HashMap;
use uuid::Uuid;

const DEFAULT_PAGE_SIZE: u32 = 30;
const MAX_PAGE_SIZE: u32 = 100;
const STALE_SENDING_TIMEOUT_MS: i64 = 60_000;

pub(super) fn now_millis() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

fn map_sqlx(err: sqlx::Error) -> NotificationHubError {
    if is_busy(&err) {
        NotificationHubError::Busy
    } else {
        NotificationHubError::Database(err)
    }
}

fn truncate_utf8(input: &str, max_bytes: usize) -> String {
    let trimmed = input.trim();
    if trimmed.len() <= max_bytes {
        return trimmed.to_owned();
    }
    let mut end = max_bytes;
    while end > 0 && !trimmed.is_char_boundary(end) {
        end -= 1;
    }
    trimmed[..end].to_owned()
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub(super) struct TelegramTargetConfig {
    #[serde(default)]
    pub chat_id: String,
    #[serde(default)]
    pub thread_id: Option<i64>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub(super) struct ZaloTargetConfig {
    #[serde(default)]
    pub recipient_id: String,
}

#[derive(FromRow)]
struct InboxRow {
    id: String,
    topic: String,
    severity: String,
    title: String,
    body: String,
    source_kind: Option<String>,
    source_id: Option<String>,
    metadata_json: String,
    is_read: i64,
    read_at_ms: Option<i64>,
    created_at_ms: i64,
}

#[derive(FromRow)]
struct OutboxStatusRow {
    notification_id: String,
    channel_kind: String,
    status: String,
    attempts: i64,
    last_error_code: Option<String>,
    updated_at_ms: i64,
}

#[derive(FromRow)]
pub(super) struct ChannelConfigRow {
    pub channel_kind: String,
    pub enabled: i64,
    pub target_json: String,
    pub subscribed_topics_json: String,
    pub encrypted_credential: Option<String>,
}

#[derive(Debug, Clone)]
pub(super) struct ClaimedDeliveryAttempt {
    pub outbox_id: String,
    pub notification_id: String,
    pub channel: ExternalChannelKind,
    pub attempts: i64,
    pub max_attempts: i64,
    pub content: NotificationContent,
    pub target_json: String,
    pub encrypted_credential: Option<String>,
    pub channel_enabled: bool,
}

fn row_to_item(row: InboxRow, deliveries: Vec<ChannelDeliveryStatusView>) -> NotificationItemView {
    let topic = NotificationTopic::parse(&row.topic).unwrap_or(NotificationTopic::TaskCompleted);
    let severity = NotificationSeverity::parse(&row.severity).unwrap_or(NotificationSeverity::Info);
    let source_kind = row
        .source_kind
        .as_deref()
        .and_then(NotificationSourceKind::parse);
    let metadata = serde_json::from_str::<Value>(&row.metadata_json)
        .ok()
        .filter(Value::is_object)
        .unwrap_or_else(|| json!({}));

    NotificationItemView {
        id: row.id,
        content: NotificationContent {
            topic,
            severity,
            title: row.title,
            body: row.body,
            source_kind,
            source_id: row.source_id,
        },
        metadata,
        is_read: row.is_read != 0,
        read_at_ms: row.read_at_ms,
        created_at_ms: row.created_at_ms,
        deliveries,
    }
}

async fn load_deliveries_map(
    db: &Database,
    notification_ids: &[String],
) -> Result<HashMap<String, Vec<ChannelDeliveryStatusView>>, NotificationHubError> {
    let mut map: HashMap<String, Vec<ChannelDeliveryStatusView>> = HashMap::new();
    if notification_ids.is_empty() {
        return Ok(map);
    }

    let placeholders = std::iter::repeat_n("?", notification_ids.len())
        .collect::<Vec<_>>()
        .join(",");
    let sql = format!(
        "SELECT notification_id, channel_kind, status, attempts, last_error_code, updated_at_ms \
         FROM notification_outbox WHERE notification_id IN ({placeholders}) \
         ORDER BY channel_kind ASC"
    );
    let mut query = sqlx::query_as::<_, OutboxStatusRow>(&sql);
    for id in notification_ids {
        query = query.bind(id);
    }
    let rows = query.fetch_all(db.pool()).await.map_err(map_sqlx)?;
    for row in rows {
        let Some(channel) = ExternalChannelKind::parse(&row.channel_kind) else {
            continue;
        };
        let Some(status) = OutboxDeliveryStatus::parse(&row.status) else {
            continue;
        };
        map.entry(row.notification_id)
            .or_default()
            .push(ChannelDeliveryStatusView {
                channel,
                status,
                attempts: row.attempts,
                last_error_code: row.last_error_code,
                updated_at_ms: row.updated_at_ms,
            });
    }
    Ok(map)
}

async fn hydrate_optional_inbox_row(
    db: &Database,
    row: Option<InboxRow>,
) -> Result<Option<NotificationItemView>, NotificationHubError> {
    let Some(row) = row else {
        return Ok(None);
    };
    let mut deliveries_map = load_deliveries_map(db, std::slice::from_ref(&row.id)).await?;
    let deliveries = deliveries_map.remove(&row.id).unwrap_or_default();
    Ok(Some(row_to_item(row, deliveries)))
}

pub(super) async fn get_inbox_item_by_id(
    db: &Database,
    id: &str,
) -> Result<Option<NotificationItemView>, NotificationHubError> {
    let row = sqlx::query_as::<_, InboxRow>(
        "SELECT id, topic, severity, title, body, source_kind, source_id, metadata_json, \
         is_read, read_at_ms, created_at_ms \
         FROM notifications_inbox WHERE id = ?",
    )
    .bind(id)
    .fetch_optional(db.pool())
    .await
    .map_err(map_sqlx)?;

    hydrate_optional_inbox_row(db, row).await
}

async fn get_inbox_item_by_dedup_key(
    db: &Database,
    dedup_key: &str,
) -> Result<Option<NotificationItemView>, NotificationHubError> {
    let row = sqlx::query_as::<_, InboxRow>(
        "SELECT id, topic, severity, title, body, source_kind, source_id, metadata_json, \
         is_read, read_at_ms, created_at_ms \
         FROM notifications_inbox WHERE dedup_key = ?",
    )
    .bind(dedup_key)
    .fetch_optional(db.pool())
    .await
    .map_err(map_sqlx)?;

    hydrate_optional_inbox_row(db, row).await
}

pub(super) async fn publish_to_store(
    db: &Database,
    input: NotificationInput,
) -> Result<(NotificationItemView, bool), NotificationHubError> {
    let title = truncate_utf8(&input.content.title, 512);
    if title.is_empty() {
        return Err(NotificationHubError::InvalidInput(
            "empty_notification_title",
        ));
    }
    let body = truncate_utf8(&input.content.body, 8192);
    let dedup_key = input
        .dedup_key
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| truncate_utf8(s, 256));
    let source_id = input
        .content
        .source_id
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| truncate_utf8(s, 256));
    let metadata_obj = if input.metadata.is_object() {
        input.metadata
    } else {
        json!({})
    };
    let metadata_json = {
        let raw = serde_json::to_string(&metadata_obj).unwrap_or_else(|_| "{}".to_owned());
        if raw.len() <= 16_384 {
            raw
        } else {
            "{}".to_owned()
        }
    };

    if let Some(ref key) = dedup_key
        && let Some(existing) = get_inbox_item_by_dedup_key(db, key).await?
    {
        return Ok((existing, false));
    }

    let now_ms = now_millis();
    let notification_id = Uuid::new_v4().to_string();

    let mut tx = db.pool().begin().await.map_err(map_sqlx)?;
    let insert_res = sqlx::query(
        "INSERT INTO notifications_inbox \
         (id, dedup_key, topic, severity, title, body, source_kind, source_id, metadata_json, is_read, read_at_ms, created_at_ms) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, 0, NULL, ?)",
    )
    .bind(&notification_id)
    .bind(&dedup_key)
    .bind(input.content.topic.as_str())
    .bind(input.content.severity.as_str())
    .bind(&title)
    .bind(&body)
    .bind(input.content.source_kind.map(NotificationSourceKind::as_str))
    .bind(&source_id)
    .bind(&metadata_json)
    .bind(now_ms)
    .execute(&mut *tx)
    .await;

    match insert_res {
        Ok(_) => {}
        Err(err) => {
            let _ = tx.rollback().await;
            if let Some(ref key) = dedup_key
                && let Some(existing) = get_inbox_item_by_dedup_key(db, key).await?
            {
                return Ok((existing, false));
            }
            return Err(map_sqlx(err));
        }
    }

    let channel_rows = sqlx::query_as::<_, ChannelConfigRow>(
        "SELECT channel_kind, enabled, target_json, subscribed_topics_json, encrypted_credential \
         FROM notification_channel_configs WHERE enabled = 1 ORDER BY channel_kind ASC",
    )
    .fetch_all(&mut *tx)
    .await
    .map_err(map_sqlx)?;

    for ch in channel_rows {
        let Some(channel_kind) = ExternalChannelKind::parse(&ch.channel_kind) else {
            continue;
        };
        let subscribed: Vec<NotificationTopic> =
            serde_json::from_str(&ch.subscribed_topics_json).unwrap_or_default();
        if !subscribed.contains(&input.content.topic) {
            continue;
        }
        let outbox_id = Uuid::new_v4().to_string();
        sqlx::query(
            "INSERT INTO notification_outbox \
             (id, notification_id, channel_kind, status, attempts, max_attempts, next_attempt_at_ms, last_error_code, external_message_id, created_at_ms, updated_at_ms) \
             VALUES (?, ?, ?, 'pending', 0, 4, ?, NULL, NULL, ?, ?)",
        )
        .bind(&outbox_id)
        .bind(&notification_id)
        .bind(channel_kind.as_str())
        .bind(now_ms)
        .bind(now_ms)
        .bind(now_ms)
        .execute(&mut *tx)
        .await
        .map_err(map_sqlx)?;
    }

    tx.commit().await.map_err(map_sqlx)?;

    let created = get_inbox_item_by_id(db, &notification_id)
        .await?
        .ok_or(NotificationHubError::NotFound)?;
    Ok((created, true))
}

pub(super) async fn count_unread(db: &Database) -> Result<u64, NotificationHubError> {
    let unread: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM notifications_inbox WHERE is_read = 0")
            .fetch_one(db.pool())
            .await
            .map_err(map_sqlx)?;
    Ok(unread.max(0) as u64)
}

pub(super) async fn list_inbox_from_store(
    db: &Database,
    query: &InboxListQuery,
) -> Result<InboxPageView, NotificationHubError> {
    let page = query.page.unwrap_or(1);
    let page_size = query.page_size.unwrap_or(DEFAULT_PAGE_SIZE);
    if page == 0 || page > 10_000 || !(1..=MAX_PAGE_SIZE).contains(&page_size) {
        return Err(NotificationHubError::InvalidInput("invalid_query"));
    }

    let unread_only = query.unread_only.unwrap_or(false);
    let unread_count = count_unread(db).await?;

    let mut where_clauses: Vec<&'static str> = Vec::new();
    if unread_only {
        where_clauses.push("is_read = 0");
    }
    if query.topic.is_some() {
        where_clauses.push("topic = ?");
    }
    let where_sql = if where_clauses.is_empty() {
        String::new()
    } else {
        format!("WHERE {}", where_clauses.join(" AND "))
    };

    let count_sql = format!("SELECT COUNT(*) FROM notifications_inbox {where_sql}");
    let mut count_q = sqlx::query_scalar::<_, i64>(&count_sql);
    if let Some(topic) = query.topic {
        count_q = count_q.bind(topic.as_str());
    }
    let total = count_q.fetch_one(db.pool()).await.map_err(map_sqlx)?.max(0) as u64;

    let offset = i64::from(page - 1) * i64::from(page_size);
    let items_sql = format!(
        "SELECT id, topic, severity, title, body, source_kind, source_id, metadata_json, \
         is_read, read_at_ms, created_at_ms \
         FROM notifications_inbox {where_sql} \
         ORDER BY created_at_ms DESC, id DESC LIMIT ? OFFSET ?"
    );
    let mut items_q = sqlx::query_as::<_, InboxRow>(&items_sql);
    if let Some(topic) = query.topic {
        items_q = items_q.bind(topic.as_str());
    }
    let rows = items_q
        .bind(i64::from(page_size))
        .bind(offset)
        .fetch_all(db.pool())
        .await
        .map_err(map_sqlx)?;

    let ids: Vec<String> = rows.iter().map(|r| r.id.clone()).collect();
    let mut deliveries_map = load_deliveries_map(db, &ids).await?;

    let items = rows
        .into_iter()
        .map(|row| {
            let deliveries = deliveries_map.remove(&row.id).unwrap_or_default();
            row_to_item(row, deliveries)
        })
        .collect();

    Ok(InboxPageView {
        items,
        total,
        unread_count,
        page,
        page_size,
    })
}

pub(super) async fn mark_read_in_store(
    db: &Database,
    ids: &[String],
    mark_all: bool,
) -> Result<MarkReadOutput, NotificationHubError> {
    let now_ms = now_millis();
    let updated = if mark_all {
        sqlx::query("UPDATE notifications_inbox SET is_read = 1, read_at_ms = ? WHERE is_read = 0")
            .bind(now_ms)
            .execute(db.pool())
            .await
            .map_err(map_sqlx)?
            .rows_affected()
    } else if ids.is_empty() {
        0
    } else {
        if ids.len() > 200 {
            return Err(NotificationHubError::InvalidInput("too_many_ids"));
        }
        let placeholders = std::iter::repeat_n("?", ids.len())
            .collect::<Vec<_>>()
            .join(",");
        let sql = format!(
            "UPDATE notifications_inbox SET is_read = 1, read_at_ms = ? \
             WHERE is_read = 0 AND id IN ({placeholders})"
        );
        let mut q = sqlx::query(&sql).bind(now_ms);
        for id in ids {
            q = q.bind(id);
        }
        q.execute(db.pool())
            .await
            .map_err(map_sqlx)?
            .rows_affected()
    };

    let unread_count = count_unread(db).await?;
    Ok(MarkReadOutput {
        updated,
        unread_count,
    })
}

pub(super) async fn delete_inbox_in_store(
    db: &Database,
    id: Option<&str>,
    read_only: bool,
) -> Result<DeleteInboxOutput, NotificationHubError> {
    let deleted = if let Some(target_id) = id {
        sqlx::query("DELETE FROM notifications_inbox WHERE id = ?")
            .bind(target_id)
            .execute(db.pool())
            .await
            .map_err(map_sqlx)?
            .rows_affected()
    } else if read_only {
        sqlx::query("DELETE FROM notifications_inbox WHERE is_read = 1")
            .execute(db.pool())
            .await
            .map_err(map_sqlx)?
            .rows_affected()
    } else {
        return Err(NotificationHubError::InvalidInput("invalid_delete_scope"));
    };

    let unread_count = count_unread(db).await?;
    Ok(DeleteInboxOutput {
        deleted,
        unread_count,
    })
}

fn has_env_fallback(secret_resolver: &dyn SecretResolver, channel: ExternalChannelKind) -> bool {
    let Some(env_name) = notification_channel_secret_env(channel.as_str()) else {
        return false;
    };
    let Ok(secret_ref) = SecretRef::parse(env_name.to_owned()) else {
        return false;
    };
    secret_resolver.resolve(&secret_ref).is_ok()
}

pub(super) async fn load_channel_config_row(
    db: &Database,
    channel: ExternalChannelKind,
) -> Result<ChannelConfigRow, NotificationHubError> {
    sqlx::query_as::<_, ChannelConfigRow>(
        "SELECT channel_kind, enabled, target_json, subscribed_topics_json, encrypted_credential \
         FROM notification_channel_configs WHERE channel_kind = ?",
    )
    .bind(channel.as_str())
    .fetch_one(db.pool())
    .await
    .map_err(map_sqlx)
}

pub(super) async fn get_settings_from_store(
    db: &Database,
    secret_resolver: &dyn SecretResolver,
) -> Result<NotificationSettingsView, NotificationHubError> {
    let state_row = sqlx::query(
        "SELECT revision, web_ui_toast_enabled, retention_days \
         FROM notification_settings_state WHERE id = 1",
    )
    .fetch_one(db.pool())
    .await
    .map_err(map_sqlx)?;

    let revision: i64 = state_row.get("revision");
    let toast_enabled: i64 = state_row.get("web_ui_toast_enabled");
    let retention_days: i64 = state_row.get("retention_days");

    let tg_row = load_channel_config_row(db, ExternalChannelKind::Telegram).await?;
    let tg_target: TelegramTargetConfig =
        serde_json::from_str(&tg_row.target_json).unwrap_or_default();
    let tg_topics: Vec<NotificationTopic> =
        serde_json::from_str(&tg_row.subscribed_topics_json).unwrap_or_default();
    let tg_meta = credentials::metadata(tg_row.encrypted_credential.as_deref());

    let zalo_row = load_channel_config_row(db, ExternalChannelKind::Zalo).await?;
    let zalo_target: ZaloTargetConfig =
        serde_json::from_str(&zalo_row.target_json).unwrap_or_default();
    let zalo_topics: Vec<NotificationTopic> =
        serde_json::from_str(&zalo_row.subscribed_topics_json).unwrap_or_default();
    let zalo_meta = credentials::metadata(zalo_row.encrypted_credential.as_deref());

    Ok(NotificationSettingsView {
        revision,
        web_ui: WebUiChannelSettingsView {
            enabled: true,
            toast_enabled: toast_enabled != 0,
            retention_days,
        },
        telegram: TelegramChannelSettingsView {
            enabled: tg_row.enabled != 0,
            chat_id: tg_target.chat_id,
            thread_id: tg_target.thread_id,
            subscribed_topics: tg_topics,
            has_saved_token: tg_row.encrypted_credential.is_some(),
            has_env_fallback: has_env_fallback(secret_resolver, ExternalChannelKind::Telegram),
            credential_metadata: tg_meta,
        },
        zalo: ZaloChannelSettingsView {
            enabled: zalo_row.enabled != 0,
            recipient_id: zalo_target.recipient_id,
            subscribed_topics: zalo_topics,
            has_saved_token: zalo_row.encrypted_credential.is_some(),
            has_env_fallback: has_env_fallback(secret_resolver, ExternalChannelKind::Zalo),
            credential_metadata: zalo_meta,
        },
    })
}

fn dedup_topics(topics: &[NotificationTopic]) -> Vec<NotificationTopic> {
    let mut out = Vec::new();
    for &t in topics {
        if !out.contains(&t) {
            out.push(t);
        }
    }
    out
}

fn resolve_patched_credential(
    secret_resolver: &dyn SecretResolver,
    channel: ExternalChannelKind,
    current_encrypted: Option<String>,
    patch: crate::app::admin::Patch<SecretValue>,
) -> Result<Option<String>, NotificationHubError> {
    match patch.value() {
        None => Ok(current_encrypted),
        Some(None) => Ok(None),
        Some(Some(secret)) => {
            if !credentials::valid_input(&secret) {
                return Err(NotificationHubError::InvalidInput("invalid_bot_token"));
            }
            match secret_resolver.seal(&secret, channel.credential_scope()) {
                Ok(sealed) => Ok(Some(sealed)),
                Err(SecretResolveError::Invalid) => {
                    Err(NotificationHubError::InvalidInput("invalid_bot_token"))
                }
                Err(SecretResolveError::Unavailable) => {
                    Err(NotificationHubError::EncryptionUnavailable)
                }
            }
        }
    }
}

pub(super) async fn update_settings_in_store(
    db: &Database,
    secret_resolver: &dyn SecretResolver,
    expected_revision: i64,
    update: NotificationSettingsUpdate,
    request_id: &str,
) -> Result<NotificationSettingsView, NotificationHubError> {
    if expected_revision <= 0 {
        return Err(NotificationHubError::InvalidInput("invalid_if_match"));
    }
    if !(1..=365).contains(&update.web_ui.retention_days) {
        return Err(NotificationHubError::InvalidInput("invalid_retention_days"));
    }

    let tg_chat_id = update.telegram.chat_id.trim().to_owned();
    if tg_chat_id.len() > 256 {
        return Err(NotificationHubError::InvalidInput("invalid_chat_id"));
    }
    let zalo_recipient_id = update.zalo.recipient_id.trim().to_owned();
    if zalo_recipient_id.len() > 256 {
        return Err(NotificationHubError::InvalidInput("invalid_recipient_id"));
    }

    let tg_current = load_channel_config_row(db, ExternalChannelKind::Telegram).await?;
    let zalo_current = load_channel_config_row(db, ExternalChannelKind::Zalo).await?;

    let tg_encrypted = resolve_patched_credential(
        secret_resolver,
        ExternalChannelKind::Telegram,
        tg_current.encrypted_credential,
        update.telegram.bot_token,
    )?;
    let zalo_encrypted = resolve_patched_credential(
        secret_resolver,
        ExternalChannelKind::Zalo,
        zalo_current.encrypted_credential,
        update.zalo.bot_token,
    )?;

    if update.telegram.enabled {
        if tg_chat_id.is_empty() {
            return Err(NotificationHubError::InvalidInput("missing_chat_id"));
        }
        if tg_encrypted.is_none()
            && !has_env_fallback(secret_resolver, ExternalChannelKind::Telegram)
        {
            return Err(NotificationHubError::InvalidInput("missing_bot_token"));
        }
    }

    if update.zalo.enabled {
        if zalo_recipient_id.is_empty() {
            return Err(NotificationHubError::InvalidInput("missing_recipient_id"));
        }
        if zalo_encrypted.is_none() && !has_env_fallback(secret_resolver, ExternalChannelKind::Zalo)
        {
            return Err(NotificationHubError::InvalidInput("missing_bot_token"));
        }
    }

    let tg_target_json = serde_json::to_string(&TelegramTargetConfig {
        chat_id: tg_chat_id,
        thread_id: update.telegram.thread_id,
    })
    .map_err(|_| NotificationHubError::InvalidInput("invalid_target_config"))?;
    let tg_topics_json =
        serde_json::to_string(&dedup_topics(&update.telegram.subscribed_topics))
            .map_err(|_| NotificationHubError::InvalidInput("invalid_subscribed_topics"))?;

    let zalo_target_json = serde_json::to_string(&ZaloTargetConfig {
        recipient_id: zalo_recipient_id,
    })
    .map_err(|_| NotificationHubError::InvalidInput("invalid_target_config"))?;
    let zalo_topics_json = serde_json::to_string(&dedup_topics(&update.zalo.subscribed_topics))
        .map_err(|_| NotificationHubError::InvalidInput("invalid_subscribed_topics"))?;

    let now_ms = now_millis();
    let new_revision = expected_revision + 1;

    let mut tx = db.pool().begin().await.map_err(map_sqlx)?;
    let state_update = sqlx::query(
        "UPDATE notification_settings_state \
         SET revision = ?, web_ui_toast_enabled = ?, retention_days = ?, updated_at_ms = ? \
         WHERE id = 1 AND revision = ?",
    )
    .bind(new_revision)
    .bind(i64::from(update.web_ui.toast_enabled))
    .bind(update.web_ui.retention_days)
    .bind(now_ms)
    .bind(expected_revision)
    .execute(&mut *tx)
    .await
    .map_err(map_sqlx)?;

    if state_update.rows_affected() == 0 {
        let _ = tx.rollback().await;
        audit_conflict(
            db.pool(),
            request_id.to_owned(),
            "notification_settings",
            1,
            expected_revision,
        )
        .await;
        return Err(NotificationHubError::RevisionConflict);
    }

    sqlx::query(
        "UPDATE notification_channel_configs \
         SET enabled = ?, target_json = ?, subscribed_topics_json = ?, encrypted_credential = ?, updated_at_ms = ? \
         WHERE channel_kind = 'telegram'",
    )
    .bind(i64::from(update.telegram.enabled))
    .bind(&tg_target_json)
    .bind(&tg_topics_json)
    .bind(&tg_encrypted)
    .bind(now_ms)
    .execute(&mut *tx)
    .await
    .map_err(map_sqlx)?;

    sqlx::query(
        "UPDATE notification_channel_configs \
         SET enabled = ?, target_json = ?, subscribed_topics_json = ?, encrypted_credential = ?, updated_at_ms = ? \
         WHERE channel_kind = 'zalo'",
    )
    .bind(i64::from(update.zalo.enabled))
    .bind(&zalo_target_json)
    .bind(&zalo_topics_json)
    .bind(&zalo_encrypted)
    .bind(now_ms)
    .execute(&mut *tx)
    .await
    .map_err(map_sqlx)?;

    audit(
        &mut *tx,
        request_id,
        "notification_settings",
        Some(1),
        "update",
        Some(expected_revision),
        Some(new_revision),
        AuditOutcome::Success,
        1,
    )
    .await
    .map_err(map_sqlx)?;

    tx.commit().await.map_err(map_sqlx)?;

    get_settings_from_store(db, secret_resolver).await
}

pub(super) async fn claim_due_outbox_batch(
    db: &Database,
    limit: u32,
) -> Result<Vec<ClaimedDeliveryAttempt>, NotificationHubError> {
    let now_ms = now_millis();
    let stale_cutoff = now_ms - STALE_SENDING_TIMEOUT_MS;

    // Recover any stale 'sending' rows from interrupted attempts.
    sqlx::query(
        "UPDATE notification_outbox \
         SET status = 'pending', next_attempt_at_ms = ?, updated_at_ms = ? \
         WHERE status = 'sending' AND updated_at_ms <= ?",
    )
    .bind(now_ms)
    .bind(now_ms)
    .bind(stale_cutoff)
    .execute(db.pool())
    .await
    .map_err(map_sqlx)?;

    let candidate_ids: Vec<String> = sqlx::query_scalar(
        "SELECT id FROM notification_outbox \
         WHERE status = 'pending' AND next_attempt_at_ms <= ? \
         ORDER BY next_attempt_at_ms ASC, created_at_ms ASC LIMIT ?",
    )
    .bind(now_ms)
    .bind(i64::from(limit))
    .fetch_all(db.pool())
    .await
    .map_err(map_sqlx)?;

    let mut claimed = Vec::new();
    for outbox_id in candidate_ids {
        let updated = sqlx::query(
            "UPDATE notification_outbox \
             SET status = 'sending', attempts = attempts + 1, updated_at_ms = ? \
             WHERE id = ? AND status = 'pending'",
        )
        .bind(now_ms)
        .bind(&outbox_id)
        .execute(db.pool())
        .await
        .map_err(map_sqlx)?;

        if updated.rows_affected() == 0 {
            continue;
        }

        let row = sqlx::query(
            "SELECT o.id, o.notification_id, o.channel_kind, o.attempts, o.max_attempts, \
                    i.topic, i.severity, i.title, i.body, i.source_kind, i.source_id, \
                    c.enabled AS channel_enabled, c.target_json, c.encrypted_credential \
             FROM notification_outbox o \
             JOIN notifications_inbox i ON i.id = o.notification_id \
             JOIN notification_channel_configs c ON c.channel_kind = o.channel_kind \
             WHERE o.id = ?",
        )
        .bind(&outbox_id)
        .fetch_optional(db.pool())
        .await
        .map_err(map_sqlx)?;

        let Some(row) = row else {
            continue;
        };
        let channel_str: String = row.get("channel_kind");
        let Some(channel) = ExternalChannelKind::parse(&channel_str) else {
            continue;
        };
        let topic_str: String = row.get("topic");
        let severity_str: String = row.get("severity");
        let source_kind_str: Option<String> = row.get("source_kind");

        claimed.push(ClaimedDeliveryAttempt {
            outbox_id: row.get("id"),
            notification_id: row.get("notification_id"),
            channel,
            attempts: row.get("attempts"),
            max_attempts: row.get("max_attempts"),
            content: NotificationContent {
                topic: NotificationTopic::parse(&topic_str)
                    .unwrap_or(NotificationTopic::TaskCompleted),
                severity: NotificationSeverity::parse(&severity_str)
                    .unwrap_or(NotificationSeverity::Info),
                title: row.get("title"),
                body: row.get("body"),
                source_kind: source_kind_str
                    .as_deref()
                    .and_then(NotificationSourceKind::parse),
                source_id: row.get("source_id"),
            },
            target_json: row.get("target_json"),
            encrypted_credential: row.get("encrypted_credential"),
            channel_enabled: row.get::<i64, _>("channel_enabled") != 0,
        });
    }

    Ok(claimed)
}

pub(super) async fn complete_outbox_sent(
    db: &Database,
    outbox_id: &str,
    external_message_id: Option<&str>,
) -> Result<(), NotificationHubError> {
    let now_ms = now_millis();
    sqlx::query(
        "UPDATE notification_outbox \
         SET status = 'sent', external_message_id = ?, last_error_code = NULL, updated_at_ms = ? \
         WHERE id = ?",
    )
    .bind(external_message_id)
    .bind(now_ms)
    .bind(outbox_id)
    .execute(db.pool())
    .await
    .map_err(map_sqlx)?;
    Ok(())
}

pub(super) async fn complete_outbox_failure(
    db: &Database,
    outbox_id: &str,
    error_code: &str,
    retry_delay_ms: Option<i64>,
) -> Result<(), NotificationHubError> {
    let now_ms = now_millis();
    if let Some(delay_ms) = retry_delay_ms {
        let next_at = now_ms + delay_ms.max(1_000);
        sqlx::query(
            "UPDATE notification_outbox \
             SET status = 'pending', next_attempt_at_ms = ?, last_error_code = ?, updated_at_ms = ? \
             WHERE id = ?",
        )
        .bind(next_at)
        .bind(error_code)
        .bind(now_ms)
        .bind(outbox_id)
        .execute(db.pool())
        .await
        .map_err(map_sqlx)?;
    } else {
        sqlx::query(
            "UPDATE notification_outbox \
             SET status = 'failed', last_error_code = ?, updated_at_ms = ? \
             WHERE id = ?",
        )
        .bind(error_code)
        .bind(now_ms)
        .bind(outbox_id)
        .execute(db.pool())
        .await
        .map_err(map_sqlx)?;
    }
    Ok(())
}

pub(super) async fn purge_expired_notifications(
    db: &Database,
) -> Result<u64, NotificationHubError> {
    let retention_days: i64 =
        sqlx::query_scalar("SELECT retention_days FROM notification_settings_state WHERE id = 1")
            .fetch_optional(db.pool())
            .await
            .map_err(map_sqlx)?
            .unwrap_or(30)
            .clamp(1, 365);

    let cutoff_ms = now_millis() - retention_days * 86_400_000;
    let deleted = sqlx::query("DELETE FROM notifications_inbox WHERE created_at_ms < ?")
        .bind(cutoff_ms)
        .execute(db.pool())
        .await
        .map_err(map_sqlx)?
        .rows_affected();
    Ok(deleted)
}
