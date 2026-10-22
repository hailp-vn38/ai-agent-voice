use super::*;
use crate::providers::{AsrError, AsrEvent, AsrResult, AsrSession};
use std::sync::{Condvar, atomic::AtomicUsize};

#[derive(Default)]
struct Gate {
    entered: AtomicBool,
    open: Mutex<bool>,
    wake: Condvar,
}
impl Gate {
    fn block(&self) {
        self.entered.store(true, Ordering::Release);
        let mut open = self.open.lock().unwrap();
        while !*open {
            open = self.wake.wait(open).unwrap();
        }
    }
    fn release(&self) {
        *self.open.lock().unwrap() = true;
        self.wake.notify_all();
    }
    fn wait(&self) {
        let deadline = Instant::now() + Duration::from_secs(2);
        while !self.entered.load(Ordering::Acquire) {
            assert!(Instant::now() < deadline, "worker did not reach gate");
            thread::yield_now();
        }
    }
}
struct Provider {
    pushes: Arc<AtomicUsize>,
    finish: Arc<Gate>,
    reset: Arc<Gate>,
}
struct Session {
    pushes: Arc<AtomicUsize>,
    finish: Arc<Gate>,
    reset: Arc<Gate>,
}
impl AsrProvider for Provider {
    fn open(&self) -> Result<Box<dyn AsrSession>, AsrError> {
        Ok(Box::new(Session {
            pushes: self.pushes.clone(),
            finish: self.finish.clone(),
            reset: self.reset.clone(),
        }))
    }
}
impl AsrSession for Session {
    fn push_pcm(&mut self, pcm: &PcmF32Mono) -> Result<Vec<AsrEvent>, AsrError> {
        assert_eq!(pcm.sample_rate_hz(), 16000);
        self.pushes.fetch_add(pcm.samples().len(), Ordering::SeqCst);
        Ok(vec![])
    }
    fn finish(&mut self) -> Result<AsrResult, AsrError> {
        self.finish.block();
        Ok(AsrResult::new("Xin chào"))
    }
    fn cancel(&mut self) {
        self.reset.block();
    }
}
fn fixture() -> (AsrWorkerRuntime, Arc<Gate>, Arc<Gate>, Arc<AtomicUsize>) {
    let finish = Arc::new(Gate::default());
    let reset = Arc::new(Gate::default());
    let pushes = Arc::new(AtomicUsize::new(0));
    let runtime = AsrWorkerRuntime::new(
        Arc::new(Provider {
            pushes: pushes.clone(),
            finish: finish.clone(),
            reset: reset.clone(),
        }),
        WorkerRuntimeConfig {
            max_workers: 2,
            voice_reserved_capacity: 1,
            command_capacity: 2,
            cleanup_grace: Duration::from_millis(30),
            ..Default::default()
        },
    );
    (runtime, finish, reset, pushes)
}
#[test]
fn asr_background_reserves_capacity_pins_stream_and_isolates_voice_events() {
    let (runtime, finish, reset, pushes) = fixture();
    finish.release();
    reset.release();
    let operation = runtime.open_background().unwrap();
    assert!(matches!(
        runtime.open_background(),
        Err(AsrWorkerError::Capacity)
    ));
    let voice = runtime.open(WorkerIdentity::new("voice", 1, 1)).unwrap();
    operation
        .send(AsrCommand::Push(PcmF32Mono::from_samples(vec![0.0; 960])))
        .unwrap();
    loop {
        match operation.send(AsrCommand::Finish) {
            Ok(()) => break,
            Err(AsrWorkerError::QueueFull) => thread::yield_now(),
            Err(e) => panic!("{e}"),
        }
    }
    let deadline = Instant::now() + Duration::from_secs(2);
    let mut text = None;
    while text.is_none() {
        runtime.supervise_pending();
        if let Some(AsrWorkerEvent::Final {
            text: final_text, ..
        }) = operation.poll()
        {
            text = Some(final_text);
        }
        assert!(Instant::now() < deadline);
        thread::yield_now();
    }
    assert_eq!(text.as_deref(), Some("Xin chào"));
    assert_eq!(pushes.load(Ordering::SeqCst), 960);
    assert!(operation.cleanup());
    runtime.send(voice, AsrCommand::Cancel).unwrap();
    while runtime.pilot_work_pending() {
        runtime.supervise_pending();
        assert!(Instant::now() < deadline);
        thread::yield_now();
    }
    assert!(runtime.open_background().is_ok());
}
#[test]
fn asr_background_backpressure_keeps_finish_atomic_and_quarantines_unreaped_decode() {
    let (runtime, finish, reset, _) = fixture();
    reset.release();
    let operation = runtime.open_background().unwrap();
    operation.send(AsrCommand::Finish).unwrap();
    finish.wait();
    operation.send(AsrCommand::Cancel).unwrap();
    operation.send(AsrCommand::Cancel).unwrap();
    assert!(matches!(
        operation.send(AsrCommand::Finish),
        Err(AsrWorkerError::QueueFull)
    ));
    assert!(!operation.cleanup());
    assert!(!operation.acknowledged());
    assert!(matches!(
        runtime.open_background(),
        Err(AsrWorkerError::Capacity)
    ));
    finish.release();
    let deadline = Instant::now() + Duration::from_secs(2);
    while !operation.acknowledged() {
        assert!(Instant::now() < deadline);
        thread::yield_now();
    }
    assert!(matches!(
        runtime.open_background(),
        Err(AsrWorkerError::Capacity)
    ));
}
#[test]
fn asr_background_drop_waits_for_exact_cleanup_acknowledgement() {
    let (runtime, finish, reset, _) = fixture();
    finish.release();
    let operation = runtime.open_background().unwrap();
    drop(operation);
    reset.wait();
    assert!(matches!(
        runtime.open_background(),
        Err(AsrWorkerError::Capacity)
    ));
    reset.release();
    let deadline = Instant::now() + Duration::from_secs(2);
    while runtime.pilot_work_pending() {
        assert!(Instant::now() < deadline);
        thread::yield_now();
    }
    assert!(runtime.open_background().is_ok());
}
#[test]
fn asr_background_rejects_provider_with_all_capacity_reserved_for_voice() {
    let (mut runtime, _, _, _) = fixture();
    runtime.admission = ProviderRuntimeAdmission::new(1, 1);
    assert!(!runtime.has_background_capacity());
    assert!(matches!(
        runtime.open_background(),
        Err(AsrWorkerError::Capacity)
    ));
}

