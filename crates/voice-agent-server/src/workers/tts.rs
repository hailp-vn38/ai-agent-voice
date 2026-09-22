use std::{
    collections::HashMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread,
    time::Instant,
};

use crate::{audio::PcmF32Mono, providers::TtsProvider};

use super::WorkerRuntimeConfig;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct TtsLease(u64);

#[derive(Debug)]
pub enum TtsWorkerEvent {
    Pcm(PcmF32Mono),
    Finished,
    Failed,
    Cancelled,
    TimedOut,
    CleanupTimedOut,
}

impl TtsWorkerEvent {
    fn is_terminal(&self) -> bool {
        matches!(self, Self::Finished | Self::Failed | Self::Cancelled)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum TtsWorkerError {
    #[error("TTS worker capacity is exhausted")]
    Capacity,
    #[error("TTS worker lease is not active")]
    UnknownLease,
}

/// Application-owned boundary for native TTS operation state. Native threads only publish
/// events; this runtime consumes terminal acknowledgement before changing slot capacity.
pub struct TtsWorkerRuntime {
    provider: Arc<dyn TtsProvider>,
    config: WorkerRuntimeConfig,
    state: Arc<Mutex<State>>,
}

struct State {
    next: u64,
    slots: HashMap<TtsLease, Slot>,
}

struct Slot {
    cancelled: Arc<AtomicBool>,
    events: Arc<Mutex<mpsc::Receiver<TtsWorkerEvent>>>,
    deadline: Instant,
    cleanup_deadline: Option<Instant>,
    quarantined: bool,
    cleanup_reported: bool,
}

impl TtsWorkerRuntime {
    pub fn new(provider: Arc<dyn TtsProvider>, config: WorkerRuntimeConfig) -> Self {
        config
            .validate()
            .expect("invalid TTS worker runtime config");
        Self {
            provider,
            config,
            state: Arc::new(Mutex::new(State {
                next: 1,
                slots: HashMap::new(),
            })),
        }
    }

    /// Accepts work against native structural capacity; no second admission semaphore exists.
    pub fn start(&self, text: String) -> Result<TtsLease, TtsWorkerError> {
        let mut state = self.state.lock().expect("TTS worker state poisoned");
        if state.slots.len() >= self.config.max_workers {
            return Err(TtsWorkerError::Capacity);
        }
        let lease = TtsLease(state.next);
        state.next += 1;
        let cancelled = Arc::new(AtomicBool::new(false));
        let (events_tx, events_rx) = mpsc::sync_channel(self.config.command_capacity);
        state.slots.insert(
            lease,
            Slot {
                cancelled: Arc::clone(&cancelled),
                events: Arc::new(Mutex::new(events_rx)),
                deadline: Instant::now() + self.config.final_timeout,
                cleanup_deadline: None,
                quarantined: false,
                cleanup_reported: false,
            },
        );
        let provider = Arc::clone(&self.provider);
        thread::spawn(move || {
            // Native inference intentionally does not run on the Tokio executor.
            let result = provider.synthesize(&text);
            let event = if cancelled.load(Ordering::Acquire) {
                TtsWorkerEvent::Cancelled
            } else {
                match result {
                    Ok(pcm) => {
                        if events_tx.send(TtsWorkerEvent::Pcm(pcm)).is_err() {
                            return;
                        }
                        TtsWorkerEvent::Finished
                    }
                    Err(_) => TtsWorkerEvent::Failed,
                }
            };
            let _ = events_tx.send(event);
        });
        Ok(lease)
    }

    /// Runtime-owned event ingress. Receiving terminal acknowledgement is the sole path that
    /// releases a slot; SpeechOutput observes the returned event but cannot release capacity.
    pub fn poll(&self, lease: TtsLease) -> Result<Option<TtsWorkerEvent>, TtsWorkerError> {
        let mut state = self.state.lock().expect("TTS worker state poisoned");
        let Some(slot) = state.slots.get_mut(&lease) else {
            return Err(TtsWorkerError::UnknownLease);
        };
        if slot.quarantined {
            return Ok(None);
        }
        if slot
            .cleanup_deadline
            .is_some_and(|deadline| Instant::now() >= deadline)
            && !slot.cleanup_reported
        {
            slot.quarantined = true;
            slot.cleanup_reported = true;
            return Ok(Some(TtsWorkerEvent::CleanupTimedOut));
        }
        if Instant::now() >= slot.deadline && slot.cleanup_deadline.is_none() {
            slot.cancelled.store(true, Ordering::Release);
            slot.cleanup_deadline = Some(Instant::now() + self.config.cleanup_grace);
            return Ok(Some(TtsWorkerEvent::TimedOut));
        }
        let event = match slot
            .events
            .lock()
            .expect("TTS worker event receiver poisoned")
            .try_recv()
        {
            Ok(event) => event,
            Err(mpsc::TryRecvError::Empty) => return Ok(None),
            Err(mpsc::TryRecvError::Disconnected) => TtsWorkerEvent::Failed,
        };
        if event.is_terminal() {
            state.slots.remove(&lease);
        }
        Ok(Some(event))
    }

    pub fn cancel(&self, lease: TtsLease) -> Result<(), TtsWorkerError> {
        let mut state = self.state.lock().expect("TTS worker state poisoned");
        let slot = state
            .slots
            .get_mut(&lease)
            .ok_or(TtsWorkerError::UnknownLease)?;
        slot.cancelled.store(true, Ordering::Release);
        slot.cleanup_deadline = Some(Instant::now() + self.config.cleanup_grace);
        Ok(())
    }

    /// Keeps cleanup independent from a Voice Session that has already stopped observing a
    /// lease. The runtime waits for native acknowledgement or quarantines the occupied slot.
    pub fn cancel_and_detach(&self, lease: TtsLease) -> Result<(), TtsWorkerError> {
        self.cancel(lease)?;
        let state = Arc::clone(&self.state);
        let cleanup_grace = self.config.cleanup_grace;
        thread::spawn(move || {
            let events = {
                let state = state.lock().expect("TTS worker state poisoned");
                state.slots.get(&lease).map(|slot| Arc::clone(&slot.events))
            };
            let Some(events) = events else { return };
            let deadline = Instant::now() + cleanup_grace;
            let mut terminal = false;
            while let Some(remaining) = deadline.checked_duration_since(Instant::now()) {
                match events
                    .lock()
                    .expect("TTS worker event receiver poisoned")
                    .recv_timeout(remaining)
                {
                    Ok(event) if event.is_terminal() => {
                        terminal = true;
                        break;
                    }
                    Ok(_) => {}
                    Err(mpsc::RecvTimeoutError::Timeout | mpsc::RecvTimeoutError::Disconnected) => {
                        break;
                    }
                }
            }
            let mut state = state.lock().expect("TTS worker state poisoned");
            if terminal {
                state.slots.remove(&lease);
            } else if let Some(slot) = state.slots.get_mut(&lease) {
                slot.quarantined = true;
                slot.cleanup_reported = true;
            }
        });
        Ok(())
    }

    pub fn active_leases(&self) -> usize {
        self.state
            .lock()
            .expect("TTS worker state poisoned")
            .slots
            .len()
    }
}
