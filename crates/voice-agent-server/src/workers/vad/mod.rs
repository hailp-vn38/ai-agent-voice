use std::{
    collections::HashMap,
    sync::{Arc, Mutex, mpsc},
    thread,
    time::Instant,
};
use tokio::sync::mpsc as session_mpsc;

use crate::{
    audio::PcmF32Mono,
    providers::{VadInput, VadProvider, VadSession},
};

use super::{
    ProviderAdmissionError, ProviderCapacityPermit, ProviderRuntimeAdmission,
    ProviderWorkloadClass, WorkerIdentity, WorkerRuntimeConfig,
};

mod diagnostic;
mod pool;
#[cfg(test)]
mod tests;

pub use diagnostic::VadDiagnosticOperation;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct VadWorkerLease(u64);

/// Identity of one semantic VAD capture cycle, independent of its pinned worker lease.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct VadCaptureCycleId(u64);

impl VadCaptureCycleId {
    pub const fn new(value: u64) -> Self {
        Self(value)
    }
}

#[derive(Debug)]
pub enum VadCommand {
    Push {
        cycle: VadCaptureCycleId,
        pcm: PcmF32Mono,
    },
    Reset {
        cycle: VadCaptureCycleId,
    },
    Close,
}

#[derive(Debug, Clone, PartialEq)]
pub enum VadWorkerEvent {
    Opened {
        identity: WorkerIdentity,
    },
    Probability {
        identity: WorkerIdentity,
        cycle: VadCaptureCycleId,
        probability: crate::providers::VadProbability,
    },
    SpeechStart {
        identity: WorkerIdentity,
        cycle: VadCaptureCycleId,
        start_sample: u64,
    },
    SpeechEnd {
        identity: WorkerIdentity,
        cycle: VadCaptureCycleId,
        end_sample: u64,
    },
    ResetDone {
        identity: WorkerIdentity,
        cycle: VadCaptureCycleId,
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
    #[error("VAD native readiness failed")]
    Initialization,
    #[error("VAD worker capacity is exhausted")]
    Capacity,
    #[error("VAD worker lease is not active")]
    UnknownLease,
    #[error("VAD worker command queue is full")]
    QueueFull,
}

#[derive(Clone)]
pub struct VadWorkerRuntime {
    pool: Option<Arc<pool::VadSessionPool>>,
    unload: Arc<Mutex<()>>,
    provider: Arc<dyn VadProvider>,
    config: WorkerRuntimeConfig,
    state: Arc<Mutex<State>>,
    events_rx: Arc<Mutex<mpsc::Receiver<VadWorkerEvent>>>,
    events_tx: mpsc::Sender<VadWorkerEvent>,
    routes: Arc<Mutex<HashMap<String, session_mpsc::Sender<VadWorkerEvent>>>>,
    admission: ProviderRuntimeAdmission,
}
struct State {
    threads: super::supervision::NativeThreads,
    next_lease: u64,
    slots: HashMap<VadWorkerLease, Slot>,
}
struct Slot {
    identity: WorkerIdentity,
    command_tx: mpsc::SyncSender<VadCommand>,
    state: SlotState,
    _permit: Option<Arc<ProviderCapacityPermit>>,
}
enum SlotState {
    Active,
    Resetting { deadline: Instant },
    Cleaning { deadline: Instant },
    Quarantined,
}

impl VadWorkerRuntime {
    /// Physical work remains occupied until terminal acknowledgement, including quarantine.
    pub(crate) fn pilot_work_pending(&self) -> bool {
        self.admission.view_usage() != 0
    }

    pub fn new(provider: Arc<dyn VadProvider>, config: WorkerRuntimeConfig) -> Self {
        config.validate().expect("invalid worker runtime config");
        let admission =
            ProviderRuntimeAdmission::new(config.max_workers, config.voice_reserved_capacity);
        Self::new_with_admission(provider, config, admission)
    }

    pub(crate) fn new_with_admission(
        provider: Arc<dyn VadProvider>,
        config: WorkerRuntimeConfig,
        admission: ProviderRuntimeAdmission,
    ) -> Self {
        let (events_tx, events_rx) = mpsc::channel();
        Self {
            pool: None,
            unload: Arc::new(Mutex::new(())),
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
        }
    }

