use std::{
    collections::HashMap,
    sync::{Arc, Mutex, mpsc},
    thread,
    time::Instant,
};
use tokio::sync::mpsc as session_mpsc;

use crate::{
    audio::PcmF32Mono,
    providers::{VadInput, VadProvider},
};

use super::{WorkerIdentity, WorkerRuntimeConfig};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct VadWorkerLease(u64);

#[derive(Debug)]
pub enum VadCommand {
    Push(PcmF32Mono),
    Reset,
    Close,
}

#[derive(Debug, Clone, PartialEq)]
pub enum VadWorkerEvent {
    Opened {
        identity: WorkerIdentity,
    },
    Probability {
        identity: WorkerIdentity,
        probability: crate::providers::VadProbability,
    },
    SpeechStart {
        identity: WorkerIdentity,
        start_sample: u64,
    },
    SpeechEnd {
        identity: WorkerIdentity,
        end_sample: u64,
    },
    ResetDone {
        identity: WorkerIdentity,
    },
    Closed {
        identity: WorkerIdentity,
    },
    Failed {
        identity: WorkerIdentity,
    },
    ResetTimedOut {
        identity: WorkerIdentity,
    },
    CleanupTimedOut {
        identity: WorkerIdentity,
    },
}

#[derive(Debug, thiserror::Error)]
pub enum VadWorkerError {
    #[error("VAD worker capacity is exhausted")]
    Capacity,
    #[error("VAD worker lease is not active")]
    UnknownLease,
    #[error("VAD worker command queue is full")]
    QueueFull,
}

pub struct VadWorkerRuntime {
    provider: Arc<dyn VadProvider>,
    config: WorkerRuntimeConfig,
    state: Mutex<State>,
    events_rx: Mutex<mpsc::Receiver<VadWorkerEvent>>,
    events_tx: mpsc::Sender<VadWorkerEvent>,
    routes: Mutex<HashMap<String, session_mpsc::Sender<VadWorkerEvent>>>,
}
struct State {
    next_lease: u64,
    slots: HashMap<VadWorkerLease, Slot>,
}
struct Slot {
    identity: WorkerIdentity,
    command_tx: mpsc::SyncSender<VadCommand>,
    state: SlotState,
}
enum SlotState {
    Active,
    Resetting { deadline: Instant },
    Cleaning { deadline: Instant },
    Quarantined,
}

impl VadWorkerRuntime {
    pub fn new(provider: Arc<dyn VadProvider>, config: WorkerRuntimeConfig) -> Self {
        config.validate().expect("invalid worker runtime config");
        let (events_tx, events_rx) = mpsc::channel();
        Self {
            provider,
            config,
            state: Mutex::new(State {
                next_lease: 1,
                slots: HashMap::new(),
            }),
            events_rx: Mutex::new(events_rx),
            events_tx,
            routes: Mutex::new(HashMap::new()),
        }
    }

    pub fn runtime_config(&self) -> WorkerRuntimeConfig {
        self.config.clone()
    }

    /// Registers one actor mailbox for a complete Auto cycle. Worker events are routed by
    /// session identity by the application-owned supervisor, never consumed by actors globally.
    pub fn register_session(&self, session: &str) -> session_mpsc::Receiver<VadWorkerEvent> {
        let (sender, receiver) = session_mpsc::channel(self.config.command_capacity);
        self.routes
            .lock()
            .expect("VAD worker routes poisoned")
            .insert(session.to_owned(), sender);
        receiver
    }

