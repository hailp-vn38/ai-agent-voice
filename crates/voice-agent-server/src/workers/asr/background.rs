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

    /// Transcribes a canonical 16 kHz mono PCM buffer on the Background workload lane
    /// (`ProviderWorkloadClass::Background`), framing into 100 ms chunks, handling
    /// `QueueFull` backpressure, and guaranteeing synchronous `cleanup()` inside `spawn_blocking`.
    pub async fn transcribe_background_pcm(
        &self,
        pcm: PcmF32Mono,
    ) -> Result<String, AsrWorkerError> {
        const FRAME_SAMPLES: usize = 1_600; // 100 ms @ 16 kHz
        let runtime = self.clone();
        tokio::task::spawn_blocking(move || {
            let op = runtime.open_background()?;
            let push_deadline =
                Instant::now() + runtime.config.final_timeout + Duration::from_secs(5);

            for chunk in pcm.samples().chunks(FRAME_SAMPLES) {
                let mut frame = Some(PcmF32Mono::from_samples(chunk.to_vec()));
                loop {
                    while let Some(ev) = op.poll() {
                        match ev {
                            AsrWorkerEvent::Failed { .. }
                            | AsrWorkerEvent::Cancelled { .. }
                            | AsrWorkerEvent::FinalTimedOut { .. }
                            | AsrWorkerEvent::CleanupTimedOut { .. } => {
                                let _ = op.cleanup();
                                return Err(AsrWorkerError::Initialization);
                            }
                            _ => {}
                        }
                    }
                    let next_frame = frame.take().expect("frame ready for push");
                    match op.send(AsrCommand::Push(next_frame.clone())) {
                        Ok(()) => break,
                        Err(AsrWorkerError::QueueFull) => {
                            if Instant::now() >= push_deadline {
                                let _ = op.cleanup();
                                return Err(AsrWorkerError::QueueFull);
                            }
                            frame = Some(next_frame);
                            thread::sleep(Duration::from_millis(2));
                        }
                        Err(e) => {
                            let _ = op.cleanup();
                            return Err(e);
                        }
                    }
                }
            }

            loop {
                match op.send(AsrCommand::Finish) {
                    Ok(()) => break,
                    Err(AsrWorkerError::QueueFull) => {
                        if Instant::now() >= push_deadline {
                            let _ = op.cleanup();
                            return Err(AsrWorkerError::QueueFull);
                        }
                        while let Some(ev) = op.poll() {
                            if matches!(
                                ev,
                                AsrWorkerEvent::Failed { .. }
                                    | AsrWorkerEvent::Cancelled { .. }
                                    | AsrWorkerEvent::FinalTimedOut { .. }
                                    | AsrWorkerEvent::CleanupTimedOut { .. }
                            ) {
                                let _ = op.cleanup();
                                return Err(AsrWorkerError::Initialization);
                            }
                        }
                        thread::sleep(Duration::from_millis(2));
                    }
                    Err(e) => {
                        let _ = op.cleanup();
                        return Err(e);
                    }
                }
            }

            let final_deadline = Instant::now()
                + runtime.config.final_timeout
                + runtime.config.cleanup_grace
                + Duration::from_secs(1);

            loop {
                match op.poll() {
                    Some(AsrWorkerEvent::Final { text, .. }) => {
                        let _ = op.cleanup();
                        return Ok(text);
                    }
                    Some(
                        AsrWorkerEvent::Failed { .. }
                        | AsrWorkerEvent::Cancelled { .. }
                        | AsrWorkerEvent::FinalTimedOut { .. }
                        | AsrWorkerEvent::CleanupTimedOut { .. },
                    ) => {
                        let _ = op.cleanup();
                        return Err(AsrWorkerError::Initialization);
                    }
                    Some(AsrWorkerEvent::Opened { .. }) => {}
                    None => {
                        if Instant::now() >= final_deadline {
                            let _ = op.cleanup();
                            return Err(AsrWorkerError::Capacity);
                        }
                        thread::sleep(Duration::from_millis(2));
                    }
                }
            }
        })
        .await
        .map_err(|_| AsrWorkerError::Initialization)?
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
