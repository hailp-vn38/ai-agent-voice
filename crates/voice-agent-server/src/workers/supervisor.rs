use std::{
    sync::{
        Arc, Mutex, Weak,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::Duration,
};

use super::{AsrWorkerRuntime, VadWorkerRuntime};

/// Sole production driver for worker event routing and timeout quarantine.
/// Weak observations allow the runtime manager to own retention and unload.
/// The resource remains owned by its manager while native cleanup is pending.
pub struct WorkerSupervisor {
    stopping: Arc<AtomicBool>,
    asr: Arc<Mutex<Vec<Weak<AsrWorkerRuntime>>>>,
    vad: Arc<Mutex<Vec<Weak<VadWorkerRuntime>>>>,
    thread: Option<thread::JoinHandle<()>>,
}

impl WorkerSupervisor {
    pub fn start(asr: Arc<AsrWorkerRuntime>, vad: Arc<VadWorkerRuntime>) -> Self {
        Self::start_many(vec![asr], vec![vad])
    }

    pub fn start_many(asr: Vec<Arc<AsrWorkerRuntime>>, vad: Vec<Arc<VadWorkerRuntime>>) -> Self {
        let stopping = Arc::new(AtomicBool::new(false));
        let thread_stopping = Arc::clone(&stopping);
        let asr = Arc::new(Mutex::new(asr.iter().map(Arc::downgrade).collect()));
        let vad = Arc::new(Mutex::new(vad.iter().map(Arc::downgrade).collect()));
        let thread_asr = Arc::clone(&asr);
        let thread_vad = Arc::clone(&vad);
        let thread = thread::spawn(move || {
            while !thread_stopping.load(Ordering::Acquire) {
                for runtime in live(&thread_asr) {
                    runtime.supervise_pending();
                }
                for runtime in live(&thread_vad) {
                    runtime.supervise_pending();
                }
                thread::sleep(Duration::from_millis(1));
            }
        });
        Self {
            stopping,
            asr,
            vad,
            thread: Some(thread),
        }
    }

    pub fn observe_asr(&self, runtime: Arc<AsrWorkerRuntime>) {
        observe(&self.asr, &runtime);
    }
    pub fn observe_vad(&self, runtime: Arc<VadWorkerRuntime>) {
        observe(&self.vad, &runtime);
    }
}

fn observe<T>(runtimes: &Mutex<Vec<Weak<T>>>, runtime: &Arc<T>) {
    let mut runtimes = runtimes.lock().expect("worker supervisor poisoned");
    runtimes.retain(|entry| entry.strong_count() > 0);
    let weak = Arc::downgrade(runtime);
    if !runtimes.iter().any(|entry| entry.ptr_eq(&weak)) {
        runtimes.push(weak);
    }
}

fn live<T>(runtimes: &Mutex<Vec<Weak<T>>>) -> Vec<Arc<T>> {
    let mut runtimes = runtimes.lock().expect("worker supervisor poisoned");
    let mut live = Vec::with_capacity(runtimes.len());
    runtimes.retain(|entry| match entry.upgrade() {
        Some(runtime) => {
            live.push(runtime);
            true
        }
        None => false,
    });
    live
}

impl Drop for WorkerSupervisor {
    fn drop(&mut self) {
        self.stopping.store(true, Ordering::Release);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}
