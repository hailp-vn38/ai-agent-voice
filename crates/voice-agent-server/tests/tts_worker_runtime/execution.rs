use super::harness::*;

#[test]
fn native_workers_are_created_once_per_pool_slot_not_per_synthesis() {
    let created = Arc::new(AtomicUsize::new(0));
    let runtime = TtsWorkerRuntime::new(
        Arc::new(CountedWorkerProvider(Arc::clone(&created))),
        WorkerRuntimeConfig {
            max_workers: 2,
            voice_reserved_capacity: 1,
            command_capacity: 4,
            final_timeout: Duration::from_secs(1),
            cleanup_grace: Duration::from_millis(10),
        },
    );
    for _ in 0..8 {
        let lease = runtime.start("fixture".into()).unwrap();
        loop {
            if matches!(runtime.poll(lease).unwrap(), Some(TtsWorkerEvent::Finished)) {
                break;
            }
            std::thread::sleep(Duration::from_millis(1));
        }
    }
    assert_eq!(created.load(Ordering::SeqCst), 2);
}

#[test]
fn native_slot_stays_occupied_until_runtime_consumes_terminal_acknowledgement() {
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
    assert!(runtime.start("must not reuse active slot".into()).is_err());
    release_tx.send(()).unwrap();
    let next_event = || {
        for _ in 0..100 {
            if let Some(event) = runtime.poll(lease).unwrap() {
                return event;
            }
            std::thread::sleep(Duration::from_millis(1));
        }
        panic!("native worker did not publish its event")
    };
    assert!(matches!(next_event(), TtsWorkerEvent::Pcm(_)));
    assert_eq!(
        runtime.active_leases(),
        1,
        "PCM is not terminal acknowledgement"
    );
    assert!(matches!(next_event(), TtsWorkerEvent::Finished));
    assert_eq!(runtime.active_leases(), 0);
}

#[test]
fn streaming_provider_delivers_each_pcm_chunk_before_terminal_acknowledgement() {
    let runtime = TtsWorkerRuntime::new(
        Arc::new(StreamingTts),
        WorkerRuntimeConfig {
            max_workers: 1,
            voice_reserved_capacity: 1,
            command_capacity: 4,
            final_timeout: Duration::from_secs(1),
            cleanup_grace: Duration::from_millis(10),
        },
    );
    let lease = runtime.start("non-user fixture".into()).unwrap();
    let next_event = || {
        for _ in 0..100 {
            if let Some(event) = runtime.poll(lease).unwrap() {
                return event;
            }
            std::thread::sleep(Duration::from_millis(1));
        }
        panic!("native worker did not publish its event")
    };

    assert!(matches!(next_event(), TtsWorkerEvent::Pcm(_)));
    assert!(matches!(next_event(), TtsWorkerEvent::Pcm(_)));
    assert_eq!(runtime.active_leases(), 1);
    assert!(matches!(next_event(), TtsWorkerEvent::Finished));
    assert_eq!(runtime.active_leases(), 0);
}

#[test]
fn completed_synthesis_is_not_timed_out_while_pcm_waits_for_pacing() {
    let runtime = TtsWorkerRuntime::new(
        Arc::new(StreamingTts),
        WorkerRuntimeConfig {
            max_workers: 1,
            voice_reserved_capacity: 1,
            command_capacity: 4,
            final_timeout: Duration::from_millis(10),
            cleanup_grace: Duration::from_millis(10),
        },
    );
    let lease = runtime.start("a sentence with queued PCM".into()).unwrap();
    std::thread::sleep(Duration::from_millis(30));

    assert!(matches!(
        runtime.poll(lease).unwrap(),
        Some(TtsWorkerEvent::Pcm(_))
    ));
    assert!(matches!(
        runtime.poll(lease).unwrap(),
        Some(TtsWorkerEvent::Pcm(_))
    ));
    assert!(matches!(
        runtime.poll(lease).unwrap(),
        Some(TtsWorkerEvent::Finished)
    ));
    assert_eq!(runtime.active_leases(), 0);
}
