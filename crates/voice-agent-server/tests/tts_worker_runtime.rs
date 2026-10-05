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
    providers::{TtsBinding, TtsError, TtsProvider, TtsStream, TtsSynthesisRequest, TtsWorker},
    workers::{TtsWorkerEvent, TtsWorkerRuntime, WorkerRuntimeConfig},
};

fn binding(voice: &str) -> TtsBinding {
    TtsBinding {
        voice: voice.into(),
        language: "vi-VN".into(),
    }
}

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
        _: &TtsSynthesisRequest,
        _: &std::sync::atomic::AtomicBool,
        on_pcm: &mut dyn FnMut(PcmF32Mono) -> Result<(), TtsError>,
    ) -> Result<(), TtsError> {
        on_pcm(PcmF32Mono::new(vec![0.1; 96], 48_000))
    }
    fn reset(&mut self) -> Result<(), TtsError> {
        Ok(())
    }
}

struct BindingRecordingProvider(Arc<std::sync::Mutex<Vec<String>>>);

impl TtsProvider for BindingRecordingProvider {
    fn adapter(&self) -> &'static str {
        "binding-recording"
    }

    fn open_worker(&self) -> Result<Box<dyn TtsWorker>, TtsError> {
        Ok(Box::new(BindingRecordingWorker(Arc::clone(&self.0))))
    }
}

struct BindingRecordingWorker(Arc<std::sync::Mutex<Vec<String>>>);

impl TtsWorker for BindingRecordingWorker {
    fn synthesize(
        &mut self,
        request: &TtsSynthesisRequest,
        _: &std::sync::atomic::AtomicBool,
        on_pcm: &mut dyn FnMut(PcmF32Mono) -> Result<(), TtsError>,
    ) -> Result<(), TtsError> {
        self.0.lock().unwrap().push(request.selection.voice.clone());
        on_pcm(PcmF32Mono::new(vec![0.1; 96], 48_000))
    }

    fn reset(&mut self) -> Result<(), TtsError> {
        Ok(())
    }
}

fn wait_for_finish(runtime: &TtsWorkerRuntime, lease: voice_agent_server::workers::TtsLease) {
    loop {
        if matches!(runtime.poll(lease).unwrap(), Some(TtsWorkerEvent::Finished)) {
            return;
        }
        std::thread::sleep(Duration::from_millis(1));
    }
}

#[test]
fn logical_views_inject_their_own_binding_into_shared_workers() {
    let selected = Arc::new(std::sync::Mutex::new(Vec::new()));
    let owner = TtsWorkerRuntime::new_with_binding(
        Arc::new(BindingRecordingProvider(Arc::clone(&selected))),
        WorkerRuntimeConfig::default(),
        binding("maichi"),
    );
    let baotrang = owner.logical_view_for_test(binding("baotrang"));

    let first = owner.start("first".into()).unwrap();
    wait_for_finish(&owner, first);
    let second = baotrang.start("second".into()).unwrap();
    wait_for_finish(&baotrang, second);
    let third = owner.start("third".into()).unwrap();
    wait_for_finish(&owner, third);

    assert_eq!(
        *selected.lock().unwrap(),
        vec!["maichi", "baotrang", "maichi"]
    );
}

#[test]
fn a_stream_rejects_a_segment_from_a_different_logical_binding() {
    let selected = Arc::new(std::sync::Mutex::new(Vec::new()));
    let owner = TtsWorkerRuntime::new_with_binding(
        Arc::new(BindingRecordingProvider(selected)),
        WorkerRuntimeConfig::default(),
        binding("maichi"),
    );
    let alias = owner.logical_view_for_test(binding("baotrang"));
    let stream = owner.begin_stream();

    assert!(alias.start_in_stream(stream, "must fail".into()).is_err());
    let lease = owner
        .start_in_stream(stream, "owner still works".into())
        .unwrap();
    wait_for_finish(&owner, lease);
    owner.close_stream(stream);
}

