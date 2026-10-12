//! Durable Device-owned notifications, shared by HTTP, tools and schedule delivery.
use super::{
    Database,
    audit::{AuditOutcome, audit},
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sqlx::FromRow;
use std::sync::Arc;

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
#[derive(Clone)]
pub struct DeviceNotifications {
    database: Database,
    clock: Arc<dyn Fn() -> i64 + Send + Sync>,
}
impl DeviceNotifications {
    pub fn new(database: Database) -> Self {
        Self::with_clock(database, Arc::new(|| chrono::Utc::now().timestamp_millis()))
    }
    pub fn with_clock(database: Database, clock: Arc<dyn Fn() -> i64 + Send + Sync>) -> Self {
        Self { database, clock }
    }
    #[allow(clippy::too_many_arguments)] // Audit correlation is distinct from the durable mutation identity.
    pub async fn create(
        &self,
        device_id: &str,
        caller: &str,
        request_id: &str,
        input: CreateReminder,
        source: &str,
        audit_request_id: &str,
    ) -> Result<Reminder, NotificationError> {
        if request_id.is_empty()
            || request_id.len() > 128
            || request_id.chars().any(char::is_control)
            || !matches!(source, "web" | "voice" | "web_assistant")
        {
            return Err(NotificationError::Invalid("validation_failed"));
        }
        let message = input.message.trim();
        if message.is_empty()
            || message.chars().count() > 1024
            || message.len() > 4096
            || message.chars().any(char::is_control)
        {
            return Err(NotificationError::Invalid("validation_failed"));
        }
        let payload = serde_json::to_vec(&input)
            .map_err(|_| NotificationError::Invalid("validation_failed"))?;
        let hash = Sha256::digest(&payload)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        let mut tx = self.database.pool().begin_with("BEGIN IMMEDIATE").await?;
        let (device_row_id, kind): (i64, String) =
            sqlx::query_as("SELECT id,kind FROM devices WHERE device_id=?")
                .bind(device_id)
                .fetch_optional(&mut *tx)
                .await?
                .ok_or(NotificationError::NotFound)?;
        if kind != "physical" {
            return Err(NotificationError::Invalid("unauthorized_target"));
        }
        if let Some((stored_hash,response)) = sqlx::query_as::<_,(String,String)>("SELECT payload_hash,response_json FROM reminder_mutation_keys WHERE caller_namespace=? AND request_id=? AND operation='create' AND device_row_id=?").bind(caller).bind(request_id).bind(device_row_id).fetch_optional(&mut *tx).await? {
            if stored_hash!=hash { return Err(NotificationError::Conflict("idempotency_conflict")); }
            return serde_json::from_str(&response).map_err(|_| NotificationError::Invalid("stored_response_invalid"));
        }
        let now = (self.clock)();
        // Mutation retries are supported for seven days; prune at most one quota-sized batch.
        sqlx::query("DELETE FROM reminder_mutation_keys WHERE rowid IN (SELECT rowid FROM reminder_mutation_keys WHERE created_at_ms < ? ORDER BY created_at_ms LIMIT 500)").bind(now.saturating_sub(7 * 24 * 60 * 60 * 1000)).execute(&mut *tx).await?;
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
        let count: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM device_reminders WHERE device_row_id=? AND status='pending'",
        )
        .bind(device_row_id)
        .fetch_one(&mut *tx)
        .await?;
        if count >= 500 {
            return Err(NotificationError::Conflict("reminder_quota_exceeded"));
        }
        sqlx::query("INSERT INTO device_reminder_sync(device_row_id,schedule_revision) VALUES (?,1) ON CONFLICT(device_row_id) DO UPDATE SET schedule_revision=schedule_revision+1").bind(device_row_id).execute(&mut *tx).await?;
        let revision: i64 = sqlx::query_scalar(
            "SELECT schedule_revision FROM device_reminder_sync WHERE device_row_id=?",
        )
        .bind(device_row_id)
        .fetch_one(&mut *tx)
        .await?;
        let reminder = Reminder {
            id: uuid::Uuid::new_v4().to_string(),
            device_id: device_id.into(),
            message: message.into(),
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
        sqlx::query("INSERT INTO device_reminders(id,device_row_id,message,due_at_ms,timezone,schedule_revision,source,created_at_ms,updated_at_ms) VALUES (?,?,?,?,?,?,?,?,?)").bind(&reminder.id).bind(device_row_id).bind(&reminder.message).bind(due_at_ms).bind(&reminder.timezone).bind(revision).bind(source).bind(now).bind(now).execute(&mut *tx).await?;
        let response = serde_json::to_string(&reminder)
            .map_err(|_| NotificationError::Invalid("validation_failed"))?;
        sqlx::query("INSERT INTO reminder_mutation_keys(caller_namespace,request_id,operation,device_row_id,payload_hash,response_json,created_at_ms) VALUES (?,?,'create',?,?,?,?)").bind(caller).bind(request_id).bind(device_row_id).bind(hash).bind(response).bind(now).execute(&mut *tx).await?;
        audit(
            &mut *tx,
            audit_request_id,
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
        Ok(reminder)
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
        let row: i64 = sqlx::query_scalar("SELECT id FROM devices WHERE device_id=?")
            .bind(device_id)
            .fetch_optional(&mut *tx)
            .await?
            .ok_or(NotificationError::NotFound)?;
        let mut items:Vec<Reminder>=sqlx::query_as("SELECT r.id,d.device_id,r.message,r.due_at_ms,r.timezone,r.status,r.row_revision,r.schedule_revision,r.attempts_used,r.created_at_ms,r.updated_at_ms FROM device_reminders r JOIN devices d ON d.id=r.device_row_id WHERE r.device_row_id=? AND (? IS NULL OR r.status=?) ORDER BY r.due_at_ms,r.id LIMIT ? OFFSET ?").bind(row).bind(status).bind(status).bind(page_size).bind(i64::from(page-1)*i64::from(page_size)).fetch_all(&mut *tx).await?;
        for item in &mut items {
            item.sync_status = "pending".into();
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
