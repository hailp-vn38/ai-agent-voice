use std::{
    collections::HashMap,
    sync::{mpsc, Arc, Mutex},
    thread,
    time::Instant,
};

use crate::{
    audio::PcmF32Mono,
    providers::{VadEvent, VadProvider},
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VadWorkerEvent {
    Opened { identity: WorkerIdentity },
    SpeechStart { identity: WorkerIdentity },
    SpeechEnd { identity: WorkerIdentity },
    ResetDone { identity: WorkerIdentity },
    Closed { identity: WorkerIdentity },
    Failed { identity: WorkerIdentity },
    ResetTimedOut { identity: WorkerIdentity },
    CleanupTimedOut { identity: WorkerIdentity },
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
        }
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
                return Err(VadWorkerError::UnknownLease)
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
        if let Some(lease) = lease {
            if !matches!(
                state.slots.get(&lease).map(|slot| &slot.state),
                Some(SlotState::Quarantined)
            ) {
                state.slots.remove(&lease);
            }
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
    while let Ok(command) = commands.recv() {
        match command {
            VadCommand::Push(pcm) => match session.push_pcm(&pcm) {
                Ok(provider_events) => {
                    for event in provider_events {
                        let event = match event {
                            VadEvent::SpeechStart => VadWorkerEvent::SpeechStart {
                                identity: identity.clone(),
                            },
                            VadEvent::SpeechEnd => VadWorkerEvent::SpeechEnd {
                                identity: identity.clone(),
                            },
                        };
                        if events.send(event).is_err() {
                            return;
                        }
                    }
                }
                Err(_) => {
                    let _ = events.send(VadWorkerEvent::Failed { identity });
                    return;
                }
            },
            VadCommand::Reset => match session.reset() {
                Ok(()) => {
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
