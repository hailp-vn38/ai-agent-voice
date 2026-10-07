use std::{
    collections::{HashMap, HashSet},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread,
};

use tokio::time::Instant;

use super::{
    ProviderAdmissionError, ProviderCapacityPermit, ProviderRuntimeAdmission,
    ProviderWorkloadClass, WorkerRuntimeConfig,
};
use crate::{
    audio::PcmF32Mono,
    providers::{TtsBinding, TtsDiagnosticRequest, TtsProvider, TtsSynthesisRequest},
};

mod diagnostic;
mod pool;
mod stream;
#[cfg(test)]
mod tests;

use pool::{TtsPoolOwner, stop_and_join, worker_loop};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct TtsLease(u64);
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct TtsStreamId(u64);
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
    #[error("TTS worker initialization failed")]
    Initialization,
    #[error("TTS native cleanup has not acknowledged completion")]
    Quarantined,
    #[error("TTS worker configuration is invalid")]
    InvalidConfig,
    #[error("TTS stream binding does not match its logical runtime")]
    BindingMismatch,
    #[error("TTS worker lease is not active")]
    UnknownLease,
}

enum WorkerCommand {
    Start {
        request: TtsWorkRequest,
        cancelled: Arc<AtomicBool>,
        events: mpsc::SyncSender<TtsWorkerEvent>,
    },
    Reset {
        pending: Arc<AtomicBool>,
    },
    Shutdown,
}

enum TtsWorkRequest {
    Voice(TtsSynthesisRequest),
    Diagnostic {
        request: TtsSynthesisRequest,
        original: TtsDiagnosticRequest,
    },
}
struct WorkerRecord {
    command_tx: mpsc::SyncSender<WorkerCommand>,
    busy: bool,
    stream: Option<TtsStreamId>,
    quarantined: bool,
    healthy: Arc<AtomicBool>,
    reset_pending: Arc<AtomicBool>,
    thread: Option<thread::JoinHandle<()>>,
}
struct Slot {
    worker: usize,
    stream: Option<TtsStreamId>,
    cancelled: Arc<AtomicBool>,
    events: Arc<Mutex<mpsc::Receiver<TtsWorkerEvent>>>,
    deadline: Instant,
    refresh_on_pcm: bool,
    cleanup_deadline: Option<Instant>,
    quarantined: bool,
    cleanup_reported: bool,
    _permit: Option<ProviderCapacityPermit>,
}
struct State {
    closed: bool,
    unloading: bool,
    unloaded: bool,
    next: u64,
    next_stream: u64,
    slots: HashMap<TtsLease, Slot>,
    streams: HashMap<TtsStreamId, StreamRecord>,
    closed_streams: HashSet<TtsStreamId>,
    workers: Vec<WorkerRecord>,
}

struct StreamRecord {
    worker: Option<usize>,
    binding: TtsBinding,
}

/// Fixed native pool: a thread (and native sessions supplied by `TtsWorker`) is created once per
/// configured slot, never once per sentence.
#[derive(Clone)]
pub struct TtsWorkerRuntime {
    _owner: Arc<TtsPoolOwner>,
    readiness: super::NativeReadiness,
    provider: Arc<dyn TtsProvider>,
    config: WorkerRuntimeConfig,
    state: Arc<Mutex<State>>,
    admission: ProviderRuntimeAdmission,
    binding: TtsBinding,
}

/// Provider-boundary WAV returned by a TTS diagnostic. It deliberately never enters Voice
/// delivery conversion, resampling or Opus encoding.
pub struct TtsDiagnosticOutput {
    pub wav: Vec<u8>,
}

pub struct TtsDiagnosticOperation {
    runtime: Arc<TtsWorkerRuntime>,
    request: TtsDiagnosticRequest,
    lease: Option<TtsLease>,
    terminal: bool,
}

impl TtsWorkerRuntime {
    /// Physical work remains occupied until terminal acknowledgement, including quarantine.
    pub(crate) fn pilot_work_pending(&self) -> bool {
        self.admission.view_usage() != 0 || self.reset_pending()
    }
    pub(crate) fn reset_pending(&self) -> bool {
        self.state
            .lock()
            .expect("TTS worker state poisoned")
            .workers
            .iter()
            .any(|worker| worker.reset_pending.load(Ordering::Acquire))
    }

    pub fn new(provider: Arc<dyn TtsProvider>, config: WorkerRuntimeConfig) -> Self {
        Self::new_with_binding(provider, config, TtsBinding::readiness())
    }

    pub fn new_with_binding(
        provider: Arc<dyn TtsProvider>,
        config: WorkerRuntimeConfig,
        binding: TtsBinding,
    ) -> Self {
        Self::try_new_with_binding(provider, config, binding)
            .expect("cannot initialize TTS runtime")
    }

    /// Blocking construction seam. Ready means every retained native worker acknowledged init.
    /// Materialization calls this on its bounded blocking executor, never on the audio actor.
    pub fn try_new(
        provider: Arc<dyn TtsProvider>,
        config: WorkerRuntimeConfig,
    ) -> Result<Self, TtsWorkerError> {
        config
            .validate()
            .map_err(|_| TtsWorkerError::InvalidConfig)?;
        Self::try_new_with_binding(provider, config, TtsBinding::readiness())
    }

