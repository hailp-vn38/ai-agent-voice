use crate::{
    database::notifications::DeviceNotifications,
    services::active_device_sessions::ActiveDeviceSessions,
    session::{DirectSpeechRequest, SpeechOrigin},
};
use std::sync::Arc;
use tokio_util::sync::CancellationToken;

/// One bounded process task scans due rows. It never keeps a database claim for an offline
/// Device: current-session admission happens before the durable attempt is created.
pub fn start(
    notifications: DeviceNotifications,
    sessions: Arc<ActiveDeviceSessions>,
    stopping: CancellationToken,
) {
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(std::time::Duration::from_secs(1));
        loop {
            tokio::select! {
                _ = stopping.cancelled() => return,
                _ = tick.tick() => {}
            }
            let Ok(ids) = notifications.due_ids().await else {
                continue;
            };
            for id in ids {
                // Read the owner/session before incrementing attempts: offline and busy work is
                // deliberately not a delivery attempt.
                let Ok(Some(device)) = notifications.due_device(&id).await else {
                    continue;
                };
                let Some(session) = sessions.current(device).filter(|s| s.features.reminders)
                else {
                    continue;
                };
                let session_id = session.session_id;
                let Ok(Some((due, attempt_id))) = notifications.claim_due(&id, &session_id).await
                else {
                    continue;
                };
                let reminder_id = due.reminder_id;
                let request = DirectSpeechRequest {
                    request_id: attempt_id.clone(),
                    text: due.message,
                    origin: SpeechOrigin::Reminder {
                        reminder_id: reminder_id.clone(),
                        attempt_id: attempt_id.clone(),
                    },
                };
                if sessions.speak(device, request).await.is_err() {
                    let _ = notifications
                        .release_unstarted_attempt(&reminder_id, &attempt_id, &session_id)
                        .await;
                }
            }
        }
    });
}