    pub fn unregister_session(&self, session: &str) {
        self.routes
            .lock()
            .expect("VAD worker routes poisoned")
            .remove(session);
    }
    pub fn open(&self, identity: WorkerIdentity) -> Result<VadWorkerLease, VadWorkerError> {
        let mut state = self.state.lock().expect("VAD worker state poisoned");
        if state.slots.len() >= self.config.max_workers {
            return Err(VadWorkerError::Capacity);
        }
        let lease = VadWorkerLease(state.next_lease);
        state.next_lease += 1;
        let (command_tx, command_rx) = mpsc::sync_channel(self.config.command_capacity);
        state.slots.insert(
            lease,
            Slot {
                identity: identity.clone(),
                command_tx,
                state: SlotState::Active,
            },
        );
        let provider = Arc::clone(&self.provider);
        let events = self.events_tx.clone();
        thread::spawn(move || run_worker(provider, identity, command_rx, events));
        Ok(lease)
    }
    pub fn send(&self, lease: VadWorkerLease, command: VadCommand) -> Result<(), VadWorkerError> {
        let mut state = self.state.lock().expect("VAD worker state poisoned");
        let slot = state
            .slots
            .get_mut(&lease)
            .ok_or(VadWorkerError::UnknownLease)?;
        match command {
            VadCommand::Reset => {
                slot.state = SlotState::Resetting {
                    deadline: Instant::now() + self.config.final_timeout,
                }
            }
            VadCommand::Close => {
                slot.state = SlotState::Cleaning {
                    deadline: Instant::now() + self.config.cleanup_grace,
                }
            }
            VadCommand::Push(_) if !matches!(slot.state, SlotState::Active) => {
                return Err(VadWorkerError::UnknownLease);
            }
            VadCommand::Push(_) => {}
        }
        slot.command_tx
            .try_send(command)
            .map_err(|error| match error {
                mpsc::TrySendError::Full(_) => VadWorkerError::QueueFull,
                mpsc::TrySendError::Disconnected(_) => VadWorkerError::UnknownLease,
            })
    }
    pub fn try_recv(&self) -> Option<VadWorkerEvent> {
        let event = self
            .events_rx
            .lock()
            .expect("VAD worker events poisoned")
            .try_recv()
            .ok()?;
        self.observe(&event);
        Some(event)
    }
    pub fn recv_timeout(&self, timeout: std::time::Duration) -> Option<VadWorkerEvent> {
        let event = self
            .events_rx
            .lock()
            .expect("VAD worker events poisoned")
            .recv_timeout(timeout)
            .ok()?;
        self.observe(&event);
        Some(event)
    }
    /// Drives routing and timeout quarantine once. Production uses `WorkerSupervisor`.
    pub fn supervise_pending(&self) {
        while let Ok(event) = self
            .events_rx
            .lock()
            .expect("VAD worker events poisoned")
            .try_recv()
        {
            self.dispatch(event);
        }
        while let Some(event) = self.reap_timeouts() {
            self.dispatch(event);
        }
    }
    pub fn reap_timeouts(&self) -> Option<VadWorkerEvent> {
        let mut state = self.state.lock().expect("VAD worker state poisoned");
        let now = Instant::now();
        for slot in state.slots.values_mut() {
            let event = match &slot.state {
                SlotState::Resetting { deadline } if *deadline <= now => {
                    Some(VadWorkerEvent::ResetTimedOut {
                        identity: slot.identity.clone(),
                    })
                }
                SlotState::Cleaning { deadline } if *deadline <= now => {
                    Some(VadWorkerEvent::CleanupTimedOut {
                        identity: slot.identity.clone(),
                    })
                }
                _ => None,
            };
            if let Some(event) = event {
                slot.state = SlotState::Quarantined;
                return Some(event);
            }
        }
        None
    }
    fn observe(&self, event: &VadWorkerEvent) {
        let identity = match event {
            VadWorkerEvent::Closed { identity } | VadWorkerEvent::Failed { identity } => identity,
            _ => return,
        };
        let mut state = self.state.lock().expect("VAD worker state poisoned");
        let lease = state
            .slots
            .iter()
            .find_map(|(lease, slot)| (slot.identity == *identity).then_some(*lease));
        if let Some(lease) = lease
            && !matches!(
                state.slots.get(&lease).map(|slot| &slot.state),
                Some(SlotState::Quarantined)
            )
        {
            state.slots.remove(&lease);
        }
    }

