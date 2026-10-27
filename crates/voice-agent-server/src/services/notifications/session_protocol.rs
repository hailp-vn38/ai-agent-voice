//! The Device side of reminder delivery within one Voice Session: sync snapshots, Device Cache
//! ACKs and Playback ACKs arrive as protocol events; replies leave as reminder control messages.
use super::ReminderCoordinator;
use crate::protocol::ReminderEvent;

impl ReminderCoordinator {
    /// Applies one reminder protocol event from the Device of `session_id` and returns the
    /// reminder control message to send back to it, if the event has one. Persistence or fencing
    /// failures produce no reply: the Device re-syncs on its own schedule.
    pub async fn on_device_reminder_event(
        &self,
        device: i64,
        session_id: &str,
        event: ReminderEvent,
    ) -> Option<String> {
        match event {
            ReminderEvent::SyncRequest { known_revision } => {
                let snapshot = self.repo().snapshot(device).await.ok()?;
                let snapshot_id = uuid::Uuid::new_v4().to_string();
                let ids = snapshot.items.iter().map(|item| item.id.clone()).collect();
                if !self.sessions.remember_snapshot(
                    device,
                    session_id,
                    snapshot_id.clone(),
                    snapshot.revision,
                    ids,
                ) {
                    return None;
                }
                tracing::info!(
                    subsystem = "notification",
                    event = "reminder_snapshot_sent",
                    device_row_id = device,
                    session_id = %session_id,
                    snapshot_id = %snapshot_id,
                    known_revision = ?known_revision,
                    revision = snapshot.revision,
                    items_count = snapshot.items.len(),
                    has_more = snapshot.has_more,
                    "reminder snapshot sent to device"
                );
                Some(
                    serde_json::json!({
                        "type":"reminder", "event":"sync", "session_id":session_id,
                        "snapshot_id":snapshot_id, "revision":snapshot.revision,
                        "server_time_ms":snapshot.server_time_ms, "items":snapshot.items,
                        "has_more":snapshot.has_more, "next_sync_at_ms":snapshot.server_time_ms + 60_000,
                    })
                    .to_string(),
                )
            }
            ReminderEvent::SyncAck {
                snapshot_id,
                revision,
                next_wake_at_ms,
            } => {
                let snapshot = self.sessions.acknowledge_snapshot(
                    device,
                    session_id,
                    &snapshot_id,
                    revision,
                )?;
                tracing::info!(
                    subsystem = "notification",
                    event = "reminder_snapshot_acked",
                    device_row_id = device,
                    session_id = %session_id,
                    snapshot_id = %snapshot_id,
                    revision = snapshot.revision,
                    acked_items_count = snapshot.item_ids.len(),
                    next_wake_at_ms = ?next_wake_at_ms,
                    "reminder snapshot acknowledged by device"
                );
                let _ = self
                    .repo()
                    .acknowledge_snapshot(
                        device,
                        snapshot.revision,
                        &snapshot.item_ids,
                        next_wake_at_ms,
                    )
                    .await;
                None
            }
            ReminderEvent::Played {
                reminder_id,
                attempt_id,
            } => {
                self.sessions.clear_in_flight_reminder(device, session_id);
                let outcome = self
                    .complete_attempt_with_outcome(device, session_id, &reminder_id, &attempt_id)
                    .await
                    .ok()??;
                Some(delivery_result(
                    session_id,
                    &reminder_id,
                    &attempt_id,
                    "completed",
                    outcome.schedule_revision,
                ))
            }
            ReminderEvent::Failed {
                reminder_id,
                attempt_id,
                reason,
            } => {
                self.sessions.clear_in_flight_reminder(device, session_id);
                let outcome = self
                    .fail_attempt(device, session_id, &reminder_id, &attempt_id, &reason)
                    .await
                    .ok()??;
                Some(delivery_result(
                    session_id,
                    &reminder_id,
                    &attempt_id,
                    "failed",
                    outcome.schedule_revision,
                ))
            }
        }
    }
}

fn delivery_result(
    session_id: &str,
    reminder_id: &str,
    attempt_id: &str,
    outcome: &str,
    revision: impl serde::Serialize,
) -> String {
    serde_json::json!({
        "type":"reminder", "event":"delivery_result", "session_id":session_id,
        "reminder_id":reminder_id, "attempt_id":attempt_id,
        "outcome":outcome, "revision":revision,
    })
    .to_string()
}