#[test]
fn asr_background_retained_worker_acknowledges_only_after_its_reset() {
    struct RetainedProvider(Arc<Gate>);
    struct Retained {
        gate: Arc<Gate>,
        resets: usize,
    }
    impl AsrProvider for RetainedProvider {
        fn open(&self) -> Result<Box<dyn AsrSession>, AsrError> {
            Ok(Box::new(Retained {
                gate: self.0.clone(),
                resets: 0,
            }))
        }
    }
    impl AsrSession for Retained {
        fn push_pcm(&mut self, _: &PcmF32Mono) -> Result<Vec<AsrEvent>, AsrError> {
            Ok(vec![])
        }
        fn finish(&mut self) -> Result<AsrResult, AsrError> {
            Ok(AsrResult::new("final"))
        }
        fn cancel(&mut self) {}
        fn reset(&mut self) -> Result<(), AsrError> {
            self.resets += 1;
            if self.resets > 1 {
                self.gate.block();
            }
            Ok(())
        }
    }
    let reset = Arc::new(Gate::default());
    let runtime = AsrWorkerRuntime::try_new(
        Arc::new(RetainedProvider(reset.clone())),
        WorkerRuntimeConfig {
            max_workers: 2,
            ..Default::default()
        },
    )
    .unwrap();
    let operation = runtime.open_background().unwrap();
    operation.send(AsrCommand::Finish).unwrap();
    reset.wait();
    assert!(!operation.acknowledged());
    assert!(matches!(
        runtime.open_background(),
        Err(AsrWorkerError::Capacity)
    ));
    while let Some(event) = operation.poll() {
        assert!(matches!(event, AsrWorkerEvent::Opened { .. }));
    }
    reset.release();
    let deadline = Instant::now() + Duration::from_secs(2);
    while !operation.acknowledged() {
        assert!(Instant::now() < deadline);
        thread::yield_now();
    }
    assert!(operation.cleanup());
    assert!(runtime.open_background().is_ok());
}