    pub fn try_new_with_binding(
        provider: Arc<dyn TtsProvider>,
        config: WorkerRuntimeConfig,
        binding: TtsBinding,
    ) -> Result<Self, TtsWorkerError> {
        let admission =
            ProviderRuntimeAdmission::new(config.max_workers, config.voice_reserved_capacity);
        Self::try_new_with_admission_and_binding(provider, config, admission, binding)
    }

    pub(crate) fn try_new_with_admission_and_binding(
        provider: Arc<dyn TtsProvider>,
        config: WorkerRuntimeConfig,
        admission: ProviderRuntimeAdmission,
        binding: TtsBinding,
    ) -> Result<Self, TtsWorkerError> {
        config
            .validate()
            .map_err(|_| TtsWorkerError::InvalidConfig)?;
        let mut readiness = super::NativeReadiness::default();
        let mut workers: Vec<WorkerRecord> = Vec::with_capacity(config.max_workers);
        for index in 0..config.max_workers {
            let (command_tx, command_rx) = mpsc::sync_channel(config.command_capacity);
            let (ready_tx, ready_rx) = mpsc::sync_channel(1);
            let worker_provider = Arc::clone(&provider);
            let healthy = Arc::new(AtomicBool::new(true));
            let worker_healthy = Arc::clone(&healthy);
            let spawn = thread::Builder::new()
                .name(format!("tts-native-{index}"))
                .spawn(move || worker_loop(worker_provider, command_rx, ready_tx, worker_healthy));
            let thread = match spawn {
                Ok(thread) => thread,
                Err(_) => {
                    return Err(if stop_and_join(&mut workers) {
                        TtsWorkerError::Initialization
                    } else {
                        TtsWorkerError::Quarantined
                    });
                }
            };
            if let Ok(Some(worker_readiness)) = ready_rx.recv() {
                readiness.add(worker_readiness);
                workers.push(WorkerRecord {
                    command_tx,
                    busy: false,
                    stream: None,
                    quarantined: false,
                    healthy,
                    reset_pending: Arc::new(AtomicBool::new(false)),
                    thread: Some(thread),
                });
            } else {
                let exited = thread.join().is_ok();
                let pool_exited = stop_and_join(&mut workers);
                return Err(if exited && pool_exited {
                    TtsWorkerError::Initialization
                } else {
                    TtsWorkerError::Quarantined
                });
            }
        }
        let state = Arc::new(Mutex::new(State {
            closed: false,
            unloading: false,
            unloaded: false,
            next: 1,
            next_stream: 1,
            slots: HashMap::new(),
            streams: HashMap::new(),
            closed_streams: HashSet::new(),
            workers,
        }));
        Ok(Self {
            _owner: Arc::new(TtsPoolOwner(Arc::clone(&state))),
            readiness,
            provider,
            config,
            state,
            admission,
            binding,
        })
    }
    pub fn provider(&self) -> Arc<dyn TtsProvider> {
        Arc::clone(&self.provider)
    }
    /// Closes admission and joins retained workers. Call only on the manager's native
    /// unload thread: native destruction may block. An active stream or operation keeps
    /// ownership and accounting with the caller until its cleanup is acknowledged.
    pub(crate) fn readiness(&self) -> super::NativeReadiness {
        self.readiness
    }
    pub(crate) fn health_flags(&self) -> Vec<Arc<AtomicBool>> {
        self.state
            .lock()
            .expect("TTS worker state poisoned")
            .workers
            .iter()
            .map(|worker| Arc::clone(&worker.healthy))
            .collect()
    }

    pub(crate) fn logical_view(
        &self,
        quota: ProviderRuntimeAdmission,
        binding: TtsBinding,
    ) -> Self {
        let mut view = self.clone();
        view.admission = quota.composed(&self.admission);
        view.binding = binding;
        view
    }

    #[doc(hidden)]
    pub fn logical_view_for_test(&self, binding: TtsBinding) -> Self {
        self.logical_view(self.admission.clone(), binding)
    }
    pub fn shutdown_acknowledged(&self) -> bool {
        let mut workers = {
            let mut state = self.state.lock().expect("TTS worker state poisoned");
            state.closed = true;
            if state.unloaded {
                return true;
            }
            if state.unloading
                || !state.slots.is_empty()
                || !state.streams.is_empty()
                || self.admission.active_work() != 0
            {
                return false;
            }
            state.unloading = true;
            std::mem::take(&mut state.workers)
        };
        let acknowledged = stop_and_join(&mut workers);
        let mut state = self.state.lock().expect("TTS worker state poisoned");
        state.unloaded = acknowledged;
        // A panicked destructor has no trustworthy cleanup acknowledgement. Do not retry
        // with an empty pool and accidentally report success.
        state.unloading = !acknowledged;
        acknowledged
    }
}
