use std::{
    sync::{Arc, mpsc},
    time::Duration,
};

use voice_agent_server::{
    audio::PcmF32Mono,
    providers::{TtsError, TtsProvider},
    workers::{TtsWorkerEvent, TtsWorkerRuntime, WorkerRuntimeConfig},
};

struct BlockingTts {
    entered: mpsc::Sender<()>,
    release: std::sync::Mutex<mpsc::Receiver<()>>,
}
impl TtsProvider for BlockingTts {
    fn adapter(&self) -> &'static str {
        "blocking"
    }
    fn synthesize(&self, _: &str) -> Result<PcmF32Mono, TtsError> {
        self.entered.send(()).unwrap();
        self.release.lock().unwrap().recv().unwrap();
        Ok(PcmF32Mono::new(vec![0.25; 96], 48_000))
    }
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
