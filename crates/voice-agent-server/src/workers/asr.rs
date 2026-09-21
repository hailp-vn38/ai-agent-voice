use std::{
    collections::HashMap,
    sync::{mpsc, Arc, Mutex},
    thread,
    time::Instant,
};
use tokio::sync::mpsc as session_mpsc;

use crate::{audio::PcmF32Mono, providers::AsrProvider};

use super::{WorkerIdentity, WorkerRuntimeConfig};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct AsrStreamLease(u64);

#[derive(Debug)]
pub enum AsrCommand {
    Push(PcmF32Mono),
    Finish,
    Cancel,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AsrWorkerEvent {
    Opened {
        identity: WorkerIdentity,
    },
    Final {
        identity: WorkerIdentity,
        text: String,
    },
    Failed {
        identity: WorkerIdentity,
    },
    Cancelled {
        identity: WorkerIdentity,
    },
    FinalTimedOut {
        identity: WorkerIdentity,
    },
    CleanupTimedOut {
        identity: WorkerIdentity,
    },
}

#[derive(Debug, thiserror::Error)]
pub enum AsrWorkerError {
    #[error("worker runtime configuration is invalid: {0}")]
    InvalidConfig(&'static str),
    #[error("ASR worker capacity is exhausted")]
    Capacity,
    #[error("ASR stream lease is not active")]
    UnknownLease,
    #[error("ASR worker command queue is full")]
    QueueFull,
}

pub struct AsrWorkerRuntime {
    provider: Arc<dyn AsrProvider>,
    config: WorkerRuntimeConfig,
    state: Mutex<State>,
    events_rx: Mutex<mpsc::Receiver<AsrWorkerEvent>>,
    events_tx: mpsc::Sender<AsrWorkerEvent>,
    routes: Mutex<HashMap<String, session_mpsc::Sender<AsrWorkerEvent>>>,
}

struct State {
    next_lease: u64,
    slots: HashMap<AsrStreamLease, Slot>,
}

struct Slot {
    identity: WorkerIdentity,
    command_tx: mpsc::SyncSender<AsrCommand>,
    state: SlotState,
}

enum SlotState {
    Active,
    Finishing { deadline: Instant },
    Cleaning { deadline: Instant },
    Quarantined,
}

impl AsrWorkerRuntime {
    pub fn new(provider: Arc<dyn AsrProvider>, config: WorkerRuntimeConfig) -> Self {
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

    /// Registers exactly one actor mailbox for this Voice Session. The runtime is the sole
    /// consumer of worker events and routes them by immutable session identity.
    pub fn register_session(&self, session: &str) -> session_mpsc::Receiver<AsrWorkerEvent> {
        let (sender, receiver) = session_mpsc::channel(self.config.command_capacity);
        self.routes
            .lock()
            .expect("ASR worker routes poisoned")
            .insert(session.to_owned(), sender);
        receiver
    }

    pub fn unregister_session(&self, session: &str) {
        self.routes
            .lock()
            .expect("ASR worker routes poisoned")
            .remove(session);
    }

    pub fn open(&self, identity: WorkerIdentity) -> Result<AsrStreamLease, AsrWorkerError> {
        let mut state = self.state.lock().expect("ASR worker state poisoned");
        if state.slots.len() >= self.config.max_workers {
            return Err(AsrWorkerError::Capacity);
        }
        let lease = AsrStreamLease(state.next_lease);
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
        let events_tx = self.events_tx.clone();
        thread::spawn(move || run_worker(provider, identity, command_rx, events_tx));
        Ok(lease)
    }

    pub fn send(&self, lease: AsrStreamLease, command: AsrCommand) -> Result<(), AsrWorkerError> {
        let mut state = self.state.lock().expect("ASR worker state poisoned");
        let slot = state
            .slots
            .get_mut(&lease)
            .ok_or(AsrWorkerError::UnknownLease)?;
        match command {
            AsrCommand::Finish => {
                slot.state = SlotState::Finishing {
                    deadline: Instant::now() + self.config.final_timeout,
                }
            }
            AsrCommand::Cancel => {
                slot.state = SlotState::Cleaning {
                    deadline: Instant::now() + self.config.cleanup_grace,
                }
            }
            AsrCommand::Push(_) => {
                if !matches!(slot.state, SlotState::Active) {
                    return Err(AsrWorkerError::UnknownLease);
                }
            }
        }
        slot.command_tx
            .try_send(command)
            .map_err(|error| match error {
                mpsc::TrySendError::Full(_) => AsrWorkerError::QueueFull,
                mpsc::TrySendError::Disconnected(_) => AsrWorkerError::UnknownLease,
            })
    }

    pub fn recv_timeout(&self, timeout: std::time::Duration) -> Option<AsrWorkerEvent> {
        let event = self
            .events_rx
            .lock()
            .expect("ASR worker events poisoned")
            .recv_timeout(timeout)
            .ok()?;
        self.observe(&event);
        Some(event)
    }

    pub fn try_recv(&self) -> Option<AsrWorkerEvent> {
        let event = self
            .events_rx
            .lock()
            .expect("ASR worker events poisoned")
            .try_recv()
            .ok()?;
        self.observe(&event);
        Some(event)
    }

    /// Drives the application-owned event router once. Production calls this from
    /// `WorkerSupervisor`; direct actor tests may call it through `pump_workers`.
    pub fn supervise_pending(&self) {
        while let Ok(event) = self
            .events_rx
            .lock()
            .expect("ASR worker events poisoned")
            .try_recv()
        {
            self.dispatch(event);
        }
        while let Some(event) = self.reap_timeouts() {
            self.dispatch(event);
        }
    }

    pub fn reap_timeouts(&self) -> Option<AsrWorkerEvent> {
        let mut state = self.state.lock().expect("ASR worker state poisoned");
        let now = Instant::now();
        for slot in state.slots.values_mut() {
            let event = match &slot.state {
                SlotState::Finishing { deadline } if *deadline <= now => {
                    Some(AsrWorkerEvent::FinalTimedOut {
                        identity: slot.identity.clone(),
                    })
                }
                SlotState::Cleaning { deadline } if *deadline <= now => {
                    Some(AsrWorkerEvent::CleanupTimedOut {
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

    fn observe(&self, event: &AsrWorkerEvent) {
        let terminal = matches!(
            event,
            AsrWorkerEvent::Final { .. }
                | AsrWorkerEvent::Failed { .. }
                | AsrWorkerEvent::Cancelled { .. }
        );
        if !terminal {
            return;
        }
        let identity = match event {
            AsrWorkerEvent::Final { identity, .. }
            | AsrWorkerEvent::Failed { identity }
            | AsrWorkerEvent::Cancelled { identity } => identity,
            _ => return,
        };
        let mut state = self.state.lock().expect("ASR worker state poisoned");
        let lease = state
            .slots
            .iter()
            .find_map(|(lease, slot)| (slot.identity == *identity).then_some(*lease));
        if let Some(lease) = lease {
            if matches!(
                state.slots.get(&lease).map(|slot| &slot.state),
                Some(SlotState::Quarantined)
            ) {
                return;
            }
            state.slots.remove(&lease);
        }
    }

    fn dispatch(&self, event: AsrWorkerEvent) {
        self.observe(&event);
        let session = match &event {
            AsrWorkerEvent::Opened { identity }
            | AsrWorkerEvent::Final { identity, .. }
            | AsrWorkerEvent::Failed { identity }
            | AsrWorkerEvent::Cancelled { identity }
            | AsrWorkerEvent::FinalTimedOut { identity }
            | AsrWorkerEvent::CleanupTimedOut { identity } => identity.session(),
        };
        let route = self
            .routes
            .lock()
            .expect("ASR worker routes poisoned")
            .get(session)
            .cloned();
        if let Some(route) = route {
            let _ = route.try_send(event);
        }
    }
}

fn run_worker(
    provider: Arc<dyn AsrProvider>,
    identity: WorkerIdentity,
    commands: mpsc::Receiver<AsrCommand>,
    events: mpsc::Sender<AsrWorkerEvent>,
) {
    let Ok(mut session) = provider.open() else {
        let _ = events.send(AsrWorkerEvent::Failed { identity });
        return;
    };
    if events
        .send(AsrWorkerEvent::Opened {
            identity: identity.clone(),
        })
        .is_err()
    {
        return;
    }
    while let Ok(command) = commands.recv() {
        match command {
            AsrCommand::Push(pcm) => match session.push_pcm(&pcm) {
                Ok(events_from_provider) => {
                    let _ = events_from_provider;
                }
                Err(_) => {
                    let _ = events.send(AsrWorkerEvent::Failed { identity });
                    return;
                }
            },
            AsrCommand::Finish => match session.finish() {
                Ok(result) => {
                    let _ = events.send(AsrWorkerEvent::Final {
                        identity,
                        text: result.text().to_owned(),
                    });
                    return;
                }
                Err(_) => {
                    let _ = events.send(AsrWorkerEvent::Failed { identity });
                    return;
                }
            },
            AsrCommand::Cancel => {
                session.cancel();
                let _ = events.send(AsrWorkerEvent::Cancelled { identity });
                return;
            }
        }
    }
}
