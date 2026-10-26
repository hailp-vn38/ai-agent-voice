use std::{sync::Arc, time::Duration};

use voice_agent_server::{
    audio::PcmF32Mono,
    providers::{
        AsrError, AsrEvent, AsrProvider, AsrResult, AsrSession, VadError, VadInput, VadProbability,
        VadProvider, VadSession,
    },
    workers::{
        AsrCommand, AsrWorkerEvent, AsrWorkerRuntime, VadCaptureCycleId, VadCommand,
        VadWorkerEvent, VadWorkerRuntime, WorkerIdentity, WorkerRuntimeConfig,
    },
};

struct FakeAsr;

impl AsrProvider for FakeAsr {
    fn open(&self) -> Result<Box<dyn AsrSession>, AsrError> {
        Ok(Box::new(FakeAsrSession))
    }
}

struct FakeVad;
impl VadProvider for FakeVad {
    fn open(&self) -> Result<Box<dyn VadSession>, VadError> {
        Ok(Box::new(FakeVadSession))
    }
    fn adapter(&self) -> &'static str {
        "fake"
    }
}
struct FakeVadSession;
impl VadSession for FakeVadSession {
    fn push(&mut self, input: VadInput) -> Result<VadProbability, VadError> {
        Ok(VadProbability {
            start_sample: input.start_sample,
            end_sample: input.start_sample + 512,
            probability: 0.0,
        })
    }
    fn reset(&mut self) -> Result<(), VadError> {
        Ok(())
    }
    fn close(&mut self) -> Result<(), VadError> {
        Ok(())
    }
}

struct FakeAsrSession;

impl AsrSession for FakeAsrSession {
    fn push_pcm(&mut self, _: &PcmF32Mono) -> Result<Vec<AsrEvent>, AsrError> {
        Ok(Vec::new())
    }

    fn finish(&mut self) -> Result<AsrResult, AsrError> {
        Ok(AsrResult::new("final"))
    }

    fn cancel(&mut self) {}
}

fn identity(generation: u64) -> WorkerIdentity {
    WorkerIdentity::new("session", generation, 7)
}

#[test]
fn stale_cancel_acknowledgement_releases_its_pinned_stream_without_exposing_a_logical_event() {
    let runtime = AsrWorkerRuntime::new(
        Arc::new(FakeAsr),
        WorkerRuntimeConfig {
            max_workers: 1,
            voice_reserved_capacity: 1,
            command_capacity: 2,
            final_timeout: Duration::from_secs(1),
            cleanup_grace: Duration::from_secs(1),
        },
    );
    let lease = runtime.open(identity(1)).unwrap();
    assert!(matches!(
        runtime.recv_timeout(Duration::from_secs(1)),
        Some(AsrWorkerEvent::Opened { .. })
    ));

    runtime.send(lease, AsrCommand::Cancel).unwrap();
    let acknowledgement = runtime.recv_timeout(Duration::from_secs(1)).unwrap();
    assert!(
        matches!(acknowledgement, AsrWorkerEvent::Cancelled { ref identity } if identity.generation() == 1)
    );
    assert!(runtime.open(identity(2)).is_ok());
}

#[test]
fn cleanup_timeout_quarantines_the_exact_worker_instead_of_reusing_it() {
    let runtime = AsrWorkerRuntime::new(
        Arc::new(FakeAsr),
        WorkerRuntimeConfig {
            max_workers: 1,
            voice_reserved_capacity: 1,
            command_capacity: 1,
            final_timeout: Duration::from_secs(1),
            cleanup_grace: Duration::from_millis(1),
        },
    );
    let lease = runtime.open(identity(3)).unwrap();
    let _ = runtime.recv_timeout(Duration::from_secs(1));
    runtime.send(lease, AsrCommand::Cancel).unwrap();
    std::thread::sleep(Duration::from_millis(2));

    let timeout = runtime.reap_timeouts().unwrap();
    assert!(
        matches!(timeout, AsrWorkerEvent::CleanupTimedOut { ref identity } if identity.generation() == 3)
    );
    assert!(runtime.open(identity(4)).is_err());
}

#[test]
fn vad_reset_and_close_are_acknowledgement_barriers_before_a_slot_is_reused() {
    let runtime = VadWorkerRuntime::new(Arc::new(FakeVad), WorkerRuntimeConfig::default());
    let lease = runtime.open(identity(5)).unwrap();
    assert!(matches!(
        runtime.recv_timeout(Duration::from_secs(1)),
        Some(VadWorkerEvent::Opened { .. })
    ));
    runtime
        .send(
            lease,
            VadCommand::Reset {
                cycle: VadCaptureCycleId::new(1),
            },
        )
        .unwrap();
    assert!(matches!(
        runtime.recv_timeout(Duration::from_secs(1)),
        Some(VadWorkerEvent::ResetDone { .. })
    ));
    runtime.send(lease, VadCommand::Close).unwrap();
    assert!(matches!(
        runtime.recv_timeout(Duration::from_secs(1)),
        Some(VadWorkerEvent::Closed { .. })
    ));
}

