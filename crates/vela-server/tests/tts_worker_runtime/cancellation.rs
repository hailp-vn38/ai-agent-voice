use super::harness::*;

#[test]
fn detached_cancel_drains_queued_pcm_and_releases_full_event_channel() {
    let (first_sent, first_received) = mpsc::channel();
    let runtime = TtsWorkerRuntime::new(
        Arc::new(BurstingWorkerProvider(first_sent)),
        WorkerRuntimeConfig {
            max_workers: 1,
            voice_reserved_capacity: 1,
            command_capacity: 1,
            final_timeout: Duration::from_secs(1),
            cleanup_grace: Duration::from_secs(1),
        },
    );
    let lease = runtime.start("file replay".into()).unwrap();
    first_received.recv_timeout(Duration::from_secs(1)).unwrap();
    runtime.cancel_and_detach(lease).unwrap();
    for _ in 0..100 {
        if runtime.active_leases() == 0 {
            break;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(runtime.active_leases(), 0);
    let next = runtime
        .start("slot reused after acknowledgement".into())
        .unwrap();
    runtime.cancel_and_detach(next).unwrap();
}

#[test]
fn detached_cancel_quarantines_a_worker_that_misses_cleanup_grace() {
    let (entered_tx, entered_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let runtime = TtsWorkerRuntime::new(
        Arc::new(BlockingTts {
            entered: entered_tx,
            release: std::sync::Mutex::new(release_rx),
        }),
        WorkerRuntimeConfig {
            max_workers: 1,
            voice_reserved_capacity: 1,
            command_capacity: 2,
            final_timeout: Duration::from_secs(1),
            cleanup_grace: Duration::from_millis(10),
        },
    );
    let lease = runtime.start("non-user fixture".into()).unwrap();
    entered_rx.recv_timeout(Duration::from_secs(1)).unwrap();
    runtime.cancel_and_detach(lease).unwrap();
    std::thread::sleep(Duration::from_millis(30));
    assert!(
        runtime
            .start("quarantined slot is not reusable".into())
            .is_err()
    );
    release_tx.send(()).unwrap();
}

#[test]
fn timeout_starts_cleanup_from_worker_acceptance_and_quarantines_without_later_polling() {
    let (entered_tx, entered_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let runtime = TtsWorkerRuntime::new(
        Arc::new(BlockingTts {
            entered: entered_tx,
            release: std::sync::Mutex::new(release_rx),
        }),
        WorkerRuntimeConfig {
            max_workers: 1,
            voice_reserved_capacity: 1,
            command_capacity: 2,
            final_timeout: Duration::from_millis(10),
            cleanup_grace: Duration::from_millis(10),
        },
    );
    let lease = runtime.start("non-user fixture".into()).unwrap();
    entered_rx.recv_timeout(Duration::from_secs(1)).unwrap();
    std::thread::sleep(Duration::from_millis(15));
    assert!(matches!(
        runtime.poll(lease).unwrap(),
        Some(TtsWorkerEvent::TimedOut)
    ));
    runtime.cancel_and_detach(lease).unwrap();
    std::thread::sleep(Duration::from_millis(30));
    assert!(
        runtime
            .start("timed-out slot is quarantined".into())
            .is_err()
    );
    release_tx.send(()).unwrap();
}
