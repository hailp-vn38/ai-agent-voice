//! Bounded local-inference workers. They own mutable provider streams; session actors only route
//! identity-tagged commands and events.

mod asr;
mod supervisor;
mod vad;

pub use asr::{AsrCommand, AsrStreamLease, AsrWorkerEvent, AsrWorkerRuntime};
pub use supervisor::WorkerSupervisor;
pub use vad::{VadCommand, VadWorkerError, VadWorkerEvent, VadWorkerLease, VadWorkerRuntime};

use std::time::Duration;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct WorkerIdentity {
    session: String,
    generation: u64,
    stream: u64,
}

impl WorkerIdentity {
    pub fn new(session: impl Into<String>, generation: u64, stream: u64) -> Self {
        Self {
            session: session.into(),
            generation,
            stream,
        }
    }

    pub fn session(&self) -> &str {
        &self.session
    }
    pub fn generation(&self) -> u64 {
        self.generation
    }
    pub fn stream(&self) -> u64 {
        self.stream
    }
}

#[derive(Clone, Debug)]
pub struct WorkerRuntimeConfig {
    pub max_workers: usize,
    pub command_capacity: usize,
    pub final_timeout: Duration,
    pub cleanup_grace: Duration,
}

impl Default for WorkerRuntimeConfig {
    fn default() -> Self {
        Self {
            max_workers: 8,
            command_capacity: 32,
            final_timeout: Duration::from_secs(15),
            cleanup_grace: Duration::from_secs(5),
        }
    }
}

impl WorkerRuntimeConfig {
    pub(crate) fn validate(&self) -> Result<(), &'static str> {
        if self.max_workers == 0 || self.command_capacity == 0 {
            return Err("worker and command capacities must be positive");
        }
        if self.final_timeout.is_zero() || self.cleanup_grace.is_zero() {
            return Err("worker timeouts must be positive");
        }
        Ok(())
    }
}
