use crate::{
    database::notifications::DeviceNotifications,
    services::{
        device::active_sessions::ActiveDeviceSessions,
        device_push::DevicePush,
        device_push_wake::{WakePublisher, WakeSignal},
        notification_hub::{NotificationHub, bridge},
        notifications::ReminderCoordinator,
    },
    session::{DirectSpeechRequest, SpeechOrigin},
};
use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
    time::Duration,
};
use tokio_util::sync::CancellationToken;

const IDLE_SWEEP_INTERVAL: Duration = Duration::from_secs(60);
const BUSY_RETRY_INTERVAL: Duration = Duration::from_millis(200);
const MIN_ZERO_DEADLINE_BACKOFF: Duration = Duration::from_millis(25);

/// Event-driven reminder scheduler. Wakes immediately on schedule mutations, device session
/// registration, turn-boundary completion, or the exact next deadline (`due_at_ms`,
/// `next_attempt_at_ms`, `lease_until_ms`, or 24-hour expiration).
pub fn start(
    notifications: DeviceNotifications,
    sessions: Arc<ActiveDeviceSessions>,
    stopping: CancellationToken,
) {
    start_with_hub(notifications, sessions, None, stopping);
}

/// Like [`start_with_hub`], plus a Wake Signal publisher used when the Device Push Channel is
/// enabled. `None` keeps behaviour identical to [`start_with_hub`].
pub fn start_with_wake(
    notifications: DeviceNotifications,
    sessions: Arc<ActiveDeviceSessions>,
    notification_hub: Option<Arc<NotificationHub>>,
    wake: Option<Arc<dyn WakePublisher>>,
    stopping: CancellationToken,
) {
    spawn_loop(notifications, sessions, notification_hub, wake, stopping);
}

pub fn start_with_hub(
    notifications: DeviceNotifications,
    sessions: Arc<ActiveDeviceSessions>,
    notification_hub: Option<Arc<NotificationHub>>,
    stopping: CancellationToken,
) {
    spawn_loop(notifications, sessions, notification_hub, None, stopping);
}

fn spawn_loop(
    notifications: DeviceNotifications,
    sessions: Arc<ActiveDeviceSessions>,
    notification_hub: Option<Arc<NotificationHub>>,
    wake: Option<Arc<dyn WakePublisher>>,
    stopping: CancellationToken,
) {
    let coordinator = ReminderCoordinator::new(notifications, Arc::clone(&sessions));
    let wakeup = Arc::clone(coordinator.scheduler_wakeup());
    tokio::spawn(async move {
        let mut notified_reminder_ids = HashSet::<String>::new();
        // reminder id -> trigger moment already signalled (one Wake Signal per due/retry moment).
        let mut signalled = HashMap::<String, i64>::new();
        loop {
            if stopping.is_cancelled() {
                return;
            }
            let notified = wakeup.notified();
            tokio::pin!(notified);

            let _ = coordinator.reap_expired_leases_and_overdue().await;

            if let Some(ref hub) = notification_hub
                && let Ok(due_for_hub) = coordinator.repo().due_for_notification_hub().await
            {
                for due in due_for_hub {
                    if notified_reminder_ids.insert(due.reminder_id.clone()) {
                        let input = bridge::from_due_device_reminder(&due);
                        let _ = hub.publish(input).await;
                    }
                }
            }

            let mut any_transient_rejection = false;
            let ready_devices = sessions.ready_reminder_devices();
            for (device_row_id, session_id) in ready_devices {
                let Ok(Some((due, attempt_id))) = coordinator
                    .repo()
                    .claim_next_due_for_device(device_row_id, &session_id)
                    .await
                else {
                    continue;
                };
                if let Some(ref hub) = notification_hub
                    && notified_reminder_ids.insert(due.reminder_id.clone())
                {
                    let input = bridge::from_due_device_reminder(&due);
                    let _ = hub.publish(input).await;
                }
                if !sessions.set_in_flight_reminder(device_row_id, &session_id, attempt_id.clone())
                {
                    let _ = coordinator
                        .repo()
                        .release_unstarted_attempt(&due.reminder_id, &attempt_id, &session_id)
                        .await;
                    continue;
                }
                let estimated_audio_ms = ((due.message.chars().count() as i64) * 80).max(2_000);
                let reminder_id = due.reminder_id;
                let device_id = due.device_id;
                let due_at_ms = due.due_at_ms;
                tracing::info!(
                    reminder_id = %reminder_id,
                    attempt_id = %attempt_id,
                    device_id = %device_id,
                    device_row_id,
                    session_id = %session_id,
                    due_at_ms,
                    message = %due.message,
                    "reminder triggered and dispatching direct speech to device"
                );
                let request = DirectSpeechRequest {
                    request_id: attempt_id.clone(),
                    text: due.message,
                    origin: SpeechOrigin::Reminder {
                        reminder_id: reminder_id.clone(),
                        attempt_id: attempt_id.clone(),
                    },
                };
                match sessions.speak(device_row_id, request).await {
                    Ok(()) => {
                        match coordinator
                            .mark_attempt_started(
                                &reminder_id,
                                &attempt_id,
                                &session_id,
                                estimated_audio_ms,
                            )
                            .await
                        {
                            Ok(true) => {
                                tracing::info!(
                                    reminder_id = %reminder_id,
                                    attempt_id = %attempt_id,
                                    device_id = %device_id,
                                    device_row_id,
                                    session_id = %session_id,
                                    estimated_audio_ms,
                                    "reminder playback accepted by session actor; awaiting playback ack"
                                );
                            }
                            _ => {
                                tracing::warn!(
                                    reminder_id = %reminder_id,
                                    attempt_id = %attempt_id,
                                    device_id = %device_id,
                                    device_row_id,
                                    session_id = %session_id,
                                    "reminder playback cancelled during pre-start revalidation"
                                );
                                sessions.clear_in_flight_reminder(device_row_id, &session_id);
                            }
                        }
                    }
                    Err(err) => {
                        tracing::warn!(
                            reminder_id = %reminder_id,
                            attempt_id = %attempt_id,
                            device_id = %device_id,
                            device_row_id,
                            session_id = %session_id,
                            error = ?err,
                            "reminder dispatch deferred before playback start; releasing unstarted claim"
                        );
                        any_transient_rejection = true;
                        sessions.clear_in_flight_reminder(device_row_id, &session_id);
                        let _ = coordinator
                            .repo()
                            .release_unstarted_attempt(&reminder_id, &attempt_id, &session_id)
                            .await;
                    }
                }
            }

            let mut wake_wait: Option<Duration> = None;
            if let Some(ref publisher) = wake {
                wake_wait = signal_unconnected_devices(
                    &coordinator,
                    &sessions,
                    publisher.as_ref(),
                    &mut signalled,
                )
                .await;
            }

            let sleep_duration = if any_transient_rejection {
                BUSY_RETRY_INTERVAL
            } else {
                let online_ids = sessions.active_reminder_device_ids();
                match coordinator.repo().next_deadline_ms(&online_ids).await {
                    Ok(Some(deadline_ms)) => {
                        let now_ms = coordinator.repo().clock_now();
                        let delta_ms = deadline_ms.saturating_sub(now_ms);
                        if delta_ms <= 0 {
                            MIN_ZERO_DEADLINE_BACKOFF
                        } else {
                            Duration::from_millis(delta_ms as u64).min(IDLE_SWEEP_INTERVAL)
                        }
                    }
                    _ => IDLE_SWEEP_INTERVAL,
                }
            };

            let sleep_duration = match wake_wait {
                Some(wait) => sleep_duration.min(wait.max(MIN_ZERO_DEADLINE_BACKOFF)),
                None => sleep_duration,
            };

            tokio::select! {
                _ = stopping.cancelled() => return,
                _ = &mut notified => {}
                _ = tokio::time::sleep(sleep_duration) => {}
            }
        }
    });
}

