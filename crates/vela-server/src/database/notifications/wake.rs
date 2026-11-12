use super::{
    DeviceNotifications, NotificationError,
    delivery::{LATENESS_GRACE_MS, MAX_REMINDER_ATTEMPTS},
};

/// A pending Reminder that could be delivered, with the moment it becomes deliverable.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WakeCandidate {
    pub reminder_id: String,
    pub device_row_id: i64,
    pub device_id: String,
    /// `max(due_at_ms, next_attempt_at_ms)`: identifies one due moment or retry checkpoint.
    pub trigger_at_ms: i64,
    /// Last moment the Reminder can still be delivered (24 hour lateness window).
    pub expires_at_ms: i64,
}

impl DeviceNotifications {
    /// Pending, deliverable Reminders of enabled Devices (due now or in the future), earliest
    /// first. Reminders under a live claim are skipped: a session is already serving them.
    pub async fn wake_candidates(&self) -> Result<Vec<WakeCandidate>, NotificationError> {
        let now = (self.clock)();
        let rows: Vec<(String, i64, String, i64, i64)> = sqlx::query_as(
            "SELECT r.id, r.device_row_id, d.device_id, \
                    MAX(r.due_at_ms, COALESCE(r.next_attempt_at_ms, r.due_at_ms)), \
                    r.due_at_ms + ? \
             FROM device_reminders r \
             JOIN devices d ON d.id = r.device_row_id \
             WHERE r.status = 'pending' \
               AND d.enabled = 1 \
               AND r.attempts_used < ? \
               AND r.due_at_ms + ? > ? \
               AND NOT EXISTS ( \
                   SELECT 1 FROM reminder_delivery_attempts a \
                   WHERE a.reminder_id = r.id \
                     AND a.status = 'claimed' \
                     AND a.lease_until_ms > ? \
               ) \
             ORDER BY 4, r.id \
             LIMIT 256",
        )
        .bind(LATENESS_GRACE_MS)
        .bind(MAX_REMINDER_ATTEMPTS)
        .bind(LATENESS_GRACE_MS)
        .bind(now)
        .bind(now)
        .fetch_all(self.database.pool())
        .await?;
        Ok(rows
            .into_iter()
            .map(
                |(reminder_id, device_row_id, device_id, trigger_at_ms, expires_at_ms)| {
                    WakeCandidate {
                        reminder_id,
                        device_row_id,
                        device_id,
                        trigger_at_ms,
                        expires_at_ms,
                    }
                },
            )
            .collect())
    }
}
