use std::{
    sync::{
        Arc,
        atomic::{AtomicI64, Ordering},
    },
    time::Duration,
};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use vela_server::{
    config::DatabaseConfig,
    database::{
        Database,
        notifications::{CreateReminder, DeviceNotifications, MutationContext, ReminderSource},
    },
    protocol::ClientFeatures,
    services::{
        active_device_sessions::ActiveDeviceSessions,
        device_push::DevicePush,
        device_push_wake::RecordingPublisher,
        notifications::{ReminderCoordinator, reminder_scheduler},
    },
    session::{SessionEvent, SpeechOrigin},
};

const T0: i64 = 1_800_000_000_000;

struct Fixture {
    device_row_id: i64,
    clock: Arc<AtomicI64>,
    repo: DeviceNotifications,
    sessions: Arc<ActiveDeviceSessions>,
    coordinator: ReminderCoordinator,
    recorder: Arc<RecordingPublisher>,
    stopping: CancellationToken,
}

async fn fixture(wake_enabled: bool) -> Fixture {
    let db = Database::connect(&DatabaseConfig {
        url: "sqlite::memory:".into(),
        ..Default::default()
    })
    .await
    .unwrap();
    let agent = sqlx::query(
        "INSERT INTO agents (key, name, created_at, updated_at) VALUES ('a', 'A', 1000, 1000)",
    )
    .execute(db.pool())
    .await
    .unwrap()
    .last_insert_rowid();
    let device_row_id = sqlx::query(
        "INSERT INTO devices (device_id, agent_id, kind, enabled, created_at, updated_at) \
         VALUES ('esp32-wake', ?, 'physical', 1, 1000, 1000)",
    )
    .bind(agent)
    .execute(db.pool())
    .await
    .unwrap()
    .last_insert_rowid();
    let clock = Arc::new(AtomicI64::new(T0));
    let c = Arc::clone(&clock);
    let repo = DeviceNotifications::with_clock(db, Arc::new(move || c.load(Ordering::SeqCst)));
    let sessions = Arc::new(ActiveDeviceSessions::default());
    let coordinator = ReminderCoordinator::new(repo.clone(), Arc::clone(&sessions));
    let recorder = Arc::new(RecordingPublisher::default());
    let stopping = CancellationToken::new();
    reminder_scheduler::start_with_wake(
        repo.clone(),
        Arc::clone(&sessions),
        None,
        wake_enabled.then(|| recorder.clone() as _),
        stopping.clone(),
    );
    Fixture {
        device_row_id,
        clock,
        repo,
        sessions,
        coordinator,
        recorder,
        stopping,
    }
}

async fn create(f: &Fixture, req: &str, delay: i64) -> String {
    let ctx = MutationContext {
        caller: "test",
        request_id: req,
        audit_request_id: req,
        source: ReminderSource::Voice,
        agent_scope: None,
    };
    f.coordinator
        .create(
            "esp32-wake",
            &ctx,
            CreateReminder {
                message: "Uống thuốc".into(),
                delay_seconds: Some(delay),
                scheduled_at: None,
                timezone: None,
            },
        )
        .await
        .unwrap()
        .id
}

async fn settle() {
    tokio::time::sleep(Duration::from_millis(120)).await;
}

fn reminder_features() -> ClientFeatures {
    ClientFeatures {
        reminders: true,
        direct_tts: true,
        ..Default::default()
    }
}

#[tokio::test]
async fn due_reminder_without_session_emits_exactly_one_wake_signal() {
    let f = fixture(true).await;
    let id = create(&f, "r1", 1).await;
    f.clock.store(T0 + 2_000, Ordering::SeqCst);
    f.sessions.scheduler_wakeup().notify_one();
    settle().await;
    f.sessions.scheduler_wakeup().notify_one();
    settle().await;

    let published = f.recorder.published();
    assert_eq!(published.len(), 1, "no re-publish on later sweeps");
    assert_eq!(published[0].0, DevicePush::signal_topic("esp32-wake"));
    let payload: serde_json::Value = serde_json::from_slice(&published[0].1).unwrap();
    assert_eq!(payload["v"], 1);
    assert_eq!(payload["type"], "wake");
    assert_eq!(payload["id"], id);
    assert_eq!(payload["expires_at"], T0 + 1_000 + 86_400_000);
    f.stopping.cancel();
}

