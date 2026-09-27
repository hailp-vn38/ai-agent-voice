use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::Duration,
};

use super::{AsrWorkerRuntime, VadWorkerRuntime};

/// Sole production driver for worker event routing and timeout quarantine.
/// It outlives individual Voice Sessions, so disconnect cannot suppress cleanup.
pub struct WorkerSupervisor {
    stopping: Arc<AtomicBool>,
}

impl WorkerSupervisor {
    pub fn start(asr: Arc<AsrWorkerRuntime>, vad: Arc<VadWorkerRuntime>) -> Self {
        Self::start_many(vec![asr], vec![vad])
    }

    pub fn start_many(asr: Vec<Arc<AsrWorkerRuntime>>, vad: Vec<Arc<VadWorkerRuntime>>) -> Self {
        let stopping = Arc::new(AtomicBool::new(false));
        let thread_stopping = Arc::clone(&stopping);
        thread::spawn(move || {
            while !thread_stopping.load(Ordering::Acquire) {
                for runtime in &asr {
                    runtime.supervise_pending();
                }
                for runtime in &vad {
                    runtime.supervise_pending();
                }
                thread::sleep(Duration::from_millis(1));
            }
        });
        Self { stopping }
    }
}

impl Drop for WorkerSupervisor {
    fn drop(&mut self) {
        self.stopping.store(true, Ordering::Release);
    }
}
