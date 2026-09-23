use std::{
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
        mpsc,
    },
    time::Duration,
};

use voice_agent_server::{
    audio::PcmF32Mono,
    providers::{TtsError, TtsProvider, TtsStream, TtsWorker},
    workers::{TtsWorkerEvent, TtsWorkerRuntime, WorkerRuntimeConfig},
};

struct BlockingTts {
    entered: mpsc::Sender<()>,
    release: std::sync::Mutex<mpsc::Receiver<()>>,
}

struct StreamingTts;

impl TtsProvider for StreamingTts {
    fn adapter(&self) -> &'static str {
        "streaming"
    }

    fn synthesize_stream(
        &self,
        _: &str,
        on_pcm: &mut dyn FnMut(PcmF32Mono) -> Result<(), TtsError>,
    ) -> Result<(), TtsError> {
        on_pcm(PcmF32Mono::new(vec![0.1; 96], 48_000))?;
        on_pcm(PcmF32Mono::new(vec![0.2; 96], 48_000))
    }
}

struct StatefulTts;

impl TtsProvider for StatefulTts {
    fn adapter(&self) -> &'static str {
        "stateful"
    }

    fn open_stream(&self) -> Option<Box<dyn TtsStream>> {
        Some(Box::new(StatefulStream { segments: 0 }))
    }
}

struct StatefulStream {
    segments: usize,
}

impl TtsStream for StatefulStream {
    fn synthesize(
        &mut self,
        _: &str,
        on_pcm: &mut dyn FnMut(PcmF32Mono) -> Result<(), TtsError>,
    ) -> Result<(), TtsError> {
        self.segments += 1;
        on_pcm(PcmF32Mono::new(vec![self.segments as f32; 96], 48_000))
    }
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

struct CountedWorkerProvider(Arc<AtomicUsize>);
impl TtsProvider for CountedWorkerProvider {
    fn adapter(&self) -> &'static str {
        "counted-worker"
    }
    fn open_worker(&self) -> Result<Box<dyn TtsWorker>, TtsError> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Ok(Box::new(CountedWorker))
    }
}
struct CountedWorker;
impl TtsWorker for CountedWorker {
    fn synthesize(
        &mut self,
        _: &str,
        _: &std::sync::atomic::AtomicBool,
        on_pcm: &mut dyn FnMut(PcmF32Mono) -> Result<(), TtsError>,
    ) -> Result<(), TtsError> {
        on_pcm(PcmF32Mono::new(vec![0.1; 96], 48_000))
    }
    fn reset(&mut self) -> Result<(), TtsError> {
        Ok(())
    }
}

#[test]
fn native_workers_are_created_once_per_pool_slot_not_per_synthesis() {
    let created = Arc::new(AtomicUsize::new(0));
    let runtime = TtsWorkerRuntime::new(
        Arc::new(CountedWorkerProvider(Arc::clone(&created))),
        WorkerRuntimeConfig {
            max_workers: 2,
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
fn one_tts_stream_preserves_provider_state_across_speech_segments() {
    let runtime = TtsWorkerRuntime::new(Arc::new(StatefulTts), WorkerRuntimeConfig::default());
    let stream = runtime.begin_stream();
    for expected in [1.0, 2.0] {
        let lease = runtime.start_in_stream(stream, "segment".into()).unwrap();
        let pcm = loop {
            if let Some(TtsWorkerEvent::Pcm(pcm)) = runtime.poll(lease).unwrap() {
                break pcm;
            }
            std::thread::sleep(Duration::from_millis(1));
        };
        assert_eq!(pcm.samples()[0], expected);
        loop {
            if matches!(runtime.poll(lease).unwrap(), Some(TtsWorkerEvent::Finished)) {
                break;
            }
            std::thread::sleep(Duration::from_millis(1));
        }
    }
    runtime.close_stream(stream);
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