#[tokio::test]
async fn ready_session_or_disabled_feature_emits_nothing() {
    let f = fixture(true).await;
    let (tx, _rx) = mpsc::channel(8);
    f.sessions.register(
        f.device_row_id,
        "s".into(),
        reminder_features(),
        tx,
        CancellationToken::new(),
    );
    create(&f, "r1", 1).await;
    f.clock.store(T0 + 2_000, Ordering::SeqCst);
    f.sessions.scheduler_wakeup().notify_one();
    settle().await;
    assert!(f.recorder.published().is_empty());
    f.stopping.cancel();

    let off = fixture(false).await;
    create(&off, "r1", 1).await;
    off.clock.store(T0 + 2_000, Ordering::SeqCst);
    off.sessions.scheduler_wakeup().notify_one();
    settle().await;
    assert!(off.recorder.published().is_empty());
    off.stopping.cancel();
}

#[tokio::test]
async fn scheduler_sleeps_until_future_reminder_then_signals() {
    let f = fixture(true).await;
    create(&f, "r1", 1).await;
    // Not yet due: nothing published. Advance clock; the deadline sleep (<=60s idle sweep)
    // is bypassed by a wakeup here, which proves the due check, not the timer.
    settle().await;
    assert!(f.recorder.published().is_empty());
    f.clock.store(T0 + 1_500, Ordering::SeqCst);
    f.sessions.scheduler_wakeup().notify_one();
    settle().await;
    assert_eq!(f.recorder.published().len(), 1);
    f.stopping.cancel();
}

#[tokio::test]
async fn retry_checkpoint_signals_again_and_publish_failure_does_not_stop_scheduler() {
    let f = fixture(true).await;
    f.recorder.set_failing(true);
    let id = create(&f, "r1", 1).await;
    f.clock.store(T0 + 2_000, Ordering::SeqCst);
    f.sessions.scheduler_wakeup().notify_one();
    settle().await;
    assert!(f.recorder.published().is_empty());

    // Failed publish is retried on the next sweep (the Device was never told).
    f.recorder.set_failing(false);
    f.sessions.scheduler_wakeup().notify_one();
    settle().await;
    assert_eq!(f.recorder.published().len(), 1);

    // A failed delivery attempt schedules a retry checkpoint: a new signal at that time.
    sqlx::query("UPDATE device_reminders SET next_attempt_at_ms = ? WHERE id = ?")
        .bind(T0 + 60_000)
        .bind(&id)
        .execute(f.repo.database().pool())
        .await
        .unwrap();
    f.sessions.scheduler_wakeup().notify_one();
    settle().await;
    assert_eq!(f.recorder.published().len(), 1, "retry not yet due");
    f.clock.store(T0 + 61_000, Ordering::SeqCst);
    f.sessions.scheduler_wakeup().notify_one();
    settle().await;
    assert_eq!(f.recorder.published().len(), 2);
    f.stopping.cancel();
}

#[tokio::test]
async fn device_connecting_after_wake_signal_completes_the_reminder() {
    let f = fixture(true).await;
    let id = create(&f, "r1", 1).await;
    f.clock.store(T0 + 2_000, Ordering::SeqCst);
    f.sessions.scheduler_wakeup().notify_one();
    settle().await;
    assert_eq!(f.recorder.published().len(), 1);

    // Simulated Device opens a session announcing reminders.
    let (tx, mut rx) = mpsc::channel(8);
    f.sessions.register(
        f.device_row_id,
        "woken".into(),
        reminder_features(),
        tx,
        CancellationToken::new(),
    );
    let Some(SessionEvent::DirectSpeech { request, reply }) =
        tokio::time::timeout(Duration::from_secs(2), rx.recv())
            .await
            .expect("direct speech after connect")
    else {
        panic!("expected direct speech")
    };
    let _ = reply.send(Ok(()));
    let SpeechOrigin::Reminder {
        reminder_id,
        attempt_id,
    } = request.origin
    else {
        panic!("expected reminder origin")
    };
    assert_eq!(reminder_id, id);
    settle().await;
    f.coordinator
        .complete_attempt_with_outcome(f.device_row_id, "woken", &reminder_id, &attempt_id)
        .await
        .unwrap();
    let list = f.coordinator.list("esp32-wake", None, 1, 10).await.unwrap();
    assert_eq!(list["items"][0]["status"], "completed");
    assert_eq!(f.recorder.published().len(), 1, "no extra signal");
    f.stopping.cancel();
}