struct BurstingWorkerProvider(mpsc::Sender<()>);
impl TtsProvider for BurstingWorkerProvider {
    fn adapter(&self) -> &'static str {
        "bursting-worker"
    }

    fn open_worker(&self) -> Result<Box<dyn TtsWorker>, TtsError> {
        Ok(Box::new(BurstingWorker(self.0.clone())))
    }
}

struct BurstingWorker(mpsc::Sender<()>);
impl TtsWorker for BurstingWorker {
    fn synthesize(
        &mut self,
        _: &TtsSynthesisRequest,
        _: &std::sync::atomic::AtomicBool,
        on_pcm: &mut dyn FnMut(PcmF32Mono) -> Result<(), TtsError>,
    ) -> Result<(), TtsError> {
        on_pcm(PcmF32Mono::new(vec![0.1; 2_880], 48_000))?;
        self.0.send(()).unwrap();
        on_pcm(PcmF32Mono::new(vec![0.2; 2_880], 48_000))
    }

    fn reset(&mut self) -> Result<(), TtsError> {
        Ok(())
    }
}

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

struct FailingNativeWorker;
impl TtsProvider for FailingNativeWorker {
    fn adapter(&self) -> &'static str {
        "native-init-failure"
    }
    fn open_worker(&self) -> Result<Box<dyn TtsWorker>, TtsError> {
        Err(TtsError::Failed)
    }
    fn synthesize(&self, _: &str) -> Result<PcmF32Mono, TtsError> {
        panic!("native init failure must never select the compatibility wrapper")
    }
}

#[test]
fn native_worker_init_failure_is_reported_before_runtime_is_published() {
    let result = TtsWorkerRuntime::try_new(
        Arc::new(FailingNativeWorker),
        WorkerRuntimeConfig {
            max_workers: 2,
            voice_reserved_capacity: 1,
            command_capacity: 2,
            final_timeout: Duration::from_secs(1),
            cleanup_grace: Duration::from_secs(1),
        },
    );
    assert!(result.is_err());
}

struct WarmupFailureProvider;
struct WarmupFailureWorker;
impl TtsProvider for WarmupFailureProvider {
    fn adapter(&self) -> &'static str {
        "warmup-failure"
    }
    fn open_worker(&self) -> Result<Box<dyn TtsWorker>, TtsError> {
        Ok(Box::new(WarmupFailureWorker))
    }
}
impl TtsWorker for WarmupFailureWorker {
    fn synthesize(
        &mut self,
        _: &TtsSynthesisRequest,
        _: &std::sync::atomic::AtomicBool,
        _: &mut dyn FnMut(PcmF32Mono) -> Result<(), TtsError>,
    ) -> Result<(), TtsError> {
        panic!("failed warmup cannot publish a worker")
    }
    fn reset(&mut self) -> Result<(), TtsError> {
        Ok(())
    }
    fn warmup(&mut self) -> Result<(), TtsError> {
        Err(TtsError::Failed)
    }
}
#[test]
fn retained_worker_warmup_failure_prevents_ready_publication() {
    assert!(
        TtsWorkerRuntime::try_new(
            Arc::new(WarmupFailureProvider),
            WorkerRuntimeConfig {
                max_workers: 1,
                voice_reserved_capacity: 1,
                command_capacity: 1,
                final_timeout: Duration::from_secs(1),
                cleanup_grace: Duration::from_secs(1),
            }
        )
        .is_err()
    );
}

