//! Short-lived control-plane enrollment records.  This module never resolves a session profile or
//! constructs a provider: registration is observed here and Voice admission remains its own seam.

use super::{Database, DatabaseError, map_sqlx_error};
use crate::config::EnrollmentConfig;
use sqlx::Row;
use std::time::Duration;
use thiserror::Error;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeviceRegistration {
    Unknown,
    Registered,
    Blocked,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EnrollmentStatus {
    Pending,
    Registered,
    Blocked,
    Expired,
    Unavailable,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EnrollmentPending {
    pub code: String,
    pub challenge: String,
    pub expires_at: i64,
}

#[derive(Clone, Debug)]
pub struct EnrollmentRequest {
    pub device_id: String,
    pub client_id: String,
    pub metadata_json: String,
    pub now: i64,
    pub ttl_seconds: u64,
    /// Values are generated outside the SQLite write transaction.  A caller supplies at most eight
    /// independent CSPRNG candidates; this method retries only a code collision, never BUSY.
    pub candidates: Vec<(String, String)>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EnrollmentClaim {
    Pending(EnrollmentPending),
    Registered,
    Blocked,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum EnrollmentCreateError {
    #[error("enrollment_capacity_exceeded")]
    Capacity,
    #[error("enrollment_code_unavailable")]
    CodeUnavailable,
    #[error("database_unavailable")]
    Database,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum EnrollmentClaimError {
    #[error("enrollment_code_invalid")]
    Invalid,
    #[error("enrollment_code_expired")]
    Expired,
    #[error("enrollment_already_claimed")]
    AlreadyClaimed,
    #[error("enrollment_cancelled")]
    Cancelled,
    #[error("database_unavailable")]
    Database,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct EnrollmentPurge {
    pub expired: u64,
    pub deleted: u64,
}

/// Application-owned bounded enrollment maintenance.  It is independent from transcript capture.
pub struct EnrollmentCleaner {
    task: JoinHandle<()>,
}

impl EnrollmentCleaner {
    pub fn start(
        database: &Database,
        config: &EnrollmentConfig,
        shutdown: CancellationToken,
    ) -> Self {
        let database = database.clone();
        let retention_seconds = config.retention_seconds;
        let interval = Duration::from_secs(config.cleanup_interval_seconds);
        let task = tokio::spawn(async move {
            let mut schedule = tokio::time::interval(interval);
            schedule.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                tokio::select! {
                    _ = shutdown.cancelled() => break,
                    _ = schedule.tick() => {
                        if let Some(now) = crate::database::unix_seconds()
                            && let Err(error) = database.purge_enrollments(now, retention_seconds, 256).await {
                            tracing::warn!(event = "enrollment_cleanup_skipped", reason = %error, "Enrollment cleanup pass was skipped");
                        }
                    }
                }
            }
        });
        Self { task }
    }
}

impl Drop for EnrollmentCleaner {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl Database {
    /// One read snapshot: Device rows are authoritative after either claim or manual create.
    pub async fn enrollment_status(
        &self,
        device_id: &str,
        code: &str,
        now: i64,
    ) -> Result<EnrollmentStatus, DatabaseError> {
        let row: (Option<i64>, Option<i64>, Option<String>, Option<i64>) = sqlx::query_as(
            "SELECT d.enabled,a.enabled,e.status,e.expires_at FROM (SELECT 1) seed \
             LEFT JOIN devices d ON d.device_id=? LEFT JOIN agents a ON a.id=d.agent_id \
             LEFT JOIN device_enrollments e ON e.device_id=? AND e.code=?",
        )
        .bind(device_id)
        .bind(device_id)
        .bind(code)
        .fetch_one(&self.pool)
        .await
        .map_err(map_sqlx_error)?;
        Ok(match row {
            (Some(1), Some(1), _, _) => EnrollmentStatus::Registered,
            (Some(_), _, _, _) => EnrollmentStatus::Blocked,
            (None, _, Some(status), Some(expiry)) if status == "pending" && now < expiry => {
                EnrollmentStatus::Pending
            }
            _ => EnrollmentStatus::Expired,
        })
    }

    /// A deliberately small read used by OTA and activation polling.  Orphaned/corrupt rows fail
    /// closed instead of looking unknown and obtaining another enrollment code.
    pub async fn device_registration(
        &self,
        device_id: &str,
    ) -> Result<DeviceRegistration, DatabaseError> {
        let row = sqlx::query(
            "SELECT d.enabled AS device_enabled, a.enabled AS agent_enabled \
             FROM devices d LEFT JOIN agents a ON a.id=d.agent_id WHERE d.device_id=?",
        )
        .bind(device_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(map_sqlx_error)?;
        Ok(match row {
            None => DeviceRegistration::Unknown,
            Some(row)
                if row.get::<i64, _>("device_enabled") == 1
                    && row.try_get::<i64, _>("agent_enabled").unwrap_or(0) == 1 =>
            {
                DeviceRegistration::Registered
            }
            Some(_) => DeviceRegistration::Blocked,
        })
    }

    pub async fn get_or_create_enrollment(
        &self,
        request: EnrollmentRequest,
        max_pending: u32,
    ) -> Result<EnrollmentClaim, EnrollmentCreateError> {
        let expires_at = request
            .now
            .checked_add(i64::try_from(request.ttl_seconds).unwrap_or(i64::MAX))
            .ok_or(EnrollmentCreateError::Database)?;
        let mut tx = self
            .pool
            .begin_with("BEGIN IMMEDIATE")
            .await
            .map_err(|_| EnrollmentCreateError::Database)?;
        let registration = registration_in_tx(&mut tx, &request.device_id)
            .await
            .map_err(|_| EnrollmentCreateError::Database)?;
        match registration {
            DeviceRegistration::Registered => return Ok(EnrollmentClaim::Registered),
            DeviceRegistration::Blocked => return Ok(EnrollmentClaim::Blocked),
            DeviceRegistration::Unknown => {}
        }
        let existing = sqlx::query_as::<_, (String, String, i64)>(
            "SELECT code,challenge,expires_at FROM device_enrollments WHERE device_id=? AND status='pending'",
        )
        .bind(&request.device_id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(|_| EnrollmentCreateError::Database)?;
        if let Some((code, challenge, pending_expires_at)) = existing {
            if request.now < pending_expires_at {
                tx.commit()
                    .await
                    .map_err(|_| EnrollmentCreateError::Database)?;
                return Ok(EnrollmentClaim::Pending(EnrollmentPending {
                    code,
                    challenge,
                    expires_at: pending_expires_at,
                }));
            }
            sqlx::query(
                "UPDATE device_enrollments SET status='expired',terminal_at=expires_at \
                 WHERE device_id=? AND status='pending' AND expires_at<=?",
            )
            .bind(&request.device_id)
            .bind(request.now)
            .execute(&mut *tx)
            .await
            .map_err(|_| EnrollmentCreateError::Database)?;
        }
        let active: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM device_enrollments WHERE status='pending' AND expires_at>?",
        )
        .bind(request.now)
        .fetch_one(&mut *tx)
        .await
        .map_err(|_| EnrollmentCreateError::Database)?;
        if active >= i64::from(max_pending) {
            return Err(EnrollmentCreateError::Capacity);
        }
        for (code, challenge) in request.candidates.into_iter().take(8) {
            let inserted = sqlx::query(
                "INSERT INTO device_enrollments \
                 (device_id,client_id,code,challenge,metadata_json,created_at,expires_at) \
                 VALUES (?,?,?,?,?,?,?) ON CONFLICT(code) DO NOTHING",
            )
            .bind(&request.device_id)
            .bind(&request.client_id)
            .bind(&code)
            .bind(&challenge)
            .bind(&request.metadata_json)
            .bind(request.now)
            .bind(expires_at)
            .execute(&mut *tx)
            .await
            .map_err(|_| EnrollmentCreateError::Database)?;
            if inserted.rows_affected() == 1 {
                tx.commit()
                    .await
                    .map_err(|_| EnrollmentCreateError::Database)?;
                return Ok(EnrollmentClaim::Pending(EnrollmentPending {
                    code,
                    challenge,
                    expires_at,
                }));
            }
        }
        Err(EnrollmentCreateError::CodeUnavailable)
    }

    pub async fn activation_pending(
        &self,
        device_id: &str,
        now: i64,
    ) -> Result<Option<bool>, DatabaseError> {
        let row: Option<(String, i64)> = sqlx::query_as(
            "SELECT status,expires_at FROM device_enrollments WHERE device_id=? ORDER BY id DESC LIMIT 1",
        )
        .bind(device_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(map_sqlx_error)?;
        Ok(row.map(|(status, expires_at)| status == "pending" && now < expires_at))
    }

    pub async fn cancel_pending_enrollment(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
        device_id: &str,
        now: i64,
    ) -> Result<(), sqlx::Error> {
        sqlx::query(
            "UPDATE device_enrollments SET status='cancelled',terminal_at=? \
             WHERE device_id=? AND status='pending'",
        )
        .bind(now)
        .bind(device_id)
        .execute(&mut **tx)
        .await
        .map(|_| ())
    }

    pub async fn purge_enrollments(
        &self,
        now: i64,
        retention_seconds: u64,
        limit: u32,
    ) -> Result<EnrollmentPurge, DatabaseError> {
        let retention_cutoff =
            now.saturating_sub(i64::try_from(retention_seconds).unwrap_or(i64::MAX));
        let mut tx = self
            .pool
            .begin_with("BEGIN IMMEDIATE")
            .await
            .map_err(map_sqlx_error)?;
        let expired = sqlx::query(
            "UPDATE device_enrollments SET status='expired',terminal_at=expires_at WHERE id IN \
             (SELECT id FROM device_enrollments WHERE status='pending' AND expires_at<=? ORDER BY expires_at,id LIMIT ?)",
        ).bind(now).bind(i64::from(limit)).execute(&mut *tx).await.map_err(map_sqlx_error)?.rows_affected();
        let deleted = sqlx::query(
            "DELETE FROM device_enrollments WHERE id IN \
             (SELECT id FROM device_enrollments WHERE status<>'pending' AND terminal_at<=? ORDER BY terminal_at,id LIMIT ?)",
        ).bind(retention_cutoff).bind(i64::from(limit)).execute(&mut *tx).await.map_err(map_sqlx_error)?.rows_affected();
        tx.commit().await.map_err(map_sqlx_error)?;
        Ok(EnrollmentPurge { expired, deleted })
    }
}

async fn registration_in_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    device_id: &str,
) -> Result<DeviceRegistration, sqlx::Error> {
    let row = sqlx::query(
        "SELECT d.enabled AS device_enabled,a.enabled AS agent_enabled FROM devices d \
         LEFT JOIN agents a ON a.id=d.agent_id WHERE d.device_id=?",
    )
    .bind(device_id)
    .fetch_optional(&mut **tx)
    .await?;
    Ok(match row {
        None => DeviceRegistration::Unknown,
        Some(row)
            if row.get::<i64, _>("device_enabled") == 1
                && row.try_get::<i64, _>("agent_enabled").unwrap_or(0) == 1 =>
        {
            DeviceRegistration::Registered
        }
        Some(_) => DeviceRegistration::Blocked,
    })
}

use super::{
    audit::{AuditOutcome, audit},
    writes::WriteError,
};
pub(crate) struct AdminEnrollmentClaim<'a> {
    pub code: &'a str,
    pub agent_key: &'a str,
    pub name: Option<&'a str>,
    pub template_key: Option<&'a str>,
}
impl Database {
    pub(crate) async fn claim_device_enrollment(
        &self,
        input: AdminEnrollmentClaim<'_>,
        request_id: &str,
    ) -> Result<super::devices::Device, WriteError> {
        let pool = &self.pool;
        let mut tx = match pool.begin_with("BEGIN IMMEDIATE").await {
            Ok(tx) => tx,
            Err(error_value) => return Err(WriteError::Sql(error_value)),
        };
        let row: Option<(String, String, i64, String)> = match sqlx::query_as(
            "SELECT status,device_id,expires_at,metadata_json FROM device_enrollments WHERE code=?",
        )
        .bind(input.code)
        .fetch_optional(&mut *tx)
        .await
        {
            Ok(row) => row,
            Err(error_value) => return Err(WriteError::Sql(error_value)),
        };
        let Some((status, device_identity, expires_at, metadata_json)) = row else {
            return Err(WriteError::Missing("enrollment_code_invalid"));
        };
        match status.as_str() {
            "claimed" => return Err(WriteError::Conflict("enrollment_already_claimed")),
            "cancelled" => return Err(WriteError::Conflict("enrollment_cancelled")),
            "expired" => return Err(WriteError::Gone("enrollment_code_expired")),
            "pending" if crate::database::unix_seconds().unwrap_or_default() >= expires_at => {
                return Err(WriteError::Gone("enrollment_code_expired"));
            }
            "pending" => {}
            _ => {
                return Err(WriteError::Unavailable);
            }
        }
        let agent: Result<(i64, i64), _> =
            sqlx::query_as("SELECT id,enabled FROM agents WHERE key=?")
                .bind(input.agent_key)
                .fetch_one(&mut *tx)
                .await;
        let (agent_id, enabled) = match agent {
            Ok(value) => value,
            Err(sqlx::Error::RowNotFound) => {
                return Err(WriteError::Invalid("invalid_agent"));
            }
            Err(error_value) => return Err(WriteError::Sql(error_value)),
        };
        if enabled != 1 {
            return Err(WriteError::Invalid("invalid_agent"));
        }
        let template_id =
            match super::devices::template_override_id(&mut tx, agent_id, input.template_key).await
            {
                Ok(value) => value,
                Err(sqlx::Error::RowNotFound) => {
                    return Err(WriteError::Invalid("invalid_template_override"));
                }
                Err(error_value) => return Err(WriteError::Sql(error_value)),
            };
        let existing: Result<Option<(i64,)>, _> =
            sqlx::query_as("SELECT id FROM devices WHERE device_id=?")
                .bind(&device_identity)
                .fetch_optional(&mut *tx)
                .await;
        match existing {
            Ok(None) => {}
            Ok(Some(_)) => return Err(WriteError::Conflict("device_already_registered")),
            Err(error_value) => return Err(WriteError::Sql(error_value)),
        }
        let timestamp = crate::database::unix_seconds().unwrap_or_default();
        let device_row = match sqlx::query("INSERT INTO devices (device_id,agent_id,template_id,name,metadata_json,created_at,updated_at) VALUES (?,?,?,?,?,?,?)")
        .bind(&device_identity).bind(agent_id).bind(template_id).bind(input.name).bind(metadata_json).bind(timestamp).bind(timestamp).execute(&mut *tx).await {
        Ok(result) => result.last_insert_rowid(),
        Err(error_value) if super::is_busy(&error_value) => return Err(WriteError::Sql(error_value)),
        Err(error_value) if is_unique_constraint(&error_value) => {
            return Err(WriteError::Conflict("device_already_registered"));
        }
        Err(error_value) => return Err(WriteError::Sql(error_value)),
    };
        let claimed = match sqlx::query("UPDATE device_enrollments SET status='claimed',claimed_device_id=?,terminal_at=? WHERE code=? AND status='pending' AND expires_at>?")
        .bind(device_row).bind(timestamp).bind(input.code).bind(timestamp).execute(&mut *tx).await {
        Ok(result) => result.rows_affected(), Err(error_value) => return Err(WriteError::Sql(error_value)),
    };
        if claimed != 1 {
            return Err(WriteError::Conflict("enrollment_already_claimed"));
        }
        if audit(
            &mut *tx,
            request_id,
            "device",
            Some(device_row),
            "create",
            None,
            Some(1),
            AuditOutcome::Success,
            1,
        )
        .await
        .is_err()
            || audit(
                &mut *tx,
                request_id,
                "device_enrollment",
                None,
                "claim",
                None,
                None,
                AuditOutcome::Success,
                1,
            )
            .await
            .is_err()
        {
            return Err(WriteError::Unavailable);
        }
        let device: super::devices::Device = match sqlx::query_as("SELECT d.id,d.device_id,a.key AS agent_key,t.key AS template_key,d.name,d.description,d.enabled,d.metadata_json,d.revision,d.created_at,d.updated_at FROM devices d JOIN agents a ON a.id=d.agent_id LEFT JOIN agent_templates t ON t.id=d.template_id WHERE d.id=?")
        .bind(device_row).fetch_one(&mut *tx).await { Ok(device) => device, Err(error_value) => return Err(WriteError::Sql(error_value)) };
        if tx.commit().await.is_err() {
            return Err(WriteError::Unavailable);
        }
        Ok(device)
    }
}
fn is_unique_constraint(error: &sqlx::Error) -> bool {
    matches!(error, sqlx::Error::Database(database_error) if matches!(database_error.code().as_deref(), Some("1555" | "2067" | "SQLITE_CONSTRAINT_UNIQUE")))
}
