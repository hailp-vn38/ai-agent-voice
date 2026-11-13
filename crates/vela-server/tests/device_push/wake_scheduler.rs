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
        device_push::{DevicePush, RecordingPublisher},
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
    let device_push = wake_enabled.then(|| {
        Arc::new(
            DevicePush::with_publisher(&crate::harness::push_config(), recorder.clone())
                .expect("push enabled"),
        )
    });
    reminder_scheduler::start_with_wake(
        repo.clone(),
        Arc::clone(&sessions),
        None,
        device_push,
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

/// Lets the scheduler finish a sweep. Only for asserting that something did *not* happen;
/// expected effects are awaited with [`published`] or [`eventually`].
async fn settle() {
    tokio::time::sleep(Duration::from_millis(120)).await;
}

async fn eventually(what: &str, condition: impl Fn() -> bool) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    while !condition() {
        assert!(tokio::time::Instant::now() < deadline, "timed out: {what}");
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
}

/// Wakes the scheduler and waits until `count` Wake Signals were published.
async fn published(f: &Fixture, count: usize) -> Vec<(String, Vec<u8>)> {
    f.sessions.scheduler_wakeup().notify_one();
    eventually("wake signal published", || {
        f.recorder.published().len() >= count
    })
    .await;
    f.recorder.published()
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
    published(&f, 1).await;
    f.sessions.scheduler_wakeup().notify_one();
    settle().await;

    let published = f.recorder.published();
    assert_eq!(published.len(), 1, "no re-publish on later sweeps");
    assert_eq!(published[0].0, DevicePush::signal_topic("esp32-wake"));
    let payload: serde_json::Value = serde_json::from_slice(&published[0].1).unwrap();
    assert_eq!(payload["v"], 1);
    assert_eq!(payload["type"], "wake");
    assert_eq!(payload["id"], id);
    assert_eq!(payload["at"], T0 + 1_000);
    assert_eq!(payload["expires_at"], T0 + 1_000 + 86_400_000);
    assert_eq!(
        payload.as_object().unwrap().len(),
        5,
        "no Reminder content in the signal"
    );
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
    assert_eq!(published(&f, 1).await.len(), 1);
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
    assert_eq!(published(&f, 1).await.len(), 1);

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
    let signals: Vec<serde_json::Value> = published(&f, 2)
        .await
        .iter()
        .map(|(_, payload)| serde_json::from_slice(payload).unwrap())
        .collect();
    // Same Reminder, different trigger moment: the Device can tell the retry from a replay.
    assert_eq!(signals[0]["id"], signals[1]["id"]);
    assert_eq!(signals[0]["at"], T0 + 1_000);
    assert_eq!(signals[1]["at"], T0 + 60_000);
    f.stopping.cancel();
}

#[derive(Clone, Default)]
struct LogCapture(Arc<std::sync::Mutex<Vec<u8>>>);

impl std::io::Write for LogCapture {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for LogCapture {
    type Writer = LogCapture;
    fn make_writer(&'a self) -> Self::Writer {
        self.clone()
    }
}

fn capture_logs() -> (LogCapture, tracing::subscriber::DefaultGuard) {
    let capture = LogCapture::default();
    let guard = tracing::subscriber::set_default(
        tracing_subscriber::fmt()
            .with_writer(capture.clone())
            .with_ansi(false)
            .finish(),
    );
    (capture, guard)
}

impl LogCapture {
    fn count(&self, needle: &str) -> usize {
        String::from_utf8_lossy(&self.0.lock().unwrap())
            .matches(needle)
            .count()
    }
}

const DID_NOT_CONNECT: &str = "wake signal sent but device did not connect";
const COULD_NOT_SEND: &str = "wake signal could not be sent";

#[tokio::test]
async fn signalled_device_that_stays_away_is_logged_once_without_another_signal() {
    let (logs, _guard) = capture_logs();
    let f = fixture(true).await;
    create(&f, "r1", 1).await;
    f.clock.store(T0 + 2_000, Ordering::SeqCst);
    published(&f, 1).await;
    settle().await;
    assert_eq!(logs.count(DID_NOT_CONNECT), 0, "still within the grace");

    // The Device never opens a session: reported once the grace is over, and only once.
    f.clock.store(T0 + 2_000 + 31_000, Ordering::SeqCst);
    f.sessions.scheduler_wakeup().notify_one();
    eventually("miss logged", || logs.count(DID_NOT_CONNECT) == 1).await;
    f.sessions.scheduler_wakeup().notify_one();
    settle().await;
    assert_eq!(logs.count(DID_NOT_CONNECT), 1);
    assert_eq!(logs.count(COULD_NOT_SEND), 0);
    assert_eq!(f.recorder.published().len(), 1, "a miss never re-publishes");
    f.stopping.cancel();
}

#[tokio::test]
async fn failed_publish_is_logged_as_a_send_failure_not_as_a_device_miss() {
    let (logs, _guard) = capture_logs();
    let f = fixture(true).await;
    f.recorder.set_failing(true);
    create(&f, "r1", 1).await;
    f.clock.store(T0 + 2_000 + 31_000, Ordering::SeqCst);
    f.sessions.scheduler_wakeup().notify_one();
    eventually("send failure logged", || logs.count(COULD_NOT_SEND) >= 1).await;
    assert_eq!(logs.count(DID_NOT_CONNECT), 0);
    f.stopping.cancel();
}

#[tokio::test]
async fn device_connecting_after_wake_signal_completes_the_reminder() {
    let f = fixture(true).await;
    let id = create(&f, "r1", 1).await;
    f.clock.store(T0 + 2_000, Ordering::SeqCst);
    assert_eq!(published(&f, 1).await.len(), 1);

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
