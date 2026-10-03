use std::{
    sync::{Arc, Mutex, mpsc},
    time::Duration,
};
use voice_agent_server::{
    audio::PcmF32Mono,
    providers::{AsrError, AsrEvent, AsrProvider, AsrResult, AsrSession},
    workers::{AsrCommand, AsrWorkerEvent, AsrWorkerRuntime, WorkerIdentity, WorkerRuntimeConfig},
};

struct BarrierProvider {
    entered: mpsc::Sender<()>,
    release: Arc<Mutex<mpsc::Receiver<()>>>,
}
struct BarrierSession {
    entered: mpsc::Sender<()>,
    release: Arc<Mutex<mpsc::Receiver<()>>>,
}
impl AsrProvider for BarrierProvider {
    fn open(&self) -> Result<Box<dyn AsrSession>, AsrError> {
        Ok(Box::new(BarrierSession {
            entered: self.entered.clone(),
            release: self.release.clone(),
        }))
    }
}
impl AsrSession for BarrierSession {
    fn push_pcm(&mut self, _: &PcmF32Mono) -> Result<Vec<AsrEvent>, AsrError> {
        Ok(vec![])
    }
    fn finish(&mut self) -> Result<AsrResult, AsrError> {
        Ok(AsrResult::new("fixture"))
    }
    fn cancel(&mut self) {}
}
impl Drop for BarrierSession {
    fn drop(&mut self) {
        self.entered.send(()).unwrap();
        self.release.lock().unwrap().recv().unwrap();
    }
}
#[test]
fn asr_final_event_does_not_acknowledge_native_exit_or_release_capacity() {
    let (entered_tx, entered_rx) = mpsc::channel();
    let (release_tx, release_rx) = mpsc::channel();
    let runtime = AsrWorkerRuntime::new(
        Arc::new(BarrierProvider {
            entered: entered_tx,
            release: Arc::new(Mutex::new(release_rx)),
        }),
        WorkerRuntimeConfig {
            max_workers: 1,
            voice_reserved_capacity: 1,
            command_capacity: 2,
            final_timeout: Duration::from_secs(1),
            cleanup_grace: Duration::from_secs(1),
        },
    );
    let lease = runtime.open(WorkerIdentity::new("session", 1, 1)).unwrap();
    assert!(matches!(
        runtime.recv_timeout(Duration::from_secs(1)),
        Some(AsrWorkerEvent::Opened { .. })
    ));
    runtime.send(lease, AsrCommand::Finish).unwrap();
    assert!(matches!(
        runtime.recv_timeout(Duration::from_secs(1)),
        Some(AsrWorkerEvent::Final { .. })
    ));
    entered_rx.recv_timeout(Duration::from_secs(1)).unwrap();
    assert!(runtime.open(WorkerIdentity::new("session", 1, 2)).is_err());
    assert!(!runtime.shutdown_acknowledged());
    release_tx.send(()).unwrap();
    let deadline = std::time::Instant::now() + Duration::from_secs(1);
    while !runtime.shutdown_acknowledged() {
        assert!(std::time::Instant::now() < deadline);
        std::thread::yield_now();
    }
    assert!(runtime.open(WorkerIdentity::new("session", 1, 3)).is_err());
}

struct CountedVad {
    opens: Arc<std::sync::atomic::AtomicUsize>,
    drops: Arc<std::sync::atomic::AtomicUsize>,
}
struct CountedVadSession {
    samples: u64,
    drops: Arc<std::sync::atomic::AtomicUsize>,
}
impl voice_agent_server::providers::VadProvider for CountedVad {
    fn adapter(&self) -> &'static str {
        "counted"
    }
    fn open(
        &self,
    ) -> Result<
        Box<dyn voice_agent_server::providers::VadSession>,
        voice_agent_server::providers::VadError,
    > {
        self.opens.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Ok(Box::new(CountedVadSession {
            samples: 0,
            drops: self.drops.clone(),
        }))
    }
}
impl voice_agent_server::providers::VadSession for CountedVadSession {
    fn push(
        &mut self,
        input: voice_agent_server::providers::VadInput,
    ) -> Result<
        voice_agent_server::providers::VadProbability,
        voice_agent_server::providers::VadError,
    > {
        self.samples += 512;
        Ok(voice_agent_server::providers::VadProbability {
            start_sample: input.start_sample,
            end_sample: input.start_sample + 512,
            probability: if self.samples == 512 { 0.1 } else { 0.2 },
        })
    }
    fn reset(&mut self) -> Result<(), voice_agent_server::providers::VadError> {
        self.samples = 0;
        Ok(())
    }
}
impl Drop for CountedVadSession {
    fn drop(&mut self) {
        self.drops.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    }
}
#[test]
fn ready_vad_retains_warmed_workers_and_resets_between_session_owners() {
    use voice_agent_server::workers::{
        VadCaptureCycleId, VadCommand, VadWorkerEvent, VadWorkerRuntime,
    };
    let opens = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let drops = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let runtime = VadWorkerRuntime::try_new(
        Arc::new(CountedVad {
            opens: opens.clone(),
            drops: drops.clone(),
        }),
        WorkerRuntimeConfig {
            max_workers: 1,
            voice_reserved_capacity: 1,
            command_capacity: 2,
            final_timeout: Duration::from_secs(1),
            cleanup_grace: Duration::from_secs(1),
        },
    )
    .unwrap();
    assert_eq!(opens.load(std::sync::atomic::Ordering::SeqCst), 1);
    for session in ["first", "second"] {
        let deadline = std::time::Instant::now() + Duration::from_secs(1);
        let lease = loop {
            if let Ok(lease) = runtime.open(WorkerIdentity::new(session, 1, 1)) {
                break lease;
            }
            assert!(std::time::Instant::now() < deadline);
            std::thread::yield_now();
        };
        assert!(matches!(
            runtime.recv_timeout(Duration::from_secs(1)),
            Some(VadWorkerEvent::Opened { .. })
        ));
        runtime
            .send(
                lease,
                VadCommand::Push {
                    cycle: VadCaptureCycleId::new(1),
                    pcm: PcmF32Mono::new(vec![0.0; 960], 16_000),
                },
            )
            .unwrap();
        let Some(VadWorkerEvent::Probability { probability, .. }) =
            runtime.recv_timeout(Duration::from_secs(1))
        else {
            panic!("probability missing");
        };
        assert_eq!(
            probability.probability, 0.1,
            "warmup and previous session state must have been reset"
        );
        runtime.send(lease, VadCommand::Close).unwrap();
        assert!(matches!(
            runtime.recv_timeout(Duration::from_secs(1)),
            Some(VadWorkerEvent::Closed { .. })
        ));
    }
    let deadline = std::time::Instant::now() + Duration::from_secs(1);
    while !runtime.shutdown_acknowledged() {
        assert!(std::time::Instant::now() < deadline);
        std::thread::yield_now();
    }
    assert_eq!(opens.load(std::sync::atomic::Ordering::SeqCst), 1);
    assert_eq!(drops.load(std::sync::atomic::Ordering::SeqCst), 1);
}

