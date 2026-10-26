use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread::{self, JoinHandle},
};

type BoxedJob = Box<dyn FnOnce(WorkerBusyGuard) + Send + 'static>;

pub(in crate::workers) struct WorkerBusyGuard(Arc<AtomicBool>);

impl WorkerBusyGuard {
    pub fn finish(self) {
        self.0.store(false, Ordering::Release);
    }
}

struct PooledWorker {
    job_tx: Option<mpsc::SyncSender<BoxedJob>>,
    busy: Arc<AtomicBool>,
    handle: JoinHandle<()>,
}

/// Reusable native worker threads. Handles remain resident after a semantic terminal event until
/// native destructors exit; idle workers stay parked on their bounded job channel for reuse.
/// The owning runtime serializes admission and cleanup under its state mutex.
#[derive(Default)]
pub(in crate::workers) struct NativeThreads {
    closed: bool,
    quarantined: bool,
    workers: Vec<PooledWorker>,
}
impl NativeThreads {
    pub fn can_spawn(&mut self, _capacity: usize) -> bool {
        self.reap();
        // Logical admission and retained pools cap native ownership. A worker may remain busy
        // for a few instructions after it returned native state and sent its terminal event.
        !self.closed && !self.quarantined
    }
    pub fn execute(
        &mut self,
        name: &str,
        job: impl FnOnce(WorkerBusyGuard) + Send + 'static,
    ) -> bool {
        self.reap();
        if self.closed || self.quarantined {
            return false;
        }
        let mut job: Option<BoxedJob> = Some(Box::new(job));
        for worker in &self.workers {
            if !worker.busy.swap(true, Ordering::AcqRel) {
                if let Some(tx) = &worker.job_tx {
                    match tx.try_send(job.take().expect("job present")) {
                        Ok(()) => return true,
                        Err(
                            mpsc::TrySendError::Full(returned)
                            | mpsc::TrySendError::Disconnected(returned),
                        ) => {
                            job = Some(returned);
                            worker.busy.store(false, Ordering::Release);
                        }
                    }
                } else {
                    worker.busy.store(false, Ordering::Release);
                }
            }
        }
        let (job_tx, job_rx) = mpsc::sync_channel::<BoxedJob>(1);
        let busy = Arc::new(AtomicBool::new(true));
        let worker_busy = Arc::clone(&busy);
        let Ok(handle) = thread::Builder::new().name(name.into()).spawn(move || {
            while let Ok(task) = job_rx.recv() {
                task(WorkerBusyGuard(Arc::clone(&worker_busy)));
            }
        }) else {
            return false;
        };
        if job_tx.send(job.expect("job present")).is_err() {
            let _ = handle.join();
            return false;
        }
        self.workers.push(PooledWorker {
            job_tx: Some(job_tx),
            busy,
            handle,
        });
        true
    }
    pub fn is_closed(&self) -> bool {
        self.closed || self.quarantined
    }
    pub fn close(&mut self) -> bool {
        self.closed = true;
        for worker in &mut self.workers {
            worker.job_tx = None;
        }
        self.reap();
        !self.quarantined && self.workers.is_empty()
    }
    pub fn reap(&mut self) {
        let mut index = 0;
        while index < self.workers.len() {
            let finished = self.workers[index].handle.is_finished()
                || (self.closed && !self.workers[index].busy.load(Ordering::Acquire));
            if finished {
                let mut worker = self.workers.swap_remove(index);
                worker.job_tx = None;
                self.quarantined |= worker.handle.join().is_err();
            } else {
                index += 1;
            }
        }
    }
}
