//! Background streams have their own event queue; Voice supervision cannot consume them.
use super::*;
use std::{
    sync::atomic::{AtomicBool, Ordering},
    time::Duration,
};

pub struct AsrBackgroundOperation {
    runtime: AsrWorkerRuntime,
    lease: AsrStreamLease,
    completed: Arc<AtomicBool>,
    cleanup_started: AtomicBool,
}
impl AsrWorkerRuntime {
    pub fn has_background_capacity(&self) -> bool {
        self.admission.has_background_capacity()
    }
    pub fn open_background(&self) -> Result<AsrBackgroundOperation, AsrWorkerError> {
        let mut isolated = self.clone();
        let (tx, rx) = mpsc::channel();
        isolated.events_tx = EventSender {
            tx,
            waker: Default::default(),
        };
        isolated.events_rx = Arc::new(Mutex::new(rx));
        let completed = Arc::new(AtomicBool::new(false));
        let identity =
            WorkerIdentity::new(format!("background-asr:{}", uuid::Uuid::new_v4()), 0, 0);
        let lease = isolated.open_class(
            identity,
            ProviderWorkloadClass::Background,
            Some(completed.clone()),
        )?;
        Ok(AsrBackgroundOperation {
            runtime: isolated,
            lease,
            completed,
            cleanup_started: AtomicBool::new(false),
        })
    }
}
impl AsrBackgroundOperation {
    pub fn send(&self, command: AsrCommand) -> Result<(), AsrWorkerError> {
        self.runtime.send(self.lease, command)
    }
    pub fn poll(&self) -> Option<AsrWorkerEvent> {
        self.runtime.try_recv()
    }
    pub fn acknowledged(&self) -> bool {
        self.completed.load(Ordering::Acquire)
    }
    pub fn cleanup_grace(&self) -> Duration {
        self.runtime.config.cleanup_grace
    }
    /// Call outside the async executor. Quarantine preserves the exact slot's permit.
    pub fn cleanup(&self) -> bool {
        self.cleanup_started.store(true, Ordering::Release);
        let deadline = Instant::now() + self.cleanup_grace();
        while !self.acknowledged() {
            let _ = self.send(AsrCommand::Cancel);
            while self.poll().is_some() {}
            if Instant::now() >= deadline {
                if let Some(slot) = self
                    .runtime
                    .state
                    .lock()
                    .expect("ASR state poisoned")
                    .slots
                    .get_mut(&self.lease)
                {
                    slot.state = SlotState::Quarantined;
                }
                return false;
            }
            thread::sleep(Duration::from_millis(5));
        }
        while self.poll().is_some() {}
        true
    }
}
impl Drop for AsrBackgroundOperation {
    fn drop(&mut self) {
        if self.acknowledged() {
            while self.poll().is_some() {}
        }
        if !self.acknowledged() && !self.cleanup_started.swap(true, Ordering::AcqRel) {
            let runtime = self.runtime.clone();
            let lease = self.lease;
            let completed = self.completed.clone();
            thread::spawn(move || {
                let operation = AsrBackgroundOperation {
                    runtime,
                    lease,
                    completed,
                    cleanup_started: AtomicBool::new(true),
                };
                operation.cleanup();
            });
        }
    }
}

#[cfg(test)]
#[path = "background_tests.rs"]
mod tests;
