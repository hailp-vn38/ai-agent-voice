//! Durable Device-owned notifications, shared by HTTP, tools and schedule delivery.
mod aggregate;
mod delivery;
mod mutations;
use super::{
    Database,
    audit::{AuditOutcome, audit},
};
pub use aggregate::AgentReminderQuery;
pub use delivery::*;
pub use mutations::UpdateReminder;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sqlx::FromRow;
use std::sync::Arc;

pub const SNAPSHOT_LIMIT: i64 = 64;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReminderSource {
    Web,
    Voice,
    WebAssistant,
}

impl ReminderSource {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Web => "web",
            Self::Voice => "voice",
            Self::WebAssistant => "web_assistant",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "web" => Some(Self::Web),
            "voice" => Some(Self::Voice),
            "web_assistant" => Some(Self::WebAssistant),
            _ => None,
        }
    }
}

#[derive(Clone, Debug)]
pub struct MutationContext<'a> {
    pub caller: &'a str,
    pub request_id: &'a str,
    pub audit_request_id: &'a str,
    pub source: ReminderSource,
    pub agent_scope: Option<&'a str>,
}

#[derive(Clone, Debug)]
pub struct ReminderSnapshot {
    pub revision: i64,
    pub items: Vec<SnapshotItem>,
    pub has_more: bool,
    pub server_time_ms: i64,
}

#[derive(Clone, Debug, Serialize, FromRow)]
pub struct SnapshotItem {
    pub id: String,
    pub due_at_ms: i64,
    pub revision: i64,
}