struct CountedAsr {
    opens: Arc<std::sync::atomic::AtomicUsize>,
}
struct CountedAsrSession {
    samples: usize,
    cancelled: bool,
}
impl AsrProvider for CountedAsr {
    fn open(&self) -> Result<Box<dyn AsrSession>, AsrError> {
        self.opens.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Ok(Box::new(CountedAsrSession {
            samples: 0,
            cancelled: false,
        }))
    }
}
impl AsrSession for CountedAsrSession {
    fn push_pcm(&mut self, pcm: &PcmF32Mono) -> Result<Vec<AsrEvent>, AsrError> {
        assert!(!self.cancelled);
        self.samples += pcm.samples().len();
        Ok(vec![])
    }
    fn finish(&mut self) -> Result<AsrResult, AsrError> {
        assert!(!self.cancelled);
        Ok(AsrResult::new(self.samples.to_string()))
    }
    fn cancel(&mut self) {
        self.cancelled = true;
    }
    fn reset(&mut self) -> Result<(), AsrError> {
        self.samples = 0;
        self.cancelled = false;
        Ok(())
    }
}
#[test]
fn ready_asr_retains_warmed_workers_without_carrying_audio_between_sessions() {
    let opens = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let runtime = AsrWorkerRuntime::try_new(
        Arc::new(CountedAsr {
            opens: opens.clone(),
        }),
        WorkerRuntimeConfig {
            max_workers: 1,
            voice_reserved_capacity: 1,
            command_capacity: 2,
            final_timeout: Duration::from_secs(1),
            cleanup_grace: Duration::from_secs(1),
        },
    )
    .unwrap();
    for session in ["first", "second"] {
        let deadline = std::time::Instant::now() + Duration::from_secs(1);
        let lease = loop {
            if let Ok(lease) = runtime.open(WorkerIdentity::new(session, 1, 1)) {
                break lease;
            }
            assert!(std::time::Instant::now() < deadline);
            std::thread::yield_now();
        };
        assert!(matches!(
            runtime.recv_timeout(Duration::from_secs(1)),
            Some(AsrWorkerEvent::Opened { .. })
        ));
        runtime
            .send(
                lease,
                AsrCommand::Push(PcmF32Mono::new(vec![0.0; 960], 16_000)),
            )
            .unwrap();
        runtime.send(lease, AsrCommand::Finish).unwrap();
        let Some(AsrWorkerEvent::Final { text, .. }) = runtime.recv_timeout(Duration::from_secs(1))
        else {
            panic!("final missing");
        };
        assert_eq!(
            text, "960",
            "warmup and previous audio must not enter this session"
        );
    }
    assert_eq!(opens.load(std::sync::atomic::Ordering::SeqCst), 1);
}

#[tokio::test]
async fn asr_diagnostics_reuse_retained_sessions_and_acknowledge_reset_before_next_request() {
    use voice_agent_server::services::provider_diagnostic::ProviderDiagnosticOperation;
    let opens = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let runtime = AsrWorkerRuntime::try_new(
        Arc::new(CountedAsr {
            opens: opens.clone(),
        }),
        WorkerRuntimeConfig {
            max_workers: 1,
            voice_reserved_capacity: 1,
            command_capacity: 2,
            final_timeout: Duration::from_secs(1),
            cleanup_grace: Duration::from_secs(1),
        },
    )
    .unwrap();
    for samples in [960, 1920, 960] {
        let mut operation = runtime.diagnostic(PcmF32Mono::new(vec![0.0; samples], 16_000));
        assert_eq!(
            operation
                .execute(tokio_util::sync::CancellationToken::new())
                .await
                .unwrap(),
            samples.to_string()
        );
        assert!(operation.await_terminal_acknowledgement().await);
    }
    assert_eq!(opens.load(std::sync::atomic::Ordering::SeqCst), 1);
    assert!(runtime.shutdown_acknowledged());
}
