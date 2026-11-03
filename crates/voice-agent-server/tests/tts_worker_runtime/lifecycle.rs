use super::harness::*;

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