const PUBLISH_RETRY_INTERVAL: Duration = Duration::from_secs(5);

/// Publishes one Wake Signal per due moment (or retry checkpoint) for Devices without a session
/// able to take the Reminder. Returns how long the scheduler may sleep before the next
/// signal-relevant moment (a future trigger, or a retry after a failed publish).
async fn signal_unconnected_devices(
    coordinator: &ReminderCoordinator,
    sessions: &ActiveDeviceSessions,
    publisher: &dyn WakePublisher,
    signalled: &mut HashMap<String, i64>,
) -> Option<Duration> {
    let candidates = match coordinator.repo().wake_candidates().await {
        Ok(candidates) => candidates,
        Err(err) => {
            tracing::warn!(error = ?err, "wake signal candidate query failed");
            return Some(PUBLISH_RETRY_INTERVAL);
        }
    };
    let now_ms = coordinator.repo().clock_now();
    let connected: HashSet<i64> = sessions.active_reminder_device_ids().into_iter().collect();
    signalled.retain(|id, _| candidates.iter().any(|c| &c.reminder_id == id));

    let mut wait: Option<Duration> = None;
    let mut shorten = |candidate: Duration| {
        wait = Some(wait.map_or(candidate, |current| current.min(candidate)));
    };
    for candidate in candidates {
        if connected.contains(&candidate.device_row_id) {
            continue;
        }
        if candidate.trigger_at_ms > now_ms {
            shorten(Duration::from_millis(
                (candidate.trigger_at_ms - now_ms) as u64,
            ));
            continue;
        }
        if signalled.get(&candidate.reminder_id) == Some(&candidate.trigger_at_ms) {
            continue;
        }
        if signalled.contains_key(&candidate.reminder_id) {
            tracing::warn!(
                reminder_id = %candidate.reminder_id,
                device_id = %candidate.device_id,
                "wake signal sent but device did not connect; retry mark reached"
            );
        }
        let topic = DevicePush::signal_topic(&candidate.device_id);
        let payload = WakeSignal::new(&candidate.reminder_id, candidate.expires_at_ms).to_payload();
        match publisher.publish(&topic, payload).await {
            Ok(()) => {
                signalled.insert(candidate.reminder_id.clone(), candidate.trigger_at_ms);
                tracing::info!(
                    reminder_id = %candidate.reminder_id,
                    device_id = %candidate.device_id,
                    topic = %topic,
                    "wake signal published; waiting for device to connect"
                );
            }
            Err(err) => {
                tracing::warn!(
                    reminder_id = %candidate.reminder_id,
                    device_id = %candidate.device_id,
                    error = %err,
                    "wake signal publish failed; will retry"
                );
                shorten(PUBLISH_RETRY_INTERVAL);
            }
        }
    }
    wait
}
