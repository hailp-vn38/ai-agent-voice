use crate::{audio::PcmF32Mono, providers::AsrProvider};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex, mpsc},
    thread,
    time::Instant,
};
use tokio::sync::mpsc as session_mpsc;

use super::{
    ProviderAdmissionError, ProviderCapacityPermit, ProviderRuntimeAdmission,
    ProviderWorkloadClass, WorkerIdentity, WorkerRuntimeConfig,
};

mod diagnostic;
mod pool;

pub use diagnostic::AsrDiagnosticOperation;

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
    #[error("ASR worker initialization failed")]
    Initialization,
    #[error("ASR worker capacity is exhausted")]
    Capacity,
    #[error("ASR stream lease is not active")]
    UnknownLease,
    #[error("ASR worker command queue is full")]
    QueueFull,
}

#[derive(Clone)]
pub struct AsrWorkerRuntime {
    provider: Arc<dyn AsrProvider>,
    config: WorkerRuntimeConfig,
    state: Arc<Mutex<State>>,
    events_rx: Arc<Mutex<mpsc::Receiver<AsrWorkerEvent>>>,
    events_tx: mpsc::Sender<AsrWorkerEvent>,
    routes: Arc<Mutex<HashMap<String, session_mpsc::Sender<AsrWorkerEvent>>>>,
    admission: ProviderRuntimeAdmission,
    pool: Option<Arc<pool::AsrSessionPool>>,
    unload: Arc<Mutex<()>>,
}

struct State {
    threads: super::supervision::NativeThreads,
    next_lease: u64,
    slots: HashMap<AsrStreamLease, Slot>,
}

