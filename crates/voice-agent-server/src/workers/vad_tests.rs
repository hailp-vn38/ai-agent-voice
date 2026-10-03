use std::sync::Arc;

use crate::providers::{VadError, VadInput, VadProbability, VadProvider, VadSession};

use super::{VadCommand, VadWorkerRuntime, WorkerIdentity, WorkerRuntimeConfig};

struct TestVad;

impl VadProvider for TestVad {
    fn open(&self) -> Result<Box<dyn VadSession>, VadError> {
        Ok(Box::new(TestVadSession))
    }

    fn adapter(&self) -> &'static str {
        "test-vad"
    }
}

struct TestVadSession;

impl VadSession for TestVadSession {
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
}

#[test]
fn a_diagnostic_worker_lease_leaves_the_voice_reservation_available() {
    let runtime = VadWorkerRuntime::new(
        Arc::new(TestVad),
        WorkerRuntimeConfig {
            max_workers: 2,
            voice_reserved_capacity: 1,
            command_capacity: 1,
            final_timeout: std::time::Duration::from_secs(1),
            cleanup_grace: std::time::Duration::from_secs(1),
        },
    );
    let diagnostic = runtime
        .open_diagnostic(WorkerIdentity::new("diagnostic", 0, 0))
        .expect("the service already owns the diagnostic permit");
    let voice = runtime
        .open(WorkerIdentity::new("voice", 0, 0))
        .expect("the voice reservation remains available");
    runtime.send(diagnostic, VadCommand::Close).unwrap();
    runtime.send(voice, VadCommand::Close).unwrap();
}