    pub fn try_new(
        provider: Arc<dyn VadProvider>,
        config: WorkerRuntimeConfig,
    ) -> Result<Self, VadWorkerError> {
        config
            .validate()
            .map_err(|_| VadWorkerError::Initialization)?;
        let admission =
            ProviderRuntimeAdmission::new(config.max_workers, config.voice_reserved_capacity);
        Self::try_new_with_admission(provider, config, admission)
    }
    pub(crate) fn try_new_with_admission(
        provider: Arc<dyn VadProvider>,
        config: WorkerRuntimeConfig,
        admission: ProviderRuntimeAdmission,
    ) -> Result<Self, VadWorkerError> {
        config
            .validate()
            .map_err(|_| VadWorkerError::Initialization)?;
        let pool = pool::VadSessionPool::initialize(provider.as_ref(), config.max_workers)
            .map_err(|_| VadWorkerError::Initialization)?;
        let mut runtime = Self::new_with_admission(provider, config, admission);
        runtime.pool = Some(pool);
        Ok(runtime)
    }

    /// Closes admission. A terminal event alone cannot acknowledge native destruction.
    pub fn shutdown_acknowledged(&self) -> bool {
        let _unload = self.unload.lock().expect("VAD unload poisoned");
        let acknowledged = {
            let mut state = self.state.lock().expect("VAD worker state poisoned");
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
        let permit = self
            .admission
            .try_admit(ProviderWorkloadClass::Voice)
            .map_err(|_| VadWorkerError::Capacity)?;
        self.open_with_permit(identity, Some(permit))
    }

    /// Opens the worker side of an Admin diagnostic after `ProviderDiagnosticService` has
    /// already admitted its exact Diagnostic permit. It must never take a second Voice permit.
    pub(super) fn open_diagnostic(
        &self,
        identity: WorkerIdentity,
    ) -> Result<VadWorkerLease, VadWorkerError> {
        self.open_with_permit(identity, None)
    }

    fn open_with_permit(
        &self,
        identity: WorkerIdentity,
        permit: Option<ProviderCapacityPermit>,
    ) -> Result<VadWorkerLease, VadWorkerError> {
        let mut state = self.state.lock().expect("VAD worker state poisoned");
        if state.slots.len() >= self.config.max_workers
            || !state.threads.can_spawn(self.config.max_workers)
        {
            return Err(VadWorkerError::Capacity);
        }
        let retained = match &self.pool {
            Some(pool) => Some(pool.take().ok_or(VadWorkerError::Capacity)?),
            None => None,
        };
        let permit = permit.map(Arc::new);
        let thread_permit = permit.clone();
        let lease = VadWorkerLease(state.next_lease);
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
        let events = self.events_tx.clone();
        let spawn = thread::Builder::new()
            .name("vad-native".into())
            .spawn(move || {
                let terminal = run_worker(provider, retained, identity, command_rx, events.clone());
                drop(thread_permit);
                if let Some(event) = terminal {
                    let _ = events.send(event);
                }
            });
        match spawn {
            Ok(handle) => state.threads.retain(handle),
            Err(_) => {
                state.slots.remove(&lease);
                return Err(VadWorkerError::Capacity);
            }
        }
        Ok(lease)
    }

    /// Acquires only capacity not reserved for Voice work.
    pub fn admit_diagnostic(&self) -> Result<ProviderCapacityPermit, ProviderAdmissionError> {
        let state = self.state.lock().expect("VAD worker state poisoned");
        if state.threads.is_closed() {
            return Err(ProviderAdmissionError::Capacity);
        }
        self.admission.try_admit(ProviderWorkloadClass::Diagnostic)
    }

    /// Builds a standalone diagnostic against this already-materialized VAD provider. Admission
    /// remains owned by `ProviderDiagnosticService`; this never changes capture-cycle state.
    pub fn diagnostic(self: &Arc<Self>) -> VadDiagnosticOperation {
        VadDiagnosticOperation::new(Arc::clone(self))
    }

    pub(super) fn quarantine_diagnostic(&self, lease: VadWorkerLease) {
        if let Some(slot) = self
            .state
            .lock()
            .expect("VAD worker state poisoned")
            .slots
            .get_mut(&lease)
        {
            slot.state = SlotState::Quarantined;
        }
    }
    pub fn send(&self, lease: VadWorkerLease, command: VadCommand) -> Result<(), VadWorkerError> {
        let mut state = self.state.lock().expect("VAD worker state poisoned");
        let slot = state
            .slots
            .get_mut(&lease)
            .ok_or(VadWorkerError::UnknownLease)?;
        let next_state = match &command {
            VadCommand::Reset { .. } if !matches!(slot.state, SlotState::Active) => {
                return Err(VadWorkerError::UnknownLease);
            }
            VadCommand::Reset { .. } => Some(SlotState::Resetting {
                deadline: Instant::now() + self.config.final_timeout,
            }),
            VadCommand::Close => Some(SlotState::Cleaning {
                deadline: Instant::now() + self.config.cleanup_grace,
            }),
            VadCommand::Push { .. } if !matches!(slot.state, SlotState::Active) => {
                return Err(VadWorkerError::UnknownLease);
            }
            VadCommand::Push { .. } => None,
        };
        slot.command_tx
            .try_send(command)
            .map_err(|error| match error {
                mpsc::TrySendError::Full(_) => VadWorkerError::QueueFull,
                mpsc::TrySendError::Disconnected(_) => VadWorkerError::UnknownLease,
            })?;
        if let Some(next_state) = next_state {
            slot.state = next_state;
        }
        Ok(())
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
        let mut state = self.state.lock().expect("VAD worker state poisoned");
        match event {
            VadWorkerEvent::ResetDone { identity, .. } => {
                if let Some(slot) = state
                    .slots
                    .values_mut()
                    .find(|slot| slot.identity == *identity)
                    && matches!(slot.state, SlotState::Resetting { .. })
                {
                    slot.state = SlotState::Active;
                }
            }
            VadWorkerEvent::Closed { identity } | VadWorkerEvent::Failed { identity } => {
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
            _ => {}
        }
    }

    fn dispatch(&self, event: VadWorkerEvent) {
        self.observe(&event);
        let session = match &event {
            VadWorkerEvent::Opened { identity }
            | VadWorkerEvent::Probability { identity, .. }
            | VadWorkerEvent::SpeechStart { identity, .. }
            | VadWorkerEvent::SpeechEnd { identity, .. }
            | VadWorkerEvent::ResetDone { identity, .. }
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
    retained: Option<Box<dyn VadSession>>,
    identity: WorkerIdentity,
    commands: mpsc::Receiver<VadCommand>,
    events: mpsc::Sender<VadWorkerEvent>,
) -> Option<VadWorkerEvent> {
    let retained_worker = retained.is_some();
    let Ok(mut session) = retained.map(Ok).unwrap_or_else(|| provider.open()) else {
        return publish_terminal(
            &events,
            VadWorkerEvent::Failed { identity },
            retained_worker,
        );
    };
    if events
        .send(VadWorkerEvent::Opened {
            identity: identity.clone(),
        })
        .is_err()
    {
        return None;
    }
    let mut rechunker = VadRechunker::default();
    while let Ok(command) = commands.recv() {
        match command {
            VadCommand::Push { cycle, pcm } => {
                let inputs = match rechunker.push(pcm) {
                    Ok(inputs) => inputs,
                    Err(()) => {
                        return publish_terminal(
                            &events,
                            VadWorkerEvent::Failed { identity },
                            retained_worker,
                        );
                    }
                };
                for input in inputs {
                    let start_sample = input.start_sample;
                    let probability = match session.push(input) {
                        Ok(probability) => probability,
                        Err(_) => {
                            return publish_terminal(
                                &events,
                                VadWorkerEvent::Failed { identity },
                                retained_worker,
                            );
                        }
                    };
                    if probability.start_sample != start_sample
                        || probability.end_sample != start_sample + 512
                    {
                        return publish_terminal(
                            &events,
                            VadWorkerEvent::Failed { identity },
                            retained_worker,
                        );
                    }
                    if events
                        .send(VadWorkerEvent::Probability {
                            identity: identity.clone(),
                            cycle,
                            probability,
                        })
                        .is_err()
                    {
                        return None;
                    }
                }
            }
            VadCommand::Reset { cycle } => match session.reset() {
                Ok(()) => {
                    rechunker.reset();
                    let _ = events.send(VadWorkerEvent::ResetDone {
                        identity: identity.clone(),
                        cycle,
                    });
                }
                Err(_) => {
                    return publish_terminal(
                        &events,
                        VadWorkerEvent::Failed { identity },
                        retained_worker,
                    );
                }
            },
            VadCommand::Close => match session.close() {
                Ok(()) => {
                    return publish_terminal(
                        &events,
                        VadWorkerEvent::Closed { identity },
                        retained_worker,
                    );
                }
                Err(_) => {
                    return publish_terminal(
                        &events,
                        VadWorkerEvent::Failed { identity },
                        retained_worker,
                    );
                }
            },
        }
    }
    None
}

fn publish_terminal(
    events: &mpsc::Sender<VadWorkerEvent>,
    event: VadWorkerEvent,
    retained: bool,
) -> Option<VadWorkerEvent> {
    if retained {
        Some(event)
    } else {
        let _ = events.send(event);
        None
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
