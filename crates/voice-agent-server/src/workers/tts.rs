use std::{
    collections::{HashMap, HashSet},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread,
    time::Instant,
};

use super::{
    ProviderAdmissionError, ProviderCapacityPermit, ProviderRuntimeAdmission,
    ProviderWorkloadClass, WorkerRuntimeConfig,
};
use crate::{
    audio::PcmF32Mono,
    providers::{
        TtsBinding, TtsDiagnosticRequest, TtsError, TtsProvider, TtsStream, TtsSynthesisRequest,
        TtsWorker,
    },
    services::provider_diagnostic::{
        ProviderDiagnosticOperation, ProviderDiagnosticOperationError,
    },
};
use tokio_util::sync::CancellationToken;

const MAX_DIAGNOSTIC_WAV_BYTES: usize = 16 * 1024 * 1024;

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
    Reset,
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
    thread: Option<thread::JoinHandle<()>>,
}
struct Slot {
    worker: usize,
    stream: Option<TtsStreamId>,
    cancelled: Arc<AtomicBool>,
    events: Arc<Mutex<mpsc::Receiver<TtsWorkerEvent>>>,
    deadline: Instant,
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
    pub fn begin_stream(&self) -> TtsStreamId {
        let mut state = self.state.lock().expect("TTS worker state poisoned");
        let stream = TtsStreamId(state.next_stream);
        state.next_stream += 1;
        if !state.closed {
            state.streams.insert(
                stream,
                StreamRecord {
                    worker: None,
                    binding: self.binding.clone(),
                },
            );
        }
        stream
    }
    pub fn close_stream(&self, stream: TtsStreamId) {
        let mut state = self.state.lock().expect("TTS worker state poisoned");
        let Some(worker) = state
            .streams
            .remove(&stream)
            .and_then(|record| record.worker)
        else {
            return;
        };
        if state.slots.values().any(|slot| slot.stream == Some(stream)) {
            state.closed_streams.insert(stream);
        } else {
            let worker = &mut state.workers[worker];
            worker.busy = false;
            worker.stream = None;
            let _ = worker.command_tx.try_send(WorkerCommand::Reset);
        }
    }
    pub fn start(&self, text: String) -> Result<TtsLease, TtsWorkerError> {
        self.start_internal(
            None,
            TtsWorkRequest::Voice(TtsSynthesisRequest {
                text,
                selection: self.binding.clone(),
            }),
            true,
        )
    }
    pub fn start_in_stream(
        &self,
        stream: TtsStreamId,
        text: String,
    ) -> Result<TtsLease, TtsWorkerError> {
        self.start_internal(
            Some(stream),
            TtsWorkRequest::Voice(TtsSynthesisRequest {
                text,
                selection: self.binding.clone(),
            }),
            true,
        )
    }
    fn start_internal(
        &self,
        stream: Option<TtsStreamId>,
        request: TtsWorkRequest,
        reserve_voice_capacity: bool,
    ) -> Result<TtsLease, TtsWorkerError> {
        let permit = reserve_voice_capacity
            .then(|| self.admission.try_admit(ProviderWorkloadClass::Voice))
            .transpose()
            .map_err(|_| TtsWorkerError::Capacity)?;
        let mut state = self.state.lock().expect("TTS worker state poisoned");
        if state.closed {
            return Err(TtsWorkerError::Capacity);
        }
        let worker = match stream {
            Some(stream) => match state
                .streams
                .get(&stream)
                .ok_or(TtsWorkerError::UnknownLease)?
            {
                StreamRecord { binding, .. } if binding != request.selection() => {
                    return Err(TtsWorkerError::BindingMismatch);
                }
                StreamRecord {
                    worker: Some(worker),
                    ..
                } if !state.workers[*worker].busy
                    && !state.workers[*worker].quarantined
                    && state.workers[*worker].healthy.load(Ordering::Acquire) =>
                {
                    *worker
                }
                StreamRecord {
                    worker: Some(_), ..
                } => return Err(TtsWorkerError::Capacity),
                StreamRecord { worker: None, .. } => state
                    .workers
                    .iter()
                    .position(|entry| {
                        !entry.busy && !entry.quarantined && entry.healthy.load(Ordering::Acquire)
                    })
                    .ok_or(TtsWorkerError::Capacity)?,
            },
            None => state
                .workers
                .iter()
                .position(|entry| {
                    !entry.busy
                        && entry.stream.is_none()
                        && !entry.quarantined
                        && entry.healthy.load(Ordering::Acquire)
                })
                .ok_or(TtsWorkerError::Capacity)?,
        };
        let lease = TtsLease(state.next);
        state.next += 1;
        let cancelled = Arc::new(AtomicBool::new(false));
        let (events_tx, events_rx) = mpsc::sync_channel(self.config.command_capacity);
        state.workers[worker].busy = true;
        if let Some(stream) = stream {
            state.workers[worker].stream = Some(stream);
            state
                .streams
                .get_mut(&stream)
                .expect("stream checked above")
                .worker = Some(worker);
        }
        state.slots.insert(
            lease,
            Slot {
                worker,
                stream,
                cancelled: Arc::clone(&cancelled),
                events: Arc::new(Mutex::new(events_rx)),
                deadline: Instant::now() + self.config.final_timeout,
                cleanup_deadline: None,
                quarantined: false,
                cleanup_reported: false,
                _permit: permit,
            },
        );
        let command = state.workers[worker].command_tx.clone();
        if command
            .send(WorkerCommand::Start {
                request,
                cancelled,
                events: events_tx,
            })
            .is_err()
        {
            release_terminal(&mut state, lease);
            return Err(TtsWorkerError::Capacity);
        }
        Ok(lease)
    }
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
            let worker = slot.worker;
            state.workers[worker].quarantined = true;
            return Ok(Some(TtsWorkerEvent::CleanupTimedOut));
        }
        let event = match slot
            .events
            .lock()
            .expect("TTS worker event receiver poisoned")
            .try_recv()
        {
            Ok(event) => event,
            Err(mpsc::TryRecvError::Empty) => {
                if Instant::now() >= slot.deadline && slot.cleanup_deadline.is_none() {
                    slot.cancelled.store(true, Ordering::Release);
                    slot.cleanup_deadline = Some(Instant::now() + self.config.cleanup_grace);
                    return Ok(Some(TtsWorkerEvent::TimedOut));
                }
                return Ok(None);
            }
            Err(mpsc::TryRecvError::Disconnected) => TtsWorkerEvent::Failed,
        };
        if event.is_terminal() {
            release_terminal(&mut state, lease);
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
    pub fn cancel_and_detach(&self, lease: TtsLease) -> Result<(), TtsWorkerError> {
        self.cancel(lease)?;
        let state = Arc::clone(&self.state);
        let grace = self.config.cleanup_grace;
        thread::spawn(move || {
            let events = state
                .lock()
                .ok()
                .and_then(|state| state.slots.get(&lease).map(|slot| Arc::clone(&slot.events)));
            let Some(events) = events else { return };
            let deadline = Instant::now() + grace;
            let terminal = loop {
                let remaining = deadline.saturating_duration_since(Instant::now());
                if remaining.is_zero() {
                    break false;
                }
                match events
                    .lock()
                    .expect("TTS worker event receiver poisoned")
                    .recv_timeout(remaining)
                {
                    Ok(event) if event.is_terminal() => break true,
                    Ok(TtsWorkerEvent::Pcm(_)) => continue,
                    _ => break false,
                }
            };
            let mut state = state.lock().expect("TTS worker state poisoned");
            if terminal {
                release_terminal(&mut state, lease);
            } else if let Some(slot) = state.slots.get_mut(&lease) {
                slot.quarantined = true;
                slot.cleanup_reported = true;
                let worker = slot.worker;
                state.workers[worker].quarantined = true;
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

    /// Acquires capacity for a bounded diagnostic operation.
    pub fn admit_diagnostic(&self) -> Result<ProviderCapacityPermit, ProviderAdmissionError> {
        let state = self.state.lock().expect("TTS worker state poisoned");
        if state.closed {
            return Err(ProviderAdmissionError::Capacity);
        }
        self.admission.try_admit(ProviderWorkloadClass::Diagnostic)
    }

    /// Builds a diagnostic for an already-materialized native worker pool. Capacity admission is
    /// owned by `ProviderDiagnosticService`; this value cannot load a provider or alter config.
    pub fn diagnostic(self: &Arc<Self>, request: TtsDiagnosticRequest) -> TtsDiagnosticOperation {
        TtsDiagnosticOperation {
            runtime: Arc::clone(self),
            request,
            lease: None,
            terminal: false,
        }
    }

    pub fn validate_diagnostic(&self, request: &TtsDiagnosticRequest) -> Result<(), TtsError> {
        self.effective_diagnostic_request(request.clone())
            .map(|_| ())
    }

    pub(super) fn start_diagnostic(
        &self,
        request: TtsDiagnosticRequest,
    ) -> Result<TtsLease, TtsWorkerError> {
        let original = request.clone();
        let request = self
            .effective_diagnostic_request(request)
            .map_err(|_| TtsWorkerError::InvalidConfig)?;
        self.start_internal(
            None,
            TtsWorkRequest::Diagnostic { request, original },
            false,
        )
    }

    fn effective_diagnostic_request(
        &self,
        request: TtsDiagnosticRequest,
    ) -> Result<TtsSynthesisRequest, TtsError> {
        let selection = TtsBinding {
            voice: request
                .voice
                .clone()
                .unwrap_or_else(|| self.binding.voice.clone()),
            language: request
                .language
                .clone()
                .unwrap_or_else(|| self.binding.language.clone()),
        };
        if self.provider.adapter() == "zerotts_onnx" {
            self.provider.validate_diagnostic(&TtsDiagnosticRequest {
                text: request.text.clone(),
                voice: Some(selection.voice.clone()),
                language: Some(selection.language.clone()),
            })?;
        } else {
            self.provider.validate_diagnostic(&request)?;
        }
        Ok(TtsSynthesisRequest {
            text: request.text,
            selection,
        })
    }

    pub(super) fn quarantine_diagnostic(&self, lease: TtsLease) {
        let mut state = self.state.lock().expect("TTS worker state poisoned");
        if let Some(worker) = state.slots.get_mut(&lease).map(|slot| {
            slot.quarantined = true;
            slot.cleanup_reported = true;
            slot.worker
        }) {
            state.workers[worker].quarantined = true;
        }
    }
}

impl TtsWorkRequest {
    fn selection(&self) -> &TtsBinding {
        match self {
            Self::Voice(request) | Self::Diagnostic { request, .. } => &request.selection,
        }
    }
}
struct TtsPoolOwner(Arc<Mutex<State>>);
impl Drop for TtsPoolOwner {
    fn drop(&mut self) {
        if let Ok(state) = self.0.lock() {
            for worker in &state.workers {
                let _ = worker.command_tx.try_send(WorkerCommand::Shutdown);
            }
        }
    }
}
fn release_terminal(state: &mut State, lease: TtsLease) {
    let Some(slot) = state.slots.remove(&lease) else {
        return;
    };
    let release = slot.stream.is_none()
        || slot
            .stream
            .is_some_and(|stream| state.closed_streams.remove(&stream));
    let worker = &mut state.workers[slot.worker];
    worker.busy = false;
    if release {
        worker.stream = None;
        let _ = worker.command_tx.try_send(WorkerCommand::Reset);
    }
}
fn stop_and_join(workers: &mut [WorkerRecord]) -> bool {
    for worker in workers.iter() {
        let _ = worker.command_tx.send(WorkerCommand::Shutdown);
    }
    let mut acknowledged = true;
    for worker in workers.iter_mut() {
        if let Some(thread) = worker.thread.take() {
            acknowledged &= thread.join().is_ok();
        }
    }
    acknowledged
}

fn worker_loop(
    provider: Arc<dyn TtsProvider>,
    commands: mpsc::Receiver<WorkerCommand>,
    ready: mpsc::SyncSender<Option<super::NativeReadiness>>,
    healthy: Arc<AtomicBool>,
) {
    let started = Instant::now();
    let mut worker = match provider.open_worker() {
        Ok(worker) => worker,
        Err(TtsError::WorkerUnsupported) => Box::new(ProviderWorker::new(provider)),
        Err(_) => {
            let _ = ready.send(None);
            return;
        }
    };
    let initialization = started.elapsed();
    let started = Instant::now();
    if worker.warmup().is_err() {
        let _ = ready.send(None);
        return;
    }
    if ready
        .send(Some(super::NativeReadiness {
            initialization,
            warmup: started.elapsed(),
        }))
        .is_err()
    {
        return;
    }
    while let Ok(command) = commands.recv() {
        match command {
            WorkerCommand::Shutdown => break,
            WorkerCommand::Reset => {
                if worker.reset().is_err() {
                    healthy.store(false, Ordering::Release);
                    break;
                }
            }
            WorkerCommand::Start {
                request,
                cancelled,
                events,
            } => {
                let diagnostic = matches!(&request, TtsWorkRequest::Diagnostic { .. });
                if let TtsWorkRequest::Voice(request) = &request {
                    tracing::info!(
                        chars = request.text.chars().count(),
                        delivery = "worker",
                        "TTS synthesis input"
                    );
                }
                let started = Instant::now();
                let result = match request {
                    TtsWorkRequest::Voice(request) => {
                        worker.synthesize(&request, &cancelled, &mut |pcm| {
                            if cancelled.load(Ordering::Acquire) {
                                return Err(TtsError::Failed);
                            }
                            send_pcm_until_cancelled(&events, &cancelled, pcm)
                        })
                    }
                    TtsWorkRequest::Diagnostic { request, original } => worker
                        .synthesize_diagnostic(&request, &original, &cancelled, &mut |pcm| {
                            if cancelled.load(Ordering::Acquire) {
                                return Err(TtsError::Failed);
                            }
                            send_pcm_until_cancelled(&events, &cancelled, pcm)
                        }),
                };
                let event = if cancelled.load(Ordering::Acquire) {
                    TtsWorkerEvent::Cancelled
                } else if result.is_ok() {
                    TtsWorkerEvent::Finished
                } else {
                    TtsWorkerEvent::Failed
                };
                if !diagnostic {
                    tracing::info!(outcome = ?event, elapsed_ms = started.elapsed().as_millis(), "TTS synthesis ended");
                }
                let _ = events.send(event);
            }
        }
    }
}

fn send_pcm_until_cancelled(
    events: &mpsc::SyncSender<TtsWorkerEvent>,
    cancelled: &AtomicBool,
    pcm: PcmF32Mono,
) -> Result<(), TtsError> {
    let mut pending = TtsWorkerEvent::Pcm(pcm);
    loop {
        if cancelled.load(Ordering::Acquire) {
            return Err(TtsError::Failed);
        }
        match events.try_send(pending) {
            Ok(()) => return Ok(()),
            Err(mpsc::TrySendError::Full(event)) => {
                pending = event;
                thread::sleep(std::time::Duration::from_millis(1));
            }
            Err(mpsc::TrySendError::Disconnected(_)) => return Err(TtsError::Failed),
        }
    }
}
struct ProviderWorker {
    provider: Arc<dyn TtsProvider>,
    stream: Option<Box<dyn TtsStream>>,
}
impl ProviderWorker {
    fn new(provider: Arc<dyn TtsProvider>) -> Self {
        let stream = provider.open_stream();
        Self { provider, stream }
    }
}
impl TtsWorker for ProviderWorker {
    fn synthesize(
        &mut self,
        request: &TtsSynthesisRequest,
        cancelled: &AtomicBool,
        on_pcm: &mut dyn FnMut(PcmF32Mono) -> Result<(), TtsError>,
    ) -> Result<(), TtsError> {
        if cancelled.load(Ordering::Acquire) {
            return Err(TtsError::Failed);
        }
        match &mut self.stream {
            Some(stream) => stream.synthesize(&request.text, on_pcm),
            None => self.provider.synthesize_stream(&request.text, on_pcm),
        }
    }
    fn reset(&mut self) -> Result<(), TtsError> {
        Ok(())
    }

    fn synthesize_diagnostic(
        &mut self,
        _request: &TtsSynthesisRequest,
        original: &TtsDiagnosticRequest,
        cancelled: &AtomicBool,
        on_pcm: &mut dyn FnMut(PcmF32Mono) -> Result<(), TtsError>,
    ) -> Result<(), TtsError> {
        self.provider
            .synthesize_diagnostic(original, cancelled, on_pcm)
    }
}

#[async_trait::async_trait]
impl ProviderDiagnosticOperation for TtsDiagnosticOperation {
    type Output = TtsDiagnosticOutput;

    async fn execute(
        &mut self,
        _: CancellationToken,
    ) -> Result<Self::Output, ProviderDiagnosticOperationError> {
        let lease = self
            .runtime
            .start_diagnostic(self.request.clone())
            .map_err(|_| ProviderDiagnosticOperationError::Unavailable)?;
        self.lease = Some(lease);
        let mut sample_rate = None;
        let mut samples = Vec::new();
        loop {
            match self
                .runtime
                .poll(lease)
                .map_err(|_| ProviderDiagnosticOperationError::Failed)?
            {
                Some(TtsWorkerEvent::Pcm(pcm)) => append_pcm(&mut sample_rate, &mut samples, pcm)?,
                Some(TtsWorkerEvent::Finished) => {
                    self.terminal = true;
                    return wav(sample_rate, samples).map(|wav| TtsDiagnosticOutput { wav });
                }
                Some(TtsWorkerEvent::Failed) | Some(TtsWorkerEvent::Cancelled) => {
                    self.terminal = true;
                    return Err(ProviderDiagnosticOperationError::Failed);
                }
                Some(TtsWorkerEvent::CleanupTimedOut) => {
                    return Err(ProviderDiagnosticOperationError::Unavailable);
                }
                Some(TtsWorkerEvent::TimedOut) | None => {
                    tokio::time::sleep(std::time::Duration::from_millis(1)).await
                }
            }
        }
    }

    fn cancel_exact(&mut self) {
        if let Some(lease) = self.lease {
            let _ = self.runtime.cancel(lease);
        }
    }

    async fn await_terminal_acknowledgement(&mut self) -> bool {
        let Some(lease) = self.lease else {
            return self.terminal;
        };
        while !self.terminal {
            match self.runtime.poll(lease) {
                Ok(Some(event)) if event.is_terminal() => self.terminal = true,
                Ok(Some(TtsWorkerEvent::CleanupTimedOut)) | Err(_) => return false,
                _ => tokio::time::sleep(std::time::Duration::from_millis(1)).await,
            }
        }
        true
    }

    fn quarantine_exact(&mut self) {
        if let Some(lease) = self.lease {
            self.runtime.quarantine_diagnostic(lease);
        }
    }
}

fn append_pcm(
    sample_rate: &mut Option<u32>,
    output: &mut Vec<i16>,
    pcm: PcmF32Mono,
) -> Result<(), ProviderDiagnosticOperationError> {
    if pcm.sample_rate_hz() == 0 || sample_rate.is_some_and(|rate| rate != pcm.sample_rate_hz()) {
        return Err(ProviderDiagnosticOperationError::InvalidResponse);
    }
    *sample_rate = Some(pcm.sample_rate_hz());
    let added = pcm.samples().len();
    if output.len().saturating_add(added) > (MAX_DIAGNOSTIC_WAV_BYTES - 44) / 2
        || pcm.samples().iter().any(|sample| !sample.is_finite())
    {
        return Err(ProviderDiagnosticOperationError::InvalidResponse);
    }
    output.extend(
        pcm.samples()
            .iter()
            .map(|sample| (sample.clamp(-1.0, 1.0) * f32::from(i16::MAX)).round() as i16),
    );
    Ok(())
}

fn wav(
    sample_rate: Option<u32>,
    samples: Vec<i16>,
) -> Result<Vec<u8>, ProviderDiagnosticOperationError> {
    let sample_rate = sample_rate
        .filter(|_| !samples.is_empty())
        .ok_or(ProviderDiagnosticOperationError::InvalidResponse)?;
    let data_bytes = u32::try_from(
        samples
            .len()
            .checked_mul(2)
            .ok_or(ProviderDiagnosticOperationError::InvalidResponse)?,
    )
    .map_err(|_| ProviderDiagnosticOperationError::InvalidResponse)?;
    let mut result = Vec::with_capacity(44 + data_bytes as usize);
    result.extend_from_slice(b"RIFF");
    result.extend_from_slice(
        &(36_u32
            .checked_add(data_bytes)
            .ok_or(ProviderDiagnosticOperationError::InvalidResponse)?)
        .to_le_bytes(),
    );
    result.extend_from_slice(b"WAVEfmt ");
    result.extend_from_slice(&16_u32.to_le_bytes());
    result.extend_from_slice(&1_u16.to_le_bytes());
    result.extend_from_slice(&1_u16.to_le_bytes());
    result.extend_from_slice(&sample_rate.to_le_bytes());
    result.extend_from_slice(
        &(sample_rate
            .checked_mul(2)
            .ok_or(ProviderDiagnosticOperationError::InvalidResponse)?)
        .to_le_bytes(),
    );
    result.extend_from_slice(&2_u16.to_le_bytes());
    result.extend_from_slice(&16_u16.to_le_bytes());
    result.extend_from_slice(b"data");
    result.extend_from_slice(&data_bytes.to_le_bytes());
    for sample in samples {
        result.extend_from_slice(&sample.to_le_bytes());
    }
    Ok(result)
}