#[test]
fn vad_probability_and_reset_acknowledgement_keep_the_capture_cycle_identity() {
    let runtime = VadWorkerRuntime::new(Arc::new(FakeVad), WorkerRuntimeConfig::default());
    let lease = runtime.open(identity(6)).unwrap();
    let _ = runtime.recv_timeout(Duration::from_secs(1));
    let cycle = VadCaptureCycleId::new(42);

    runtime
        .send(
            lease,
            VadCommand::Push {
                cycle,
                pcm: PcmF32Mono::new(vec![0.0; 960], 16_000),
            },
        )
        .unwrap();
    assert!(matches!(
        runtime.recv_timeout(Duration::from_secs(1)),
        Some(VadWorkerEvent::Probability { cycle: event_cycle, .. }) if event_cycle == cycle
    ));

    runtime.send(lease, VadCommand::Reset { cycle }).unwrap();
    assert!(matches!(
        runtime.recv_timeout(Duration::from_secs(1)),
        Some(VadWorkerEvent::ResetDone { cycle: event_cycle, .. }) if event_cycle == cycle
    ));
}

#[test]
fn sequential_asr_and_vad_sessions_reuse_pooled_native_threads() {
    use std::{sync::Mutex, thread::ThreadId};

    struct ThreadRecordingAsrSession(Arc<Mutex<Vec<ThreadId>>>);
    impl AsrSession for ThreadRecordingAsrSession {
        fn push_pcm(&mut self, _: &PcmF32Mono) -> Result<Vec<AsrEvent>, AsrError> {
            Ok(Vec::new())
        }
        fn finish(&mut self) -> Result<AsrResult, AsrError> {
            self.0.lock().unwrap().push(std::thread::current().id());
            Ok(AsrResult::new("final"))
        }
        fn reset(&mut self) -> Result<(), AsrError> {
            Ok(())
        }
        fn cancel(&mut self) {}
    }

    struct ThreadRecordingAsr(Arc<Mutex<Vec<ThreadId>>>);
    impl AsrProvider for ThreadRecordingAsr {
        fn open(&self) -> Result<Box<dyn AsrSession>, AsrError> {
            Ok(Box::new(ThreadRecordingAsrSession(Arc::clone(&self.0))))
        }
    }

    struct ThreadRecordingVadSession(Arc<Mutex<Vec<ThreadId>>>);
    impl VadSession for ThreadRecordingVadSession {
        fn push(&mut self, input: VadInput) -> Result<VadProbability, VadError> {
            Ok(VadProbability {
                start_sample: input.start_sample,
                end_sample: input.start_sample + 512,
                probability: 0.0,
            })
        }
        fn reset(&mut self) -> Result<(), VadError> {
            Ok(())
        }
        fn close(&mut self) -> Result<(), VadError> {
            self.0.lock().unwrap().push(std::thread::current().id());
            Ok(())
        }
    }

    struct ThreadRecordingVad(Arc<Mutex<Vec<ThreadId>>>);
    impl VadProvider for ThreadRecordingVad {
        fn open(&self) -> Result<Box<dyn VadSession>, VadError> {
            Ok(Box::new(ThreadRecordingVadSession(Arc::clone(&self.0))))
        }
        fn adapter(&self) -> &'static str {
            "thread-recording"
        }
    }

    let asr_threads = Arc::new(Mutex::new(Vec::new()));
    let asr_runtime = AsrWorkerRuntime::try_new(
        Arc::new(ThreadRecordingAsr(Arc::clone(&asr_threads))),
        WorkerRuntimeConfig {
            max_workers: 1,
            voice_reserved_capacity: 1,
            ..WorkerRuntimeConfig::default()
        },
    )
    .unwrap();
    asr_threads.lock().unwrap().clear();
    for generation in 1..=3 {
        let lease = asr_runtime.open(identity(generation)).unwrap();
        assert!(matches!(
            asr_runtime.recv_timeout(Duration::from_secs(1)),
            Some(AsrWorkerEvent::Opened { .. })
        ));
        asr_runtime.send(lease, AsrCommand::Finish).unwrap();
        assert!(matches!(
            asr_runtime.recv_timeout(Duration::from_secs(1)),
            Some(AsrWorkerEvent::Final { .. })
        ));
    }
    let recorded_asr = asr_threads.lock().unwrap();
    assert_eq!(recorded_asr.len(), 3);
    assert!(recorded_asr.iter().all(|id| *id == recorded_asr[0]));
    drop(recorded_asr);
    assert!(asr_runtime.shutdown_acknowledged());

    let vad_threads = Arc::new(Mutex::new(Vec::new()));
    let vad_runtime = VadWorkerRuntime::try_new(
        Arc::new(ThreadRecordingVad(Arc::clone(&vad_threads))),
        WorkerRuntimeConfig {
            max_workers: 1,
            voice_reserved_capacity: 1,
            ..WorkerRuntimeConfig::default()
        },
    )
    .unwrap();
    for generation in 1..=3 {
        let lease = vad_runtime.open(identity(generation)).unwrap();
        assert!(matches!(
            vad_runtime.recv_timeout(Duration::from_secs(1)),
            Some(VadWorkerEvent::Opened { .. })
        ));
        vad_runtime.send(lease, VadCommand::Close).unwrap();
        assert!(matches!(
            vad_runtime.recv_timeout(Duration::from_secs(1)),
            Some(VadWorkerEvent::Closed { .. })
        ));
    }
    let recorded_vad = vad_threads.lock().unwrap();
    assert_eq!(recorded_vad.len(), 3);
    assert!(recorded_vad.iter().all(|id| *id == recorded_vad[0]));
    drop(recorded_vad);
    assert!(vad_runtime.shutdown_acknowledged());
}
