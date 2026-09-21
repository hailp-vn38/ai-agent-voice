use std::{sync::Arc, time::Duration};

use voice_agent_server::{
    audio::PcmF32Mono,
    providers::{
        AsrError, AsrEvent, AsrProvider, AsrResult, AsrSession, VadError, VadEvent, VadProvider,
        VadSession,
    },
    workers::{
        AsrCommand, AsrWorkerEvent, AsrWorkerRuntime, VadCommand, VadWorkerEvent, VadWorkerRuntime,
        WorkerIdentity, WorkerRuntimeConfig,
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
    fn push_pcm(&mut self, _: &PcmF32Mono) -> Result<Vec<VadEvent>, VadError> {
        Ok(Vec::new())
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
    runtime.send(lease, VadCommand::Reset).unwrap();
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