struct Slot {
    identity: WorkerIdentity,
    command_tx: mpsc::SyncSender<AsrCommand>,
    state: SlotState,
    _permit: Arc<ProviderCapacityPermit>,
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
        let admission =
            ProviderRuntimeAdmission::new(config.max_workers, config.voice_reserved_capacity);
        Self::new_with_admission(provider, config, admission)
    }

    pub(crate) fn new_with_admission(
        provider: Arc<dyn AsrProvider>,
        config: WorkerRuntimeConfig,
        admission: ProviderRuntimeAdmission,
    ) -> Self {
        let (events_tx, events_rx) = mpsc::channel();
        Self {
            provider,
            config,
            state: Arc::new(Mutex::new(State {
                next_lease: 1,
                threads: Default::default(),
                slots: HashMap::new(),
            })),
            events_rx: Arc::new(Mutex::new(events_rx)),
            events_tx,
            routes: Arc::new(Mutex::new(HashMap::new())),
            admission,
            pool: None,
            unload: Arc::new(Mutex::new(())),
        }
    }

    pub fn try_new(
        provider: Arc<dyn AsrProvider>,
        config: WorkerRuntimeConfig,
    ) -> Result<Self, AsrWorkerError> {
        config
            .validate()
            .map_err(|_| AsrWorkerError::Initialization)?;
        let admission =
            ProviderRuntimeAdmission::new(config.max_workers, config.voice_reserved_capacity);
        Self::try_new_with_admission(provider, config, admission)
    }
    pub(crate) fn try_new_with_admission(
        provider: Arc<dyn AsrProvider>,
        config: WorkerRuntimeConfig,
        admission: ProviderRuntimeAdmission,
    ) -> Result<Self, AsrWorkerError> {
        config
            .validate()
            .map_err(|_| AsrWorkerError::Initialization)?;
        let pool = pool::AsrSessionPool::initialize(provider.as_ref(), config.max_workers)
            .map_err(|_| AsrWorkerError::Initialization)?;
        let mut runtime = Self::new_with_admission(provider, config, admission);
        runtime.pool = Some(pool);
        Ok(runtime)
    }
    /// Closes admission. A terminal event alone cannot acknowledge native destruction.
    pub fn shutdown_acknowledged(&self) -> bool {
        let _unload = self.unload.lock().expect("ASR unload poisoned");
        let acknowledged = {
            let mut state = self.state.lock().expect("ASR worker state poisoned");
            state.threads.close() && state.slots.is_empty() && self.admission.active_work() == 0
        };
        if acknowledged && let Some(pool) = &self.pool {
            pool.shutdown();
        }
        acknowledged
    }

    pub(crate) fn readiness(&self) -> super::NativeReadiness {
        self.pool
            .as_ref()
            .map(|pool| pool.readiness())
            .unwrap_or_default()
    }
    pub(crate) fn health_flags(&self) -> Vec<Arc<std::sync::atomic::AtomicBool>> {
        self.pool.iter().map(|pool| pool.health_flag()).collect()
    }

    pub(crate) fn logical_view(&self, quota: ProviderRuntimeAdmission) -> Self {
        let mut view = self.clone();
        view.admission = quota.composed(&self.admission);
        view
    }
    pub fn runtime_config(&self) -> WorkerRuntimeConfig {
        self.config.clone()
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
        let permit = self
            .admission
            .try_admit(ProviderWorkloadClass::Voice)
            .map_err(|_| AsrWorkerError::Capacity)?;
        let mut state = self.state.lock().expect("ASR worker state poisoned");
        if state.slots.len() >= self.config.max_workers
            || !state.threads.can_spawn(self.config.max_workers)
        {
            return Err(AsrWorkerError::Capacity);
        }
        let retained = match &self.pool {
            Some(pool) => Some(pool.take().ok_or(AsrWorkerError::Capacity)?),
            None => None,
        };
        let permit = Arc::new(permit);
        let thread_permit = Arc::clone(&permit);
        let lease = AsrStreamLease(state.next_lease);
        state.next_lease += 1;
        let (command_tx, command_rx) = mpsc::sync_channel(self.config.command_capacity);
        state.slots.insert(
            lease,
            Slot {
                identity: identity.clone(),
                command_tx,
                state: SlotState::Active,
                _permit: permit,
            },
        );
        let provider = Arc::clone(&self.provider);
        let events_tx = self.events_tx.clone();
        let spawn = thread::Builder::new()
            .name("asr-native".into())
            .spawn(move || {
                let terminal =
                    run_worker(provider, retained, identity, command_rx, events_tx.clone());
                // Deferred acknowledgements are published only once the native worker has
                // released its admission; `Cancel` may immediately be followed by `open`.
                drop(thread_permit);
                if let Some(event) = terminal {
                    let _ = events_tx.send(event);
                }
            });
        match spawn {
            Ok(handle) => state.threads.retain(handle),
            Err(_) => {
                state.slots.remove(&lease);
                return Err(AsrWorkerError::Capacity);
            }
        }
        Ok(lease)
    }

    /// Acquires capacity for a bounded diagnostic operation.
    pub fn admit_diagnostic(&self) -> Result<ProviderCapacityPermit, ProviderAdmissionError> {
        let state = self.state.lock().expect("ASR worker state poisoned");
        if state.threads.is_closed() {
            return Err(ProviderAdmissionError::Capacity);
        }
        self.admission.try_admit(ProviderWorkloadClass::Diagnostic)
    }

    /// Builds a standalone diagnostic operation without loading or reconfiguring the provider.
    /// Admission remains owned by `ProviderDiagnosticService`.
    pub fn diagnostic(&self, pcm: PcmF32Mono) -> AsrDiagnosticOperation {
        let provider: Arc<dyn AsrProvider> = match &self.pool {
            Some(pool) => Arc::new(Arc::clone(pool)),
            None => Arc::clone(&self.provider),
        };
        AsrDiagnosticOperation::new(provider, pcm)
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
    retained: Option<Box<dyn crate::providers::AsrSession>>,
    identity: WorkerIdentity,
    commands: mpsc::Receiver<AsrCommand>,
    events: mpsc::Sender<AsrWorkerEvent>,
) -> Option<AsrWorkerEvent> {
    let retained_worker = retained.is_some();
    let Ok(mut session) = retained.map(Ok).unwrap_or_else(|| provider.open()) else {
        return publish_terminal(
            &events,
            AsrWorkerEvent::Failed { identity },
            retained_worker,
        );
    };
    if events
        .send(AsrWorkerEvent::Opened {
            identity: identity.clone(),
        })
        .is_err()
    {
        return None;
    }
    while let Ok(command) = commands.recv() {
        match command {
            AsrCommand::Push(pcm) => match session.push_pcm(&pcm) {
                Ok(events_from_provider) => {
                    let _ = events_from_provider;
                }
                Err(_) => {
                    return publish_terminal(
                        &events,
                        AsrWorkerEvent::Failed { identity },
                        retained_worker,
                    );
                }
            },
            AsrCommand::Finish => match session.finish() {
                Ok(result) => {
                    return publish_terminal(
                        &events,
                        AsrWorkerEvent::Final {
                            identity,
                            text: result.text().to_owned(),
                        },
                        retained_worker,
                    );
                }
                Err(_) => {
                    return publish_terminal(
                        &events,
                        AsrWorkerEvent::Failed { identity },
                        retained_worker,
                    );
                }
            },
            AsrCommand::Cancel => {
                session.cancel();
                return Some(AsrWorkerEvent::Cancelled { identity });
            }
        }
    }
    None
}

fn publish_terminal(
    events: &mpsc::Sender<AsrWorkerEvent>,
    event: AsrWorkerEvent,
    retained: bool,
) -> Option<AsrWorkerEvent> {
    if retained {
        Some(event)
    } else {
        let _ = events.send(event);
        None
    }
}