struct ExitBarrierProvider {
    entered: mpsc::Sender<()>,
    release: Arc<std::sync::Mutex<mpsc::Receiver<()>>>,
}
struct ExitBarrierWorker {
    entered: mpsc::Sender<()>,
    release: Arc<std::sync::Mutex<mpsc::Receiver<()>>>,
}
impl TtsProvider for ExitBarrierProvider {
    fn adapter(&self) -> &'static str {
        "exit-barrier"
    }
    fn open_worker(&self) -> Result<Box<dyn TtsWorker>, TtsError> {
        Ok(Box::new(ExitBarrierWorker {
            entered: self.entered.clone(),
            release: self.release.clone(),
        }))
    }
}
impl TtsWorker for ExitBarrierWorker {
    fn synthesize(
        &mut self,
        _: &TtsSynthesisRequest,
        _: &std::sync::atomic::AtomicBool,
        _: &mut dyn FnMut(PcmF32Mono) -> Result<(), TtsError>,
    ) -> Result<(), TtsError> {
        Ok(())
    }
    fn reset(&mut self) -> Result<(), TtsError> {
        Ok(())
    }
}
impl Drop for ExitBarrierWorker {
    fn drop(&mut self) {
        self.entered.send(()).unwrap();
        self.release.lock().unwrap().recv().unwrap();
    }
}
#[test]
fn unload_waits_for_retained_native_worker_exit_and_closes_admission() {
    let (entered_tx, entered_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let runtime = Arc::new(TtsWorkerRuntime::new(
        Arc::new(ExitBarrierProvider {
            entered: entered_tx,
            release: Arc::new(std::sync::Mutex::new(release_rx)),
        }),
        WorkerRuntimeConfig {
            max_workers: 1,
            voice_reserved_capacity: 1,
            command_capacity: 1,
            final_timeout: Duration::from_secs(1),
            cleanup_grace: Duration::from_secs(1),
        },
    ));
    let (done_tx, done_rx) = mpsc::channel();
    let unloading = runtime.clone();
    let thread =
        std::thread::spawn(move || done_tx.send(unloading.shutdown_acknowledged()).unwrap());
    entered_rx.recv_timeout(Duration::from_secs(1)).unwrap();
    assert!(done_rx.try_recv().is_err());
    assert!(runtime.start("closed admission".into()).is_err());
    release_tx.send(()).unwrap();
    assert!(done_rx.recv_timeout(Duration::from_secs(1)).unwrap());
    thread.join().unwrap();
    assert!(runtime.shutdown_acknowledged());
}

#[test]
fn unload_refuses_an_open_stream_until_the_owner_closes_it() {
    let runtime = TtsWorkerRuntime::new(
        Arc::new(StreamingTts),
        WorkerRuntimeConfig {
            max_workers: 1,
            voice_reserved_capacity: 1,
            command_capacity: 2,
            final_timeout: Duration::from_secs(1),
            cleanup_grace: Duration::from_secs(1),
        },
    );
    let stream = runtime.begin_stream();
    assert!(!runtime.shutdown_acknowledged());
    assert!(
        runtime
            .start_in_stream(stream, "closed admission".into())
            .is_err()
    );
    runtime.close_stream(stream);
    assert!(runtime.shutdown_acknowledged());
}

struct PanickingCleanupProvider;
struct PanickingCleanupWorker;
impl TtsProvider for PanickingCleanupProvider {
    fn adapter(&self) -> &'static str {
        "panicking-cleanup"
    }
    fn open_worker(&self) -> Result<Box<dyn TtsWorker>, TtsError> {
        Ok(Box::new(PanickingCleanupWorker))
    }
}
impl TtsWorker for PanickingCleanupWorker {
    fn warmup(&mut self) -> Result<(), TtsError> {
        Err(TtsError::Failed)
    }
    fn reset(&mut self) -> Result<(), TtsError> {
        Ok(())
    }
    fn synthesize(
        &mut self,
        _: &TtsSynthesisRequest,
        _: &std::sync::atomic::AtomicBool,
        _: &mut dyn FnMut(PcmF32Mono) -> Result<(), TtsError>,
    ) -> Result<(), TtsError> {
        unreachable!()
    }
}
impl Drop for PanickingCleanupWorker {
    fn drop(&mut self) {
        panic!("native cleanup did not acknowledge exit");
    }
}
#[test]
fn initialization_cleanup_panic_is_quarantined_instead_of_a_retryable_load_failure() {
    assert!(matches!(
        TtsWorkerRuntime::try_new(
            Arc::new(PanickingCleanupProvider),
            WorkerRuntimeConfig {
                max_workers: 1,
                voice_reserved_capacity: 1,
                command_capacity: 1,
                final_timeout: Duration::from_secs(1),
                cleanup_grace: Duration::from_secs(1),
            }
        ),
        Err(voice_agent_server::workers::TtsWorkerError::Quarantined)
    ));
}
