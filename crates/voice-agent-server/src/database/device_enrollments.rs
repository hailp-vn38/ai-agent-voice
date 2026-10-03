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
                        if let Some(now) = super::unix_seconds() {
                            if let Err(error) = database.purge_enrollments(now, retention_seconds, 256).await {
                                tracing::warn!(event = "enrollment_cleanup_skipped", reason = %error, "Enrollment cleanup pass was skipped");
                            }
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