    fn dispatch(&self, event: VadWorkerEvent) {
        self.observe(&event);
        let session = match &event {
            VadWorkerEvent::Opened { identity }
            | VadWorkerEvent::Probability { identity, .. }
            | VadWorkerEvent::SpeechStart { identity, .. }
            | VadWorkerEvent::SpeechEnd { identity, .. }
            | VadWorkerEvent::ResetDone { identity }
            | VadWorkerEvent::Closed { identity }
            | VadWorkerEvent::Failed { identity }
            | VadWorkerEvent::ResetTimedOut { identity }
            | VadWorkerEvent::CleanupTimedOut { identity } => identity.session(),
        };
        let route = self
            .routes
            .lock()
            .expect("VAD worker routes poisoned")
            .get(session)
            .cloned();
        if let Some(route) = route {
            let _ = route.try_send(event);
        }
    }
}

fn run_worker(
    provider: Arc<dyn VadProvider>,
    identity: WorkerIdentity,
    commands: mpsc::Receiver<VadCommand>,
    events: mpsc::Sender<VadWorkerEvent>,
) {
    let Ok(mut session) = provider.open() else {
        let _ = events.send(VadWorkerEvent::Failed { identity });
        return;
    };
    if events
        .send(VadWorkerEvent::Opened {
            identity: identity.clone(),
        })
        .is_err()
    {
        return;
    }
    let mut rechunker = VadRechunker::default();
    while let Ok(command) = commands.recv() {
        match command {
            VadCommand::Push(pcm) => {
                let inputs = match rechunker.push(pcm) {
                    Ok(inputs) => inputs,
                    Err(()) => {
                        let _ = events.send(VadWorkerEvent::Failed { identity });
                        return;
                    }
                };
                for input in inputs {
                    let start_sample = input.start_sample;
                    let probability = match session.push(input) {
                        Ok(probability) => probability,
                        Err(_) => {
                            let _ = events.send(VadWorkerEvent::Failed { identity });
                            return;
                        }
                    };
                    if probability.start_sample != start_sample
                        || probability.end_sample != start_sample + 512
                    {
                        let _ = events.send(VadWorkerEvent::Failed { identity });
                        return;
                    }
                    if events
                        .send(VadWorkerEvent::Probability {
                            identity: identity.clone(),
                            probability,
                        })
                        .is_err()
                    {
                        return;
                    }
                }
            }
            VadCommand::Reset => match session.reset() {
                Ok(()) => {
                    rechunker.reset();
                    let _ = events.send(VadWorkerEvent::ResetDone {
                        identity: identity.clone(),
                    });
                }
                Err(_) => {
                    let _ = events.send(VadWorkerEvent::Failed { identity });
                    return;
                }
            },
            VadCommand::Close => match session.close() {
                Ok(()) => {
                    let _ = events.send(VadWorkerEvent::Closed { identity });
                    return;
                }
                Err(_) => {
                    let _ = events.send(VadWorkerEvent::Failed { identity });
                    return;
                }
            },
        }
    }
}

/// Keeps the 960-sample transport timeline separate from Silero's 512-sample contract.
#[derive(Default)]
struct VadRechunker {
    pending: Vec<f32>,
    next_sample: u64,
}

impl VadRechunker {
    fn push(&mut self, pcm: PcmF32Mono) -> Result<Vec<VadInput>, ()> {
        if pcm.sample_rate_hz() != 16_000 || pcm.samples().len() != 960 {
            return Err(());
        }
        self.pending.extend_from_slice(pcm.samples());
        let mut frames = Vec::new();
        while self.pending.len() >= 512 {
            let samples = self.pending.drain(..512).collect();
            frames.push(VadInput {
                pcm: samples,
                start_sample: self.next_sample,
            });
            self.next_sample += 512;
        }
        Ok(frames)
    }
    fn reset(&mut self) {
        self.pending.clear();
        self.next_sample = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::VadRechunker;
    use crate::audio::PcmF32Mono;

    #[test]
    fn reset_discards_rechunk_slack_and_restarts_the_sample_timeline() {
        let mut rechunker = VadRechunker::default();
        let frame = PcmF32Mono::new(vec![0.0; 960], 16_000);

        assert_eq!(rechunker.push(frame.clone()).unwrap()[0].start_sample, 0);
        rechunker.reset();
        assert_eq!(rechunker.push(frame).unwrap()[0].start_sample, 0);
    }
}
