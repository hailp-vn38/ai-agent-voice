use std::{
    sync::{
        Arc, Mutex,
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
    vad: Arc<Mutex<Vec<Arc<VadWorkerRuntime>>>>,
}

impl WorkerSupervisor {
    pub fn start(asr: Arc<AsrWorkerRuntime>, vad: Arc<VadWorkerRuntime>) -> Self {
        Self::start_many(vec![asr], vec![vad])
    }

    pub fn start_many(asr: Vec<Arc<AsrWorkerRuntime>>, vad: Vec<Arc<VadWorkerRuntime>>) -> Self {
        let stopping = Arc::new(AtomicBool::new(false));
        let thread_stopping = Arc::clone(&stopping);
        let runtimes = Arc::new(Mutex::new(vad));
        let thread_runtimes = Arc::clone(&runtimes);
        thread::spawn(move || {
            while !thread_stopping.load(Ordering::Acquire) {
                for runtime in &asr {
                    runtime.supervise_pending();
                }
                for runtime in thread_runtimes
                    .lock()
                    .expect("VAD supervisor poisoned")
                    .iter()
                {
                    runtime.supervise_pending();
                }
                thread::sleep(Duration::from_millis(1));
            }
        });
        Self {
            stopping,
            vad: runtimes,
        }
    }

    /// Test-only injection still uses the application-owned supervisor for worker events.
    pub fn observe_vad(&self, runtime: Arc<VadWorkerRuntime>) {
        self.vad
            .lock()
            .expect("VAD supervisor poisoned")
            .push(runtime);
    }
}

impl Drop for WorkerSupervisor {
    fn drop(&mut self) {
        self.stopping.store(true, Ordering::Release);
    }
}
