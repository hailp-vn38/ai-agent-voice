use std::sync::Arc;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use voice_agent_server::{
    protocol::ClientFeatures,
    services::active_device_sessions::ActiveDeviceSessions,
    session::{DirectSpeechError, DirectSpeechRequest, SessionEvent, SpeechOrigin},
};

fn speech() -> DirectSpeechRequest {
    DirectSpeechRequest {
        request_id: "admin-1".into(),
        text: "hello".into(),
        origin: SpeechOrigin::AdminDirect,
    }
}

fn capabilities() -> ClientFeatures {
    ClientFeatures {
        direct_tts: true,
        ..Default::default()
    }
}

#[tokio::test]
async fn registry_replacement_fences_old_cleanup_and_pending_admission() {
    let registry = Arc::new(ActiveDeviceSessions::default());
    let (old_tx, mut old_rx) = mpsc::channel(1);
    let old_close = CancellationToken::new();
    registry.register(1, "old".into(), capabilities(), old_tx, old_close.clone());
    let caller = registry.clone();
    let pending = tokio::spawn(async move { caller.speak(1, speech()).await });
    let Some(SessionEvent::DirectSpeech { reply, .. }) = old_rx.recv().await else {
        panic!("typed request missing")
    };
    let (new_tx, _new_rx) = mpsc::channel(1);
    registry.register(
        1,
        "new".into(),
        capabilities(),
        new_tx,
        CancellationToken::new(),
    );
    assert!(old_close.is_cancelled());
    registry.unregister(1, "old");
    assert_eq!(registry.current(1).unwrap().session_id, "new");
    reply.send(Ok(())).unwrap();
    assert_eq!(
        pending.await.unwrap(),
        Err(DirectSpeechError::InvalidSession)
    );
    registry.unregister(1, "new");
    assert!(registry.current(1).is_none());
}

#[tokio::test]
async fn registry_capacity_and_timeout_are_not_accepted() {
    let registry = Arc::new(ActiveDeviceSessions::default());
    let (tx, mut rx) = mpsc::channel(1);
    tx.try_send(SessionEvent::ClientAudio(Vec::new())).unwrap();
    registry.register(
        1,
        "one".into(),
        capabilities(),
        tx,
        CancellationToken::new(),
    );
    assert_eq!(
        registry.speak(1, speech()).await,
        Err(DirectSpeechError::CapacityReached)
    );
    let _ = rx.recv().await;
    let caller = registry.clone();
    let pending = tokio::spawn(async move { caller.speak(1, speech()).await });
    let Some(SessionEvent::DirectSpeech { reply, .. }) = rx.recv().await else {
        panic!("typed request missing")
    };
    assert_eq!(
        pending.await.unwrap(),
        Err(DirectSpeechError::CapacityReached)
    );
    assert!(
        reply.is_closed(),
        "actor must discard expired admission before starting speech"
    );
}

#[tokio::test]
async fn reminder_snapshot_ack_is_fenced_to_current_session() {
    let registry = ActiveDeviceSessions::default();
    let (old_tx, mut old_rx) = mpsc::channel(4);
    registry.register(
        1,
        "old".into(),
        ClientFeatures {
            reminders: true,
            ..Default::default()
        },
        old_tx,
        CancellationToken::new(),
    );
    registry.sync_required(1, 2);
    assert!(matches!(
        old_rx.recv().await,
        Some(SessionEvent::ReminderControl { .. })
    ));
    assert!(registry.remember_snapshot(1, "old", "snap".into(), 2, vec!["one".into()]));
    let (new_tx, _new_rx) = mpsc::channel(4);
    registry.register(
        1,
        "new".into(),
        ClientFeatures {
            reminders: true,
            ..Default::default()
        },
        new_tx,
        CancellationToken::new(),
    );
    assert!(registry.acknowledge_snapshot(1, "old", "snap", 2).is_none());
    assert!(registry.acknowledge_snapshot(1, "new", "snap", 2).is_none());
}

