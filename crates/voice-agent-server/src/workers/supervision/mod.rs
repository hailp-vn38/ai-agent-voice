use std::{
    sync::{
        Arc, Condvar, Mutex, Weak,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant},
};

mod native_threads;

pub(in crate::workers) use native_threads::NativeThreads;

use super::{AsrWorkerRuntime, VadWorkerRuntime};

#[derive(Clone)]
pub(in crate::workers) struct EventSender<T> {
    pub(in crate::workers) tx: mpsc::Sender<T>,
    pub(in crate::workers) waker: RuntimeWaker,
}

impl<T> EventSender<T> {
    pub(in crate::workers) fn send(&self, event: T) -> Result<(), mpsc::SendError<T>> {
        let result = self.tx.send(event);
        if result.is_ok() {
            self.waker.wake();
        }
        result
    }
}

#[derive(Default)]
pub(in crate::workers) struct SupervisorWaker {
    pending: Mutex<bool>,
    condvar: Condvar,
}

impl SupervisorWaker {
    pub(in crate::workers) fn wake(&self) {
        let mut pending = self
            .pending
            .lock()
            .expect("worker supervisor waker poisoned");
        *pending = true;
        self.condvar.notify_one();
    }

    fn wait_timeout(&self, timeout: Option<Duration>) {
        let mut pending = self
            .pending
            .lock()
            .expect("worker supervisor waker poisoned");
        if !*pending {
            match timeout {
                Some(duration) if !duration.is_zero() => {
                    let (guard, _) = self
                        .condvar
                        .wait_timeout(pending, duration)
                        .expect("worker supervisor waker poisoned");
                    pending = guard;
                }
                Some(_) => {}
                None => {
                    pending = self
                        .condvar
                        .wait(pending)
                        .expect("worker supervisor waker poisoned");
                }
            }
        }
        *pending = false;
    }
}

#[derive(Clone, Default)]
pub(in crate::workers) struct RuntimeWaker {
    listeners: Arc<Mutex<Vec<Weak<SupervisorWaker>>>>,
}

impl RuntimeWaker {
    pub(in crate::workers) fn register(&self, waker: &Arc<SupervisorWaker>) {
        let mut listeners = self.listeners.lock().expect("runtime waker poisoned");
        listeners.retain(|entry| entry.strong_count() > 0);
        let weak = Arc::downgrade(waker);
        if !listeners.iter().any(|entry| entry.ptr_eq(&weak)) {
            listeners.push(weak);
        }
    }

    pub(in crate::workers) fn wake(&self) {
        let mut listeners = self.listeners.lock().expect("runtime waker poisoned");
        listeners.retain(|entry| match entry.upgrade() {
            Some(waker) => {
                waker.wake();
                true
            }
            None => false,
        });
    }
}

/// Sole production driver for worker event routing and timeout quarantine.
/// Weak observations allow the runtime manager to own retention and unload.
/// The resource remains owned by its manager while native cleanup is pending.
pub struct WorkerSupervisor {
    stopping: Arc<AtomicBool>,
    waker: Arc<SupervisorWaker>,
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
        let waker = Arc::new(SupervisorWaker::default());
        for runtime in &asr {
            runtime.attach_supervisor(&waker);
        }
        for runtime in &vad {
            runtime.attach_supervisor(&waker);
        }
        waker.wake();
        let thread_stopping = Arc::clone(&stopping);
        let thread_waker = Arc::clone(&waker);
        let asr = Arc::new(Mutex::new(asr.iter().map(Arc::downgrade).collect()));
        let vad = Arc::new(Mutex::new(vad.iter().map(Arc::downgrade).collect()));
        let thread_asr = Arc::clone(&asr);
        let thread_vad = Arc::clone(&vad);
        let thread = thread::spawn(move || {
            while !thread_stopping.load(Ordering::Acquire) {
                let mut next_deadline: Option<Instant> = None;
                for runtime in live(&thread_asr) {
                    runtime.supervise_pending();
                    if let Some(deadline) = runtime.next_timeout_deadline() {
                        next_deadline =
                            Some(next_deadline.map_or(deadline, |current| current.min(deadline)));
                    }
                }
                for runtime in live(&thread_vad) {
                    runtime.supervise_pending();
                    if let Some(deadline) = runtime.next_timeout_deadline() {
                        next_deadline =
                            Some(next_deadline.map_or(deadline, |current| current.min(deadline)));
                    }
                }
                if thread_stopping.load(Ordering::Acquire) {
                    break;
                }
                let timeout = next_deadline
                    .map(|deadline| deadline.saturating_duration_since(Instant::now()));
                thread_waker.wait_timeout(timeout);
            }
        });
        Self {
            stopping,
            waker,
            asr,
            vad,
            thread: Some(thread),
        }
    }

    pub fn observe_asr(&self, runtime: Arc<AsrWorkerRuntime>) {
        runtime.attach_supervisor(&self.waker);
        observe(&self.asr, &runtime);
        self.waker.wake();
    }
    pub fn observe_vad(&self, runtime: Arc<VadWorkerRuntime>) {
        runtime.attach_supervisor(&self.waker);
        observe(&self.vad, &runtime);
        self.waker.wake();
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
        self.waker.wake();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}
