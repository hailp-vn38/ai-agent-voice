use super::*;

pub const MAX_REMINDER_ATTEMPTS: i64 = 3;
pub const CLAIM_PRE_START_LEASE_MS: i64 = 10_000;
pub const MIN_ACK_TIMEOUT_MS: i64 = 15_000;
pub const MAX_ACK_TIMEOUT_MS: i64 = 120_000;
pub const SINK_LATENCY_BUDGET_MS: i64 = 10_000;
pub const LATENESS_GRACE_MS: i64 = 86_400_000;
pub const RETRY_BACKOFF_MS: [i64; 3] = [5_000, 15_000, 60_000];

pub fn compute_ack_timeout_ms(estimated_audio_ms: i64) -> i64 {
    (estimated_audio_ms.saturating_add(SINK_LATENCY_BUDGET_MS))
        .clamp(MIN_ACK_TIMEOUT_MS, MAX_ACK_TIMEOUT_MS)
}

pub fn retry_backoff_ms(attempts_used: i64) -> i64 {
    let idx = (attempts_used.saturating_sub(1) as usize).min(RETRY_BACKOFF_MS.len() - 1);
    RETRY_BACKOFF_MS[idx]
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CompleteAttemptOutcome {
    pub schedule_revision: i64,
    pub duplicate_ack: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FailAttemptOutcome {
    pub expired: bool,
    pub schedule_revision: i64,
}

impl DeviceNotifications {
    /// Atomically claims the earliest due reminder for a specific online, reminder-capable Device
    /// without incrementing `attempts_used` yet. The attempt is created with `status = 'claimed'`
    /// and `started_at_ms = NULL`. Once the session actor admits playback, callers invoke
    /// [`Self::mark_attempt_started`] to revalidate and increment `attempts_used` exactly once.
    pub async fn claim_next_due_for_device(
        &self,
        device_row_id: i64,
        session_id: &str,
    ) -> Result<Option<(DueReminder, String)>, NotificationError> {
        let now = (self.clock)();
        let has_due: Option<i64> = sqlx::query_scalar(
            "SELECT 1 FROM device_reminders r \
             JOIN devices d ON d.id = r.device_row_id \
             WHERE r.device_row_id = ? \
               AND d.enabled = 1 \
               AND r.status = 'pending' \
               AND r.due_at_ms <= ? \
               AND (r.next_attempt_at_ms IS NULL OR r.next_attempt_at_ms <= ?) \
               AND r.attempts_used < ? \
               AND r.due_at_ms + ? > ? \
               AND NOT EXISTS ( \
                   SELECT 1 FROM reminder_delivery_attempts a \
                   WHERE a.reminder_id = r.id \
                     AND a.status = 'claimed' \
                     AND a.lease_until_ms > ? \
               ) \
             LIMIT 1",
        )
        .bind(device_row_id)
        .bind(now)
        .bind(now)
        .bind(MAX_REMINDER_ATTEMPTS)
        .bind(LATENESS_GRACE_MS)
        .bind(now)
        .bind(now)
        .fetch_optional(self.database.pool())
        .await?;
        if has_due.is_none() {
            return Ok(None);
        }

        let mut tx = self.database.pool().begin_with("BEGIN IMMEDIATE").await?;
        let row = sqlx::query_as::<_, (String, i64, String, i64, String)>(
            "SELECT r.id, r.device_row_id, d.device_id, r.due_at_ms, r.message \
             FROM device_reminders r \
             JOIN devices d ON d.id = r.device_row_id \
             WHERE r.device_row_id = ? \
               AND d.enabled = 1 \
               AND r.status = 'pending' \
               AND r.due_at_ms <= ? \
               AND (r.next_attempt_at_ms IS NULL OR r.next_attempt_at_ms <= ?) \
               AND r.attempts_used < ? \
               AND r.due_at_ms + ? > ? \
               AND NOT EXISTS ( \
                   SELECT 1 FROM reminder_delivery_attempts a \
                   WHERE a.reminder_id = r.id \
                     AND a.status = 'claimed' \
                     AND a.lease_until_ms > ? \
               ) \
             ORDER BY r.due_at_ms, r.id \
             LIMIT 1",
        )
        .bind(device_row_id)
        .bind(now)
        .bind(now)
        .bind(MAX_REMINDER_ATTEMPTS)
        .bind(LATENESS_GRACE_MS)
        .bind(now)
        .bind(now)
        .fetch_optional(&mut *tx)
        .await?;

        let Some((reminder_id, device_row_id, device_id, due_at_ms, message)) = row else {
            tx.commit().await?;
            return Ok(None);
        };

        let attempt_id = uuid::Uuid::new_v4().to_string();
        let lease_until_ms = now.saturating_add(CLAIM_PRE_START_LEASE_MS);
        sqlx::query(
            "INSERT INTO reminder_delivery_attempts(\
                attempt_id, reminder_id, session_instance_id, status, lease_until_ms, started_at_ms, created_at_ms, updated_at_ms\
             ) VALUES (?, ?, ?, 'claimed', ?, NULL, ?, ?)",
        )
        .bind(&attempt_id)
        .bind(&reminder_id)
        .bind(session_id)
        .bind(lease_until_ms)
        .bind(now)
        .bind(now)
        .execute(&mut *tx)
        .await?;

        tx.commit().await?;
        Ok(Some((
            DueReminder {
                reminder_id,
                device_row_id,
                device_id,
                due_at_ms,
                message,
            },
            attempt_id,
        )))
    }

    /// Transitions an unstarted `'claimed'` attempt to started playback after revalidating that
    /// the Device is still enabled and the reminder is still `'pending'`. Increments
    /// `attempts_used` by 1 and extends `lease_until_ms` to the audio-duration ACK deadline.
    pub async fn mark_attempt_started(
        &self,
        reminder_id: &str,
        attempt_id: &str,
        session_id: &str,
        estimated_audio_ms: i64,
    ) -> Result<bool, NotificationError> {
        let now = (self.clock)();
        let mut tx = self.database.pool().begin_with("BEGIN IMMEDIATE").await?;
        let valid = sqlx::query_scalar::<_, i64>(
            "SELECT 1 \
             FROM reminder_delivery_attempts a \
             JOIN device_reminders r ON r.id = a.reminder_id \
             JOIN devices d ON d.id = r.device_row_id \
             WHERE a.attempt_id = ? \
               AND a.reminder_id = ? \
               AND a.session_instance_id = ? \
               AND a.status = 'claimed' \
               AND r.status = 'pending' \
               AND d.enabled = 1 \
               AND r.attempts_used < ?",
        )
        .bind(attempt_id)
        .bind(reminder_id)
        .bind(session_id)
        .bind(MAX_REMINDER_ATTEMPTS)
        .fetch_optional(&mut *tx)
        .await?
        .is_some();

        if !valid {
            sqlx::query(
                "DELETE FROM reminder_delivery_attempts \
                 WHERE attempt_id = ? AND session_instance_id = ? AND status = 'claimed' AND started_at_ms IS NULL",
            )
            .bind(attempt_id)
            .bind(session_id)
            .execute(&mut *tx)
            .await?;
            tx.commit().await?;
            return Ok(false);
        }

        let lease_until_ms = now.saturating_add(compute_ack_timeout_ms(estimated_audio_ms));
        let updated = sqlx::query(
            "UPDATE reminder_delivery_attempts \
             SET started_at_ms = ?, lease_until_ms = ?, updated_at_ms = ? \
             WHERE attempt_id = ? AND session_instance_id = ? AND status = 'claimed' AND started_at_ms IS NULL",
        )
        .bind(now)
        .bind(lease_until_ms)
        .bind(now)
        .bind(attempt_id)
        .bind(session_id)
        .execute(&mut *tx)
        .await?
        .rows_affected();

        if updated == 1 {
            sqlx::query(
                "UPDATE device_reminders \
                 SET attempts_used = attempts_used + 1, updated_at_ms = ? \
                 WHERE id = ? AND status = 'pending'",
            )
            .bind(now)
            .bind(reminder_id)
            .execute(&mut *tx)
            .await?;
        }

        tx.commit().await?;
        Ok(true)
    }

    /// Compatibility claim helper for direct callers/tests.
    pub async fn claim_due(
        &self,
        reminder_id: &str,
        session_id: &str,
    ) -> Result<Option<(DueReminder, String)>, NotificationError> {
        let now = (self.clock)();
        let mut tx = self.database.pool().begin_with("BEGIN IMMEDIATE").await?;
        let row = sqlx::query_as::<_, (String, i64, String, i64, String)>(
            "SELECT r.id, r.device_row_id, d.device_id, r.due_at_ms, r.message \
             FROM device_reminders r \
             JOIN devices d ON d.id = r.device_row_id \
             WHERE r.id = ? \
               AND d.enabled = 1 \
               AND r.status = 'pending' \
               AND r.due_at_ms <= ? \
               AND (r.next_attempt_at_ms IS NULL OR r.next_attempt_at_ms <= ?) \
               AND r.attempts_used < ? \
               AND NOT EXISTS ( \
                   SELECT 1 FROM reminder_delivery_attempts a \
                   WHERE a.reminder_id = r.id \
                     AND a.status = 'claimed' \
                     AND a.lease_until_ms > ? \
               )",
        )
        .bind(reminder_id)
        .bind(now)
        .bind(now)
        .bind(MAX_REMINDER_ATTEMPTS)
        .bind(now)
        .fetch_optional(&mut *tx)
        .await?;
        let Some((reminder_id, device_row_id, device_id, due_at_ms, message)) = row else {
            tx.commit().await?;
            return Ok(None);
        };
        let attempt_id = uuid::Uuid::new_v4().to_string();
        let changed = sqlx::query(
            "UPDATE device_reminders SET attempts_used=attempts_used+1,updated_at_ms=? WHERE id=? AND status='pending'",
        )
        .bind(now)
        .bind(&reminder_id)
        .execute(&mut *tx)
        .await?
        .rows_affected();
        if changed != 1 {
            tx.commit().await?;
            return Ok(None);
        }
        sqlx::query(
            "INSERT INTO reminder_delivery_attempts(attempt_id,reminder_id,session_instance_id,status,lease_until_ms,started_at_ms,created_at_ms,updated_at_ms) VALUES (?,?,?,'claimed',?,?,?,?)",
        )
        .bind(&attempt_id)
        .bind(&reminder_id)
        .bind(session_id)
        .bind(now.saturating_add(MAX_ACK_TIMEOUT_MS))
        .bind(now)
        .bind(now)
        .bind(now)
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(Some((
            DueReminder {
                reminder_id,
                device_row_id,
                device_id,
                due_at_ms,
                message,
            },
            attempt_id,
        )))
    }

    pub async fn due_ids(&self) -> Result<Vec<String>, NotificationError> {
        let now = (self.clock)();
        Ok(sqlx::query_scalar(
            "SELECT r.id \
             FROM device_reminders r \
             JOIN devices d ON d.id = r.device_row_id \
             WHERE r.status = 'pending' \
               AND d.enabled = 1 \
               AND r.due_at_ms <= ? \
               AND (r.next_attempt_at_ms IS NULL OR r.next_attempt_at_ms <= ?) \
               AND r.attempts_used < ? \
               AND NOT EXISTS ( \
                   SELECT 1 FROM reminder_delivery_attempts a \
                   WHERE a.reminder_id = r.id \
                     AND a.status = 'claimed' \
                     AND a.lease_until_ms > ? \
               ) \
             ORDER BY r.due_at_ms, r.id \
             LIMIT 64",
        )
        .bind(now)
        .bind(now)
        .bind(MAX_REMINDER_ATTEMPTS)
        .bind(now)
        .fetch_all(self.database.pool())
        .await?)
    }

    pub async fn due_device(&self, reminder_id: &str) -> Result<Option<i64>, NotificationError> {
        Ok(sqlx::query_scalar(
            "SELECT r.device_row_id \
             FROM device_reminders r \
             JOIN devices d ON d.id = r.device_row_id \
             WHERE r.id = ? AND r.status = 'pending' AND d.enabled = 1",
        )
        .bind(reminder_id)
        .fetch_optional(self.database.pool())
        .await?)
    }

    /// Returns reminders whose `due_at_ms` has arrived (regardless of whether the target Device
    /// is online or offline) and have not yet been recorded in `notifications_inbox`.
    pub async fn due_for_notification_hub(&self) -> Result<Vec<DueReminder>, NotificationError> {
        let now = (self.clock)();
        let rows: Vec<(String, i64, String, i64, String)> = sqlx::query_as(
            "SELECT r.id, r.device_row_id, d.device_id, r.due_at_ms, r.message \
             FROM device_reminders r \
             JOIN devices d ON d.id = r.device_row_id \
             WHERE r.status IN ('pending', 'expired') \
               AND d.enabled = 1 \
               AND r.due_at_ms <= ? \
               AND r.due_at_ms + ? >= ? \
               AND NOT EXISTS ( \
                   SELECT 1 FROM notifications_inbox n \
                   WHERE n.dedup_key = 'device_reminder_due:' || r.id \
               ) \
             ORDER BY r.due_at_ms, r.id \
             LIMIT 64",
        )
        .bind(now)
        .bind(LATENESS_GRACE_MS)
        .bind(now)
        .fetch_all(self.database.pool())
        .await?;

        Ok(rows
            .into_iter()
            .map(
                |(reminder_id, device_row_id, device_id, due_at_ms, message)| DueReminder {
                    reminder_id,
                    device_row_id,
                    device_id,
                    due_at_ms,
                    message,
                },
            )
            .collect())
    }

    pub async fn complete_attempt(
        &self,
        device_row_id: i64,
        session_id: &str,
        reminder_id: &str,
        attempt_id: &str,
    ) -> Result<Option<i64>, NotificationError> {
        Ok(self
            .complete_attempt_with_outcome(device_row_id, session_id, reminder_id, attempt_id)
            .await?
            .map(|o| o.schedule_revision))
    }

    pub async fn complete_attempt_with_outcome(
        &self,
        device_row_id: i64,
        session_id: &str,
        reminder_id: &str,
        attempt_id: &str,
    ) -> Result<Option<CompleteAttemptOutcome>, NotificationError> {
        let now = (self.clock)();
        let mut tx = self.database.pool().begin_with("BEGIN IMMEDIATE").await?;
        // Idempotent duplicate ACK check: if this exact attempt was already completed, return the current schedule_revision.
        if let Some(existing_status) = sqlx::query_scalar::<_, String>(
            "SELECT a.status \
             FROM reminder_delivery_attempts a \
             JOIN device_reminders r ON r.id = a.reminder_id \
             WHERE a.attempt_id = ? \
               AND a.reminder_id = ? \
               AND a.session_instance_id = ? \
               AND r.device_row_id = ?",
        )
        .bind(attempt_id)
        .bind(reminder_id)
        .bind(session_id)
        .bind(device_row_id)
        .fetch_optional(&mut *tx)
        .await?
            && existing_status == "played"
        {
            let revision = sqlx::query_scalar::<_, i64>(
                "SELECT schedule_revision FROM device_reminder_sync WHERE device_row_id = ?",
            )
            .bind(device_row_id)
            .fetch_optional(&mut *tx)
            .await?
            .unwrap_or(1);
            tx.commit().await?;
            return Ok(Some(CompleteAttemptOutcome {
                schedule_revision: revision,
                duplicate_ack: true,
            }));
        }

        let claim_state = sqlx::query_as::<_, (Option<i64>, i64)>(
            "SELECT a.started_at_ms, a.lease_until_ms \
             FROM reminder_delivery_attempts a \
             JOIN device_reminders r ON r.id = a.reminder_id \
             WHERE a.attempt_id = ? \
               AND a.reminder_id = ? \
               AND a.session_instance_id = ? \
               AND a.status = 'claimed' \
               AND r.device_row_id = ? \
               AND r.status = 'pending'",
        )
        .bind(attempt_id)
        .bind(reminder_id)
        .bind(session_id)
        .bind(device_row_id)
        .fetch_optional(&mut *tx)
        .await?;

        let Some((started_at_ms, lease_until_ms)) = claim_state else {
            tx.commit().await?;
            return Ok(None);
        };
        if lease_until_ms <= now {
            tx.commit().await?;
            return Ok(None);
        }

        let was_unstarted = started_at_ms.is_none();
        sqlx::query(
            "UPDATE reminder_delivery_attempts \
             SET status = 'played', started_at_ms = COALESCE(started_at_ms, ?), error_code = NULL, updated_at_ms = ? \
             WHERE attempt_id = ?",
        )
        .bind(now)
        .bind(now)
        .bind(attempt_id)
        .execute(&mut *tx)
        .await?;

        if was_unstarted {
            sqlx::query(
                "UPDATE device_reminders \
                 SET status = 'completed', attempts_used = attempts_used + 1, next_attempt_at_ms = NULL, completed_at_ms = ?, updated_at_ms = ? \
                 WHERE id = ?",
            )
            .bind(now)
            .bind(now)
            .bind(reminder_id)
            .execute(&mut *tx)
            .await?;
        } else {
            sqlx::query(
                "UPDATE device_reminders \
                 SET status = 'completed', next_attempt_at_ms = NULL, completed_at_ms = ?, updated_at_ms = ? \
                 WHERE id = ?",
            )
            .bind(now)
            .bind(now)
            .bind(reminder_id)
            .execute(&mut *tx)
            .await?;
        }

        let revision = bump_schedule_revision(&mut tx, device_row_id).await?;
        tx.commit().await?;
        Ok(Some(CompleteAttemptOutcome {
            schedule_revision: revision,
            duplicate_ack: false,
        }))
    }

    /// Records a device-reported or actor-reported playback failure for a claimed attempt.
    /// Applies bounded exponential backoff or transitions the reminder to `'expired'` when
    /// `attempts_used >= MAX_REMINDER_ATTEMPTS` or the lateness grace window has elapsed.
    pub async fn fail_attempt(
        &self,
        device_row_id: i64,
        session_id: &str,
        reminder_id: &str,
        attempt_id: &str,
        reason: &str,
    ) -> Result<Option<FailAttemptOutcome>, NotificationError> {
        let now = (self.clock)();
        let mut tx = self.database.pool().begin_with("BEGIN IMMEDIATE").await?;
        let row = sqlx::query_as::<_, (Option<i64>, i64, i64)>(
            "SELECT a.started_at_ms, r.attempts_used, r.due_at_ms \
             FROM reminder_delivery_attempts a \
             JOIN device_reminders r ON r.id = a.reminder_id \
             WHERE a.attempt_id = ? \
               AND a.reminder_id = ? \
               AND a.session_instance_id = ? \
               AND a.status = 'claimed' \
               AND r.device_row_id = ? \
               AND r.status = 'pending'",
        )
        .bind(attempt_id)
        .bind(reminder_id)
        .bind(session_id)
        .bind(device_row_id)
        .fetch_optional(&mut *tx)
        .await?;

        let Some((started_at_ms, mut attempts_used, due_at_ms)) = row else {
            tx.commit().await?;
            return Ok(None);
        };

        if started_at_ms.is_none() {
            attempts_used = attempts_used.saturating_add(1);
            sqlx::query(
                "UPDATE device_reminders SET attempts_used = ?, updated_at_ms = ? WHERE id = ?",
            )
            .bind(attempts_used)
            .bind(now)
            .bind(reminder_id)
            .execute(&mut *tx)
            .await?;
        }

        sqlx::query(
            "UPDATE reminder_delivery_attempts \
             SET status = 'failed', error_code = ?, updated_at_ms = ? \
             WHERE attempt_id = ?",
        )
        .bind(reason)
        .bind(now)
        .bind(attempt_id)
        .execute(&mut *tx)
        .await?;

        let expired =
            expire_or_schedule_retry(&mut tx, reminder_id, attempts_used, due_at_ms, now).await?;
        let outcome = if expired {
            let revision = bump_schedule_revision(&mut tx, device_row_id).await?;
            FailAttemptOutcome {
                expired: true,
                schedule_revision: revision,
            }
        } else {
            let revision = sqlx::query_scalar::<_, i64>(
                "SELECT schedule_revision FROM device_reminder_sync WHERE device_row_id=?",
            )
            .bind(device_row_id)
            .fetch_optional(&mut *tx)
            .await?
            .unwrap_or(0);
            FailAttemptOutcome {
                expired: false,
                schedule_revision: revision,
            }
        };

        tx.commit().await?;
        Ok(Some(outcome))
    }

    /// An actor admission failure happens before the first playable packet, so it is not an
    /// attempt. The session fence prevents this cleanup from touching a replacement claim.
    pub async fn release_unstarted_attempt(
        &self,
        reminder_id: &str,
        attempt_id: &str,
        session_id: &str,
    ) -> Result<(), NotificationError> {
        let now = (self.clock)();
        let mut tx = self.database.pool().begin_with("BEGIN IMMEDIATE").await?;
        let existing = sqlx::query_scalar::<_, Option<i64>>(
            "SELECT started_at_ms FROM reminder_delivery_attempts \
             WHERE attempt_id=? AND reminder_id=? AND session_instance_id=? AND status='claimed'",
        )
        .bind(attempt_id)
        .bind(reminder_id)
        .bind(session_id)
        .fetch_optional(&mut *tx)
        .await?;

        if let Some(started_at_ms) = existing {
            sqlx::query(
                "DELETE FROM reminder_delivery_attempts \
                 WHERE attempt_id=? AND reminder_id=? AND session_instance_id=? AND status='claimed'",
            )
            .bind(attempt_id)
            .bind(reminder_id)
            .bind(session_id)
            .execute(&mut *tx)
            .await?;
            if started_at_ms.is_some() {
                sqlx::query(
                    "UPDATE device_reminders \
                     SET attempts_used=MAX(attempts_used-1,0), updated_at_ms=? \
                     WHERE id=? AND status='pending'",
                )
                .bind(now)
                .bind(reminder_id)
                .execute(&mut *tx)
                .await?;
            }
        }
        tx.commit().await?;
        Ok(())
    }

    /// Immediately cleans up any in-flight attempts for a WebSocket session that just closed:
    /// - Unstarted claims (`started_at_ms IS NULL`) are deleted without consuming retry quota.
    /// - Started claims (`started_at_ms IS NOT NULL`) become `'uncertain'` with backoff or expire if exhausted.
    /// Returns `(device_row_id, schedule_revision)` if any reminder transitioned to `'expired'`.
    pub async fn on_session_closed(
        &self,
        device_row_id: i64,
        session_id: &str,
    ) -> Result<Option<i64>, NotificationError> {
        let now = (self.clock)();
        let mut tx = self.database.pool().begin_with("BEGIN IMMEDIATE").await?;
        // 1. Delete unstarted claims for this session without touching attempts_used.
        sqlx::query(
            "DELETE FROM reminder_delivery_attempts \
             WHERE session_instance_id = ? \
               AND status = 'claimed' \
               AND started_at_ms IS NULL",
        )
        .bind(session_id)
        .execute(&mut *tx)
        .await?;

        // 2. Find started claims for this session.
        let started: Vec<(String, String, i64, i64)> = sqlx::query_as(
            "SELECT a.attempt_id, r.id, r.attempts_used, r.due_at_ms \
             FROM reminder_delivery_attempts a \
             JOIN device_reminders r ON r.id = a.reminder_id \
             WHERE a.session_instance_id = ? \
               AND a.status = 'claimed' \
               AND a.started_at_ms IS NOT NULL \
               AND r.device_row_id = ? \
               AND r.status = 'pending'",
        )
        .bind(session_id)
        .bind(device_row_id)
        .fetch_all(&mut *tx)
        .await?;

        let mut any_expired = false;
        for (attempt_id, reminder_id, attempts_used, due_at_ms) in started {
            sqlx::query(
                "UPDATE reminder_delivery_attempts \
                 SET status = 'uncertain', error_code = 'ws_disconnected', updated_at_ms = ? \
                 WHERE attempt_id = ?",
            )
            .bind(now)
            .bind(&attempt_id)
            .execute(&mut *tx)
            .await?;

            if expire_or_schedule_retry(&mut tx, &reminder_id, attempts_used, due_at_ms, now)
                .await?
            {
                any_expired = true;
            }
        }

        let new_revision = if any_expired {
            Some(bump_schedule_revision(&mut tx, device_row_id).await?)
        } else {
            None
        };

        tx.commit().await?;
        Ok(new_revision)
    }

    /// Reaps expired claim leases and overdue pending reminders.
    /// Returns a list of `(device_row_id, new_schedule_revision)` for devices whose schedule
    /// revision changed due to reminder expiration.
    pub async fn reap_expired_leases_and_overdue(
        &self,
    ) -> Result<Vec<(i64, i64)>, NotificationError> {
        let now = (self.clock)();
        let has_work: Option<i64> = sqlx::query_scalar(
            "SELECT 1 WHERE EXISTS ( \
                 SELECT 1 FROM reminder_delivery_attempts \
                 WHERE status = 'claimed' AND lease_until_ms <= ? \
             ) OR EXISTS ( \
                 SELECT 1 FROM device_reminders \
                 WHERE status = 'pending' AND due_at_ms + ? <= ? \
             )",
        )
        .bind(now)
        .bind(LATENESS_GRACE_MS)
        .bind(now)
        .fetch_optional(self.database.pool())
        .await?;
        if has_work.is_none() {
            return Ok(Vec::new());
        }

        let mut tx = self.database.pool().begin_with("BEGIN IMMEDIATE").await?;

        // 1. Delete unstarted claims whose pre-start lease expired without deducting quota.
        sqlx::query(
            "DELETE FROM reminder_delivery_attempts \
             WHERE status = 'claimed' \
               AND started_at_ms IS NULL \
               AND lease_until_ms <= ?",
        )
        .bind(now)
        .execute(&mut *tx)
        .await?;

        // 2. Find started claims whose ACK lease expired.
        let expired_started: Vec<(String, String, i64, i64, i64)> = sqlx::query_as(
            "SELECT a.attempt_id, r.id, r.device_row_id, r.attempts_used, r.due_at_ms \
             FROM reminder_delivery_attempts a \
             JOIN device_reminders r ON r.id = a.reminder_id \
             WHERE a.status = 'claimed' \
               AND a.started_at_ms IS NOT NULL \
               AND a.lease_until_ms <= ?",
        )
        .bind(now)
        .fetch_all(&mut *tx)
        .await?;

        let mut affected_devices = std::collections::BTreeSet::new();
        for (attempt_id, reminder_id, device_row_id, attempts_used, due_at_ms) in expired_started {
            sqlx::query(
                "UPDATE reminder_delivery_attempts \
                 SET status = 'uncertain', error_code = 'ack_timeout', updated_at_ms = ? \
                 WHERE attempt_id = ?",
            )
            .bind(now)
            .bind(&attempt_id)
            .execute(&mut *tx)
            .await?;

            if expire_or_schedule_retry(&mut tx, &reminder_id, attempts_used, due_at_ms, now)
                .await?
            {
                affected_devices.insert(device_row_id);
            }
        }

        // 3. Expire pending reminders past the 24h lateness grace window that have no active lease.
        let overdue_rows: Vec<(String, i64)> = sqlx::query_as(
            "SELECT r.id, r.device_row_id \
             FROM device_reminders r \
             WHERE r.status = 'pending' \
               AND r.due_at_ms + ? <= ? \
               AND NOT EXISTS ( \
                   SELECT 1 FROM reminder_delivery_attempts a \
                   WHERE a.reminder_id = r.id \
                     AND a.status = 'claimed' \
                     AND a.lease_until_ms > ? \
               )",
        )
        .bind(LATENESS_GRACE_MS)
        .bind(now)
        .bind(now)
        .fetch_all(&mut *tx)
        .await?;

        for (reminder_id, device_row_id) in overdue_rows {
            let changed = sqlx::query(
                "UPDATE device_reminders \
                 SET status = 'expired', next_attempt_at_ms = NULL, updated_at_ms = ? \
                 WHERE id = ? AND status = 'pending'",
            )
            .bind(now)
            .bind(&reminder_id)
            .execute(&mut *tx)
            .await?
            .rows_affected();
            if changed == 1 {
                affected_devices.insert(device_row_id);
            }
        }

        let mut updated_revisions = Vec::with_capacity(affected_devices.len());
        for device_row_id in affected_devices {
            let revision = bump_schedule_revision(&mut tx, device_row_id).await?;
            updated_revisions.push((device_row_id, revision));
        }

        tx.commit().await?;
        Ok(updated_revisions)
    }

    /// Computes the earliest epoch timestamp (in milliseconds) at which the scheduler must wake up
    /// to either reap an expired lease, expire an overdue reminder, or deliver a due reminder to
    /// one of the currently online reminder-capable devices.
    pub async fn next_deadline_ms(
        &self,
        online_device_ids: &[i64],
    ) -> Result<Option<i64>, NotificationError> {
        let min_lease: Option<i64> = sqlx::query_scalar(
            "SELECT MIN(lease_until_ms) FROM reminder_delivery_attempts WHERE status = 'claimed'",
        )
        .fetch_one(self.database.pool())
        .await?;

        let min_expiry: Option<i64> = sqlx::query_scalar(
            "SELECT MIN(due_at_ms) + ? FROM device_reminders WHERE status = 'pending'",
        )
        .bind(LATENESS_GRACE_MS)
        .fetch_one(self.database.pool())
        .await?;

        let min_online_due: Option<i64> = if online_device_ids.is_empty() {
            None
        } else {
            let mut qb = sqlx::QueryBuilder::<sqlx::Sqlite>::new(
                "SELECT MIN(MAX(r.due_at_ms, COALESCE(r.next_attempt_at_ms, r.due_at_ms))) \
                 FROM device_reminders r \
                 JOIN devices d ON d.id = r.device_row_id \
                 WHERE r.status = 'pending' \
                   AND d.enabled = 1 \
                   AND r.attempts_used < ",
            );
            qb.push_bind(MAX_REMINDER_ATTEMPTS);
            qb.push(" AND r.device_row_id IN (");
            let mut separated = qb.separated(", ");
            for id in online_device_ids {
                separated.push_bind(*id);
            }
            separated.push_unseparated(")");
            qb.push(
                " AND NOT EXISTS ( \
                    SELECT 1 FROM reminder_delivery_attempts a \
                    WHERE a.reminder_id = r.id \
                      AND a.status = 'claimed' \
                )",
            );
            qb.build_query_scalar()
                .fetch_one(self.database.pool())
                .await?
        };

        let min_unnotified_due: Option<i64> = sqlx::query_scalar(
            "SELECT MIN(r.due_at_ms) \
             FROM device_reminders r \
             JOIN devices d ON d.id = r.device_row_id \
             WHERE r.status = 'pending' \
               AND d.enabled = 1 \
               AND NOT EXISTS ( \
                   SELECT 1 FROM notifications_inbox n \
                   WHERE n.dedup_key = 'device_reminder_due:' || r.id \
               )",
        )
        .fetch_one(self.database.pool())
        .await
        .unwrap_or(None);

        Ok([min_lease, min_expiry, min_online_due, min_unnotified_due]
            .into_iter()
            .flatten()
            .min())
    }
}

pub(super) async fn bump_schedule_revision(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    device_row_id: i64,
) -> Result<i64, sqlx::Error> {
    sqlx::query(
        "INSERT INTO device_reminder_sync(device_row_id,schedule_revision) VALUES (?,1) \
         ON CONFLICT(device_row_id) DO UPDATE SET schedule_revision=schedule_revision+1",
    )
    .bind(device_row_id)
    .execute(&mut **tx)
    .await?;
    sqlx::query_scalar("SELECT schedule_revision FROM device_reminder_sync WHERE device_row_id=?")
        .bind(device_row_id)
        .fetch_one(&mut **tx)
        .await
}

async fn expire_or_schedule_retry(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    reminder_id: &str,
    attempts_used: i64,
    due_at_ms: i64,
    now: i64,
) -> Result<bool, sqlx::Error> {
    if attempts_used >= MAX_REMINDER_ATTEMPTS || now >= due_at_ms.saturating_add(LATENESS_GRACE_MS)
    {
        let changed = sqlx::query(
            "UPDATE device_reminders \
             SET status = 'expired', next_attempt_at_ms = NULL, updated_at_ms = ? \
             WHERE id = ? AND status = 'pending'",
        )
        .bind(now)
        .bind(reminder_id)
        .execute(&mut **tx)
        .await?
        .rows_affected();
        Ok(changed == 1)
    } else {
        let next_at = now.saturating_add(retry_backoff_ms(attempts_used));
        sqlx::query(
            "UPDATE device_reminders \
             SET next_attempt_at_ms = ?, updated_at_ms = ? \
             WHERE id = ? AND status = 'pending'",
        )
        .bind(next_at)
        .bind(now)
        .bind(reminder_id)
        .execute(&mut **tx)
        .await?;
        Ok(false)
    }
}