#[tokio::test]
async fn reminder_two_phase_claim_reaper_fail_and_disconnect_lifecycle() {
    use std::sync::atomic::{AtomicI64, Ordering};
    use voice_agent_server::{
        config::DatabaseConfig,
        database::{
            Database,
            notifications::{CreateReminder, DeviceNotifications, MutationContext, ReminderSource},
        },
        services::notifications::ReminderCoordinator,
    };

    let db = Database::connect(&DatabaseConfig {
        url: "sqlite::memory:".into(),
        ..Default::default()
    })
    .await
    .unwrap();
    let agent_row_id = sqlx::query(
        "INSERT INTO agents (key, name, created_at, updated_at) VALUES ('agent-1', 'Agent 1', 1000, 1000)",
    )
    .execute(db.pool())
    .await
    .unwrap()
    .last_insert_rowid();
    let device_row_id = sqlx::query(
        "INSERT INTO devices (device_id, agent_id, kind, enabled, created_at, updated_at) \
         VALUES ('esp32-1', ?, 'physical', 1, 1000, 1000)",
    )
    .bind(agent_row_id)
    .execute(db.pool())
    .await
    .unwrap()
    .last_insert_rowid();

    let now_ms = Arc::new(AtomicI64::new(1_800_000_000_000));
    let clock_ref = Arc::clone(&now_ms);
    let repo = DeviceNotifications::with_clock(db.clone(), Arc::new(move || clock_ref.load(Ordering::SeqCst)));
    let sessions = Arc::new(ActiveDeviceSessions::default());
    let coordinator = ReminderCoordinator::new(repo, Arc::clone(&sessions));

    let ctx = MutationContext {
        caller: "test",
        request_id: "req-1",
        audit_request_id: "audit-1",
        source: ReminderSource::Voice,
        agent_scope: None,
    };
    let rem = coordinator
        .create(
            "esp32-1",
            &ctx,
            CreateReminder {
                message: "Uống nước".into(),
                delay_seconds: Some(10),
                scheduled_at: None,
                timezone: None,
            },
        )
        .await
        .unwrap();

    // Advance clock to due time (10s later)
    now_ms.store(1_800_000_010_000, Ordering::SeqCst);

    // 1. Phase A claim -> release_unstarted_attempt must NOT consume attempts_used
    let (due1, attempt1) = coordinator
        .repo()
        .claim_next_due_for_device(device_row_id, "sess-1")
        .await
        .unwrap()
        .expect("should claim due reminder");
    assert_eq!(due1.reminder_id, rem.id);
    coordinator
        .repo()
        .release_unstarted_attempt(&rem.id, &attempt1, "sess-1")
        .await
        .unwrap();
    let list_after_release = coordinator.list("esp32-1", None, 1, 10).await.unwrap();
    assert_eq!(list_after_release["items"][0]["attempts_used"], 0);

    // 2. Claim again -> Phase B mark_attempt_started -> fail_attempt #1 (backoff 5s)
    let (_, attempt2) = coordinator
        .repo()
        .claim_next_due_for_device(device_row_id, "sess-1")
        .await
        .unwrap()
        .expect("should claim again immediately after unstarted release");
    assert!(
        coordinator
            .mark_attempt_started(&rem.id, &attempt2, "sess-1", 3_000)
            .await
            .unwrap()
    );
    let fail1 = coordinator
        .fail_attempt(device_row_id, "sess-1", &rem.id, &attempt2, "output_unavailable")
        .await
        .unwrap()
        .unwrap();
    assert!(!fail1.expired);
    // Still in 5s backoff window -> cannot claim yet
    assert!(
        coordinator
            .repo()
            .claim_next_due_for_device(device_row_id, "sess-1")
            .await
            .unwrap()
            .is_none()
    );

    // 3. Advance clock +5s -> Claim #2 -> Start -> Disconnect (on_session_closed marks 'uncertain' + 15s backoff)
    now_ms.fetch_add(5_000, Ordering::SeqCst);
    let (_, attempt3) = coordinator
        .repo()
        .claim_next_due_for_device(device_row_id, "sess-1")
        .await
        .unwrap()
        .expect("should claim after 5s backoff");
    assert!(
        coordinator
            .mark_attempt_started(&rem.id, &attempt3, "sess-1", 3_000)
            .await
            .unwrap()
    );
    assert!(
        coordinator
            .on_session_closed(device_row_id, "sess-1")
            .await
            .unwrap()
            .is_none()
    );

    // 4. Advance clock +15s -> Claim #3 (3rd attempt!) -> Start
    now_ms.fetch_add(15_000, Ordering::SeqCst);
    let (_, attempt4) = coordinator
        .repo()
        .claim_next_due_for_device(device_row_id, "sess-2")
        .await
        .unwrap()
        .expect("should claim 3rd attempt after 15s backoff");
    assert!(
        coordinator
            .mark_attempt_started(&rem.id, &attempt4, "sess-2", 4_000)
            .await
            .unwrap()
    );

    // CRITICAL REAPER INVARIANT: while attempt #3 is actively playing (lease_until_ms > now),
    // reap_expired_leases_and_overdue() MUST NOT expire the reminder!
    let reaped_early = coordinator.reap_expired_leases_and_overdue().await.unwrap();
    assert!(reaped_early.is_empty());
    let list_during_attempt3 = coordinator.list("esp32-1", None, 1, 10).await.unwrap();
    assert_eq!(list_during_attempt3["items"][0]["status"], "pending");
    assert_eq!(list_during_attempt3["items"][0]["attempts_used"], 3);

    // Once lease expires without ACK, reaper marks attempt #3 'uncertain' and expires reminder
    now_ms.fetch_add(30_000, Ordering::SeqCst);
    let reaped_late = coordinator.reap_expired_leases_and_overdue().await.unwrap();
    assert_eq!(reaped_late.len(), 1);
    assert_eq!(reaped_late[0].0, device_row_id);
    let list_after_expire = coordinator.list("esp32-1", None, 1, 10).await.unwrap();
    assert_eq!(list_after_expire["items"][0]["status"], "expired");
}