#[derive(Debug, thiserror::Error)]
pub enum NotificationError {
    #[error("{0}")]
    Invalid(&'static str),
    #[error("not_found")]
    NotFound,
    #[error("{0}")]
    Conflict(&'static str),
    #[error("device_notifications_unavailable")]
    Sql(#[from] sqlx::Error),
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CreateReminder {
    pub message: String,
    pub delay_seconds: Option<i64>,
    pub scheduled_at: Option<String>,
    pub timezone: Option<String>,
}
#[derive(Clone, Serialize, Deserialize, FromRow)]
pub struct Reminder {
    pub id: String,
    pub device_id: String,
    pub message: String,
    pub due_at_ms: i64,
    pub timezone: String,
    pub status: String,
    pub row_revision: i64,
    pub schedule_revision: i64,
    pub attempts_used: i64,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
    #[sqlx(skip)]
    pub sync_status: String,
}
#[derive(Clone, Debug)]
pub struct DueReminder {
    pub reminder_id: String,
    pub device_row_id: i64,
    pub message: String,
}
#[derive(Clone)]
pub struct DeviceNotifications {
    database: Database,
    agent_scope: Option<String>,
    clock: Arc<dyn Fn() -> i64 + Send + Sync>,
}
impl DeviceNotifications {
    pub fn new(database: Database) -> Self {
        Self::with_clock(database, Arc::new(|| chrono::Utc::now().timestamp_millis()))
    }
    pub fn with_clock(database: Database, clock: Arc<dyn Fn() -> i64 + Send + Sync>) -> Self {
        Self {
            database,
            clock,
            agent_scope: None,
        }
    }
    pub fn scoped_to_agent(mut self, key: &str) -> Self {
        self.agent_scope = Some(key.into());
        self
    }
    pub fn clock_now(&self) -> i64 {
        (self.clock)()
    }
    pub fn database(&self) -> &Database {
        &self.database
    }
    /// Reads the fixed cache window in one transaction. The caller owns the ephemeral snapshot
    /// token because it is tied to a current WebSocket session, never persisted identity.
    pub async fn snapshot(
        &self,
        device_row_id: i64,
    ) -> Result<ReminderSnapshot, NotificationError> {
        let mut tx = self.database.pool().begin().await?;
        let revision = sqlx::query_scalar::<_, i64>(
            "SELECT schedule_revision FROM device_reminder_sync WHERE device_row_id=?",
        )
        .bind(device_row_id)
        .fetch_optional(&mut *tx)
        .await?
        .unwrap_or(0);
        let rows: Vec<SnapshotItem> = sqlx::query_as("SELECT id,due_at_ms,schedule_revision AS revision FROM device_reminders WHERE device_row_id=? AND status='pending' ORDER BY due_at_ms,id LIMIT ?")
            .bind(device_row_id).bind(SNAPSHOT_LIMIT + 1).fetch_all(&mut *tx).await?;
        let has_more = rows.len() as i64 > SNAPSHOT_LIMIT;
        let items = rows.into_iter().take(SNAPSHOT_LIMIT as usize).collect();
        tx.commit().await?;
        Ok(ReminderSnapshot {
            revision,
            items,
            has_more,
            server_time_ms: (self.clock)(),
        })
    }

    pub async fn acknowledge_snapshot(
        &self,
        device_row_id: i64,
        revision: i64,
        item_ids: &[String],
        next_wake_at_ms: Option<i64>,
    ) -> Result<(), NotificationError> {
        let now = (self.clock)();
        let mut tx = self.database.pool().begin_with("BEGIN IMMEDIATE").await?;
        let current = sqlx::query_scalar::<_, i64>(
            "SELECT schedule_revision FROM device_reminder_sync WHERE device_row_id=?",
        )
        .bind(device_row_id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(NotificationError::NotFound)?;
        if revision > current {
            return Err(NotificationError::Invalid("invalid_snapshot"));
        }
        let previous = sqlx::query_scalar::<_, i64>(
            "SELECT last_acked_revision FROM device_reminder_sync WHERE device_row_id=?",
        )
        .bind(device_row_id)
        .fetch_one(&mut *tx)
        .await?;
        if revision >= previous {
            sqlx::query("DELETE FROM device_reminder_cached_items WHERE device_row_id=?")
                .bind(device_row_id)
                .execute(&mut *tx)
                .await?;
            for id in item_ids {
                sqlx::query("INSERT INTO device_reminder_cached_items(device_row_id,reminder_id,snapshot_revision) SELECT ?,id,? FROM device_reminders WHERE id=? AND device_row_id=? AND status='pending'")
                    .bind(device_row_id).bind(revision).bind(id).bind(device_row_id).execute(&mut *tx).await?;
            }
            sqlx::query("UPDATE device_reminder_sync SET last_acked_revision=?,last_acked_at_ms=?,last_reported_wake_at_ms=? WHERE device_row_id=?")
                .bind(revision).bind(now).bind(next_wake_at_ms).bind(device_row_id).execute(&mut *tx).await?;
        }
        tx.commit().await?;
        Ok(())
    }

    pub async fn create_with_context(
        &self,
        device_id: &str,
        ctx: &MutationContext<'_>,
        input: CreateReminder,
    ) -> Result<(Reminder, i64), NotificationError> {
        if ctx.request_id.is_empty()
            || ctx.request_id.len() > 128
            || ctx.request_id.chars().any(char::is_control)
        {
            return Err(NotificationError::Invalid("validation_failed"));
        }
        let scope = ctx.agent_scope.or(self.agent_scope.as_deref());
        let payload = serde_json::to_vec(&input)
            .map_err(|_| NotificationError::Invalid("validation_failed"))?;
        let hash = Sha256::digest(&payload)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        let mut tx = self.database.pool().begin_with("BEGIN IMMEDIATE").await?;
        let (device_row_id, kind): (i64, String) =
            sqlx::query_as("SELECT d.id,d.kind FROM devices d JOIN agents a ON a.id=d.agent_id WHERE d.device_id=? AND (? IS NULL OR a.key=?)")
                .bind(device_id)
                .bind(scope).bind(scope)
                .fetch_optional(&mut *tx)
                .await?
                .ok_or(NotificationError::NotFound)?;
        if kind != "physical" {
            return Err(NotificationError::Invalid("unauthorized_target"));
        }
        if let Some((stored_hash, response)) = sqlx::query_as::<_, (String, String)>("SELECT payload_hash,response_json FROM reminder_mutation_keys WHERE caller_namespace=? AND request_id=? AND operation='create' AND device_row_id=?").bind(ctx.caller).bind(ctx.request_id).bind(device_row_id).fetch_optional(&mut *tx).await? {
            if stored_hash != hash {
                return Err(NotificationError::Conflict("idempotency_conflict"));
            }
            let mut reminder: Reminder = serde_json::from_str(&response)
                .map_err(|_| NotificationError::Invalid("stored_response_invalid"))?;
            reminder.sync_status = sync_status(&mut tx, device_row_id, &reminder).await?;
            tx.commit().await?;
            return Ok((reminder, device_row_id));
        }
        let now = (self.clock)();
        prune_keys(&mut tx, now).await?;
        let (message, due_at_ms, timezone) = validate_input(&input, now)?;
        let count: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM device_reminders WHERE device_row_id=? AND status='pending'",
        )
        .bind(device_row_id)
        .fetch_one(&mut *tx)
        .await?;
        if count >= 500 {
            return Err(NotificationError::Conflict("reminder_quota_exceeded"));
        }
        let revision = delivery::bump_schedule_revision(&mut tx, device_row_id).await?;
        let reminder = Reminder {
            id: uuid::Uuid::new_v4().to_string(),
            device_id: device_id.into(),
            message,
            due_at_ms,
            timezone,
            status: "pending".into(),
            row_revision: 1,
            schedule_revision: revision,
            attempts_used: 0,
            created_at_ms: now,
            updated_at_ms: now,
            sync_status: "pending".into(),
        };
        sqlx::query("INSERT INTO device_reminders(id,device_row_id,message,due_at_ms,timezone,schedule_revision,source,created_at_ms,updated_at_ms) VALUES (?,?,?,?,?,?,?,?,?)").bind(&reminder.id).bind(device_row_id).bind(&reminder.message).bind(due_at_ms).bind(&reminder.timezone).bind(revision).bind(ctx.source.as_str()).bind(now).bind(now).execute(&mut *tx).await?;
        let response = serde_json::to_string(&reminder)
            .map_err(|_| NotificationError::Invalid("validation_failed"))?;
        sqlx::query("INSERT INTO reminder_mutation_keys(caller_namespace,request_id,operation,device_row_id,payload_hash,response_json,created_at_ms) VALUES (?,?,'create',?,?,?,?)").bind(ctx.caller).bind(ctx.request_id).bind(device_row_id).bind(hash).bind(response).bind(now).execute(&mut *tx).await?;
        audit(
            &mut *tx,
            ctx.audit_request_id,
            "device_reminder_schedule",
            Some(device_row_id),
            "create_reminder",
            Some(revision - 1),
            Some(revision),
            AuditOutcome::Success,
            1,
        )
        .await?;
        tx.commit().await?;
        Ok((reminder, device_row_id))
    }

    pub async fn list(
        &self,
        device_id: &str,
        status: Option<&str>,
        page: u32,
        page_size: u32,
    ) -> Result<serde_json::Value, NotificationError> {
        if status.is_some_and(|s| !matches!(s, "pending" | "completed" | "cancelled" | "expired"))
            || page == 0
            || !(1..=200).contains(&page_size)
        {
            return Err(NotificationError::Invalid("invalid_query"));
        }
        let mut tx = self.database.pool().begin().await?;
        let row: i64 = sqlx::query_scalar("SELECT d.id FROM devices d JOIN agents a ON a.id=d.agent_id WHERE d.device_id=? AND (? IS NULL OR a.key=?)")
            .bind(device_id)
            .bind(self.agent_scope.as_deref()).bind(self.agent_scope.as_deref())
            .fetch_optional(&mut *tx)
            .await?
            .ok_or(NotificationError::NotFound)?;
        let mut items:Vec<Reminder>=sqlx::query_as("SELECT r.id,d.device_id,r.message,r.due_at_ms,r.timezone,r.status,r.row_revision,r.schedule_revision,r.attempts_used,r.created_at_ms,r.updated_at_ms FROM device_reminders r JOIN devices d ON d.id=r.device_row_id WHERE r.device_row_id=? AND (? IS NULL OR r.status=?) ORDER BY r.due_at_ms,r.id LIMIT ? OFFSET ?").bind(row).bind(status).bind(status).bind(page_size).bind(i64::from(page-1)*i64::from(page_size)).fetch_all(&mut *tx).await?;
        for item in &mut items {
            item.sync_status = sync_status(&mut tx, row, item).await?;
        }
        let (revision,acked,at,wake)=sqlx::query_as::<_,(i64,i64,Option<i64>,Option<i64>)>("SELECT schedule_revision,last_acked_revision,last_acked_at_ms,last_reported_wake_at_ms FROM device_reminder_sync WHERE device_row_id=?").bind(row).fetch_optional(&mut *tx).await?.unwrap_or((0,0,None,None));
        let total: i64 = sqlx::query_scalar("SELECT count(*) FROM device_reminders WHERE device_row_id=? AND (? IS NULL OR status=?)").bind(row).bind(status).bind(status).fetch_one(&mut *tx).await?;
        tx.commit().await?;
        Ok(
            serde_json::json!({"items":items,"page":page,"page_size":page_size,"total":total,"has_more":i64::from(page)*i64::from(page_size)<total,"schedule_summary":{"schedule_revision":revision,"last_acked_revision":acked,"last_acked_at_ms":at,"last_reported_wake_at_ms":wake,"session_connected":null,"reminder_capable":null}}),
        )
    }
    pub async fn grant(&self, key: &str) -> Result<(bool, i64), NotificationError> {
        sqlx::query_as("SELECT reminders_enabled,reminder_policy_revision FROM agents WHERE key=?")
            .bind(key)
            .fetch_optional(self.database.pool())
            .await?
            .ok_or(NotificationError::NotFound)
    }

    pub async fn is_active_physical_device_for_agent(
        &self,
        device_row_id: i64,
        device_id: &str,
        agent_id: i64,
    ) -> Result<bool, NotificationError> {
        Ok(sqlx::query_scalar::<_, i64>(
            "SELECT 1 FROM devices WHERE id = ? AND device_id = ? AND kind = 'physical' AND enabled = 1 AND agent_id = ?",
        )
        .bind(device_row_id)
        .bind(device_id)
        .bind(agent_id)
        .fetch_optional(self.database.pool())
        .await?
        .is_some())
    }
    pub async fn set_grant(
        &self,
        key: &str,
        enabled: bool,
        revision: i64,
        request_id: &str,
    ) -> Result<i64, NotificationError> {
        let mut tx = self.database.pool().begin_with("BEGIN IMMEDIATE").await?;
        let row=sqlx::query("UPDATE agents SET reminders_enabled=?,reminder_policy_revision=reminder_policy_revision+1 WHERE key=? AND reminder_policy_revision=?").bind(enabled).bind(key).bind(revision).execute(&mut *tx).await?;
        if row.rows_affected() != 1 {
            let id = sqlx::query_scalar::<_, i64>("SELECT id FROM agents WHERE key=?")
                .bind(key)
                .fetch_optional(&mut *tx)
                .await?
                .ok_or(NotificationError::NotFound)?;
            tx.rollback().await?;
            super::audit::audit_conflict_action(
                self.database.pool(),
                request_id.into(),
                "agent",
                id,
                revision,
                "reminder_policy",
            )
            .await;
            return Err(NotificationError::Conflict("revision_conflict"));
        }
        let id: i64 = sqlx::query_scalar("SELECT id FROM agents WHERE key=?")
            .bind(key)
            .fetch_one(&mut *tx)
            .await?;
        audit(
            &mut *tx,
            request_id,
            "agent",
            Some(id),
            "reminder_policy",
            Some(revision),
            Some(revision + 1),
            AuditOutcome::Success,
            1,
        )
        .await?;
        tx.commit().await?;
        Ok(revision + 1)
    }
}

pub(super) async fn sync_status(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    device: i64,
    item: &Reminder,
) -> Result<String, sqlx::Error> {
    let cached = sqlx::query_scalar::<_, i64>("SELECT snapshot_revision FROM device_reminder_cached_items WHERE device_row_id=? AND reminder_id=?")
        .bind(device).bind(&item.id).fetch_optional(&mut **tx).await?;
    Ok(match cached {
        Some(revision) if revision >= item.schedule_revision => "acked".into(),
        _ => {
            let position = sqlx::query_scalar::<_, i64>("SELECT count(*) FROM device_reminders WHERE device_row_id=? AND status='pending' AND (due_at_ms<? OR (due_at_ms=? AND id<?))")
                .bind(device).bind(item.due_at_ms).bind(item.due_at_ms).bind(&item.id).fetch_one(&mut **tx).await?;
            if position >= SNAPSHOT_LIMIT {
                "out_of_window".into()
            } else {
                "pending".into()
            }
        }
    })
}

async fn prune_keys(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    now: i64,
) -> Result<(), sqlx::Error> {
    // Mutation retries are supported for seven days; prune at most one quota-sized batch.
    sqlx::query("DELETE FROM reminder_mutation_keys WHERE rowid IN (SELECT rowid FROM reminder_mutation_keys WHERE created_at_ms < ? ORDER BY created_at_ms LIMIT 500)").bind(now.saturating_sub(7*24*60*60*1000)).execute(&mut **tx).await?;
    Ok(())
}
fn validate_input(
    input: &CreateReminder,
    now: i64,
) -> Result<(String, i64, String), NotificationError> {
    let message = input.message.trim();
    if message.is_empty()
        || message.chars().count() > 1024
        || message.len() > 4096
        || message.chars().any(char::is_control)
    {
        return Err(NotificationError::Invalid("validation_failed"));
    }
    let due_at_ms = match (&input.delay_seconds, &input.scheduled_at) {
        (Some(delay), None) if (1..=31_536_000).contains(delay) => now
            .checked_add(delay * 1000)
            .ok_or(NotificationError::Invalid("validation_failed"))?,
        (None, Some(at)) => chrono::DateTime::parse_from_rfc3339(at)
            .map_err(|_| NotificationError::Invalid("timezone_required"))?
            .timestamp_millis(),
        _ => return Err(NotificationError::Invalid("validation_failed")),
    };
    let timezone = input.timezone.clone().unwrap_or_else(|| {
        if let Some(at) = &input.scheduled_at {
            chrono::DateTime::parse_from_rfc3339(at)
                .unwrap()
                .offset()
                .to_string()
        } else {
            "UTC".into()
        }
    });
    if timezone.is_empty() || timezone.len() > 128 || timezone.chars().any(char::is_control) {
        return Err(NotificationError::Invalid("validation_failed"));
    }
    Ok((message.into(), due_at_ms, timezone))
}