#[tokio::test]
async fn reminder_speech_origin_bypasses_one_second_rate_limit_for_sequential_fifo() {
    let registry = Arc::new(ActiveDeviceSessions::default());
    let (tx, mut rx) = mpsc::channel(8);
    registry.register(
        1,
        "sess-fifo".into(),
        ClientFeatures {
            reminders: true,
            direct_tts: true,
            ..Default::default()
        },
        tx,
        CancellationToken::new(),
    );

    let acceptor = tokio::spawn(async move {
        while let Some(event) = rx.recv().await {
            if let SessionEvent::DirectSpeech { reply, .. } = event {
                let _ = reply.send(Ok(()));
            }
        }
    });

    // 3 consecutive reminder speeches within <10ms must all succeed without RateLimited
    for i in 1..=3 {
        let res = registry
            .speak(
                1,
                DirectSpeechRequest {
                    request_id: format!("att-{i}"),
                    text: format!("Lời nhắc {i}"),
                    origin: SpeechOrigin::Reminder {
                        reminder_id: format!("rem-{i}"),
                        attempt_id: format!("att-{i}"),
                    },
                },
            )
            .await;
        assert_eq!(res, Ok(()), "reminder #{i} must not be rate-limited");
    }

    // AdminDirect speech still enforces the 1-second rate limit
    assert_eq!(registry.speak(1, speech()).await, Ok(()));
    assert_eq!(
        registry.speak(1, speech()).await,
        Err(DirectSpeechError::RateLimited)
    );
    acceptor.abort();
}

#[tokio::test]
async fn event_driven_scheduler_delivers_on_wakeup_and_defers_when_busy_without_consuming_quota() {
    use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
    use voice_agent_server::{
        config::DatabaseConfig,
        database::{
            Database,
            notifications::{CreateReminder, DeviceNotifications, MutationContext, ReminderSource},
        },
        services::notifications::{ReminderCoordinator, reminder_scheduler},
    };

    let db = Database::connect(&DatabaseConfig {
        url: "sqlite::memory:".into(),
        ..Default::default()
    })
    .await
    .unwrap();
    let agent_row_id = sqlx::query(
        "INSERT INTO agents (key, name, created_at, updated_at) VALUES ('agent-sched', 'Agent Sched', 1000, 1000)",
    )
    .execute(db.pool())
    .await
    .unwrap()
    .last_insert_rowid();
    let device_row_id = sqlx::query(
        "INSERT INTO devices (device_id, agent_id, kind, enabled, created_at, updated_at) \
         VALUES ('esp32-sched', ?, 'physical', 1, 1000, 1000)",
    )
    .bind(agent_row_id)
    .execute(db.pool())
    .await
    .unwrap()
    .last_insert_rowid();

    let now_ms = Arc::new(AtomicI64::new(1_800_000_000_000));
    let clock_ref = Arc::clone(&now_ms);
    let repo = DeviceNotifications::with_clock(
        db.clone(),
        Arc::new(move || clock_ref.load(Ordering::SeqCst)),
    );
    let sessions = Arc::new(ActiveDeviceSessions::default());
    let coordinator = ReminderCoordinator::new(repo.clone(), Arc::clone(&sessions));

    let (tx, mut rx) = mpsc::channel(8);
    sessions.register(
        device_row_id,
        "sess-sched".into(),
        ClientFeatures {
            reminders: true,
            direct_tts: true,
            ..Default::default()
        },
        tx,
        CancellationToken::new(),
    );

    let busy = Arc::new(AtomicBool::new(true));
    let busy_flag = Arc::clone(&busy);
    let (delivered_tx, mut delivered_rx) = mpsc::channel(4);
    let actor_sim = tokio::spawn(async move {
        while let Some(event) = rx.recv().await {
            if let SessionEvent::DirectSpeech { request, reply } = event {
                if busy_flag.load(Ordering::SeqCst) {
                    let _ = reply.send(Err(DirectSpeechError::SessionBusy));
                } else {
                    let _ = reply.send(Ok(()));
                    let _ = delivered_tx.send(request).await;
                }
            }
        }
    });

    let stopping = CancellationToken::new();
    reminder_scheduler::start(repo.clone(), Arc::clone(&sessions), stopping.clone());

    // Create a reminder due in 1 second, then advance fake clock so it is immediately due
    now_ms.store(1_800_000_005_000, Ordering::SeqCst);
    let ctx = MutationContext {
        caller: "test",
        request_id: "sched-req-1",
        audit_request_id: "sched-audit-1",
        source: ReminderSource::Voice,
        agent_scope: None,
    };
    now_ms.store(1_800_000_000_000, Ordering::SeqCst);
    let rem = coordinator
        .create(
            "esp32-sched",
            &ctx,
            CreateReminder {
                message: "Kiểm tra bếp".into(),
                delay_seconds: Some(1),
                scheduled_at: None,
                timezone: None,
            },
        )
        .await
        .unwrap();
    now_ms.store(1_800_000_002_000, Ordering::SeqCst);
    sessions.scheduler_wakeup().notify_one();

    // Give scheduler a moment while actor is busy -> attempts_used must stay 0!
    tokio::time::sleep(std::time::Duration::from_millis(80)).await;
    let list_while_busy = coordinator.list("esp32-sched", None, 1, 10).await.unwrap();
    assert_eq!(list_while_busy["items"][0]["attempts_used"], 0);

    // Turn finishes -> session returns to idle and notifies scheduler_wakeup
    busy.store(false, Ordering::SeqCst);
    sessions.scheduler_wakeup().notify_one();

    let delivered = tokio::time::timeout(std::time::Duration::from_secs(2), delivered_rx.recv())
        .await
        .expect("scheduler should deliver immediately on turn-boundary wakeup")
        .expect("delivered request");
    assert_eq!(delivered.text, "Kiểm tra bếp");
    assert!(matches!(
        delivered.origin,
        SpeechOrigin::Reminder { ref reminder_id, .. } if reminder_id == &rem.id
    ));

    // Wait briefly for mark_attempt_started to commit
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    let list_started = coordinator.list("esp32-sched", None, 1, 10).await.unwrap();
    assert_eq!(list_started["items"][0]["attempts_used"], 1);

    // Ensure no duplicate delivery happens while in_flight_reminder is set
    sessions.scheduler_wakeup().notify_one();
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(80), delivered_rx.recv())
            .await
            .is_err(),
        "must not re-deliver while reminder attempt is in-flight"
    );

    stopping.cancel();
    actor_sim.abort();
}
