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
    providers::{TtsDiagnosticRequest, TtsError, TtsProvider, TtsStream, TtsWorker},
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
    Voice(String),
    Diagnostic(TtsDiagnosticRequest),
}
struct WorkerRecord {
    command_tx: mpsc::SyncSender<WorkerCommand>,
    busy: bool,
    stream: Option<TtsStreamId>,
    quarantined: bool,
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
    next: u64,
    next_stream: u64,
    slots: HashMap<TtsLease, Slot>,
    streams: HashMap<TtsStreamId, Option<usize>>,
    closed_streams: HashSet<TtsStreamId>,
    workers: Vec<WorkerRecord>,
}

/// Fixed native pool: a thread (and native sessions supplied by `TtsWorker`) is created once per
/// configured slot, never once per sentence.
pub struct TtsWorkerRuntime {
    provider: Arc<dyn TtsProvider>,
    config: WorkerRuntimeConfig,
    state: Arc<Mutex<State>>,
    admission: ProviderRuntimeAdmission,
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
        config
            .validate()
            .expect("invalid TTS worker runtime config");
        let mut workers = Vec::with_capacity(config.max_workers);
        for index in 0..config.max_workers {
            let (command_tx, command_rx) = mpsc::sync_channel(config.command_capacity);
            let provider = Arc::clone(&provider);
            thread::Builder::new()
                .name(format!("tts-native-{index}"))
                .spawn(move || worker_loop(provider, command_rx))
                .expect("cannot start TTS native worker");
            workers.push(WorkerRecord {
                command_tx,
                busy: false,
                stream: None,
                quarantined: false,
            });
        }
        let admission =
            ProviderRuntimeAdmission::new(config.max_workers, config.voice_reserved_capacity);
        Self {
            provider,
            config,
            state: Arc::new(Mutex::new(State {
                next: 1,
                next_stream: 1,
                slots: HashMap::new(),
                streams: HashMap::new(),
                closed_streams: HashSet::new(),
                workers,
            })),
            admission,
        }
    }
    pub fn provider(&self) -> Arc<dyn TtsProvider> {
        Arc::clone(&self.provider)
    }
    pub fn begin_stream(&self) -> TtsStreamId {
        let mut state = self.state.lock().expect("TTS worker state poisoned");
        let stream = TtsStreamId(state.next_stream);
        state.next_stream += 1;
        state.streams.insert(stream, None);
        stream
    }
    pub fn close_stream(&self, stream: TtsStreamId) {
        let mut state = self.state.lock().expect("TTS worker state poisoned");
        let Some(worker) = state.streams.remove(&stream).flatten() else {
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
        self.start_internal(None, TtsWorkRequest::Voice(text), true)
    }
    pub fn start_in_stream(
        &self,
        stream: TtsStreamId,
        text: String,
    ) -> Result<TtsLease, TtsWorkerError> {
        self.start_internal(Some(stream), TtsWorkRequest::Voice(text), true)
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
        let worker = match stream {
            Some(stream) => match state
                .streams
                .get(&stream)
                .copied()
                .ok_or(TtsWorkerError::UnknownLease)?
            {
                Some(worker) if !state.workers[worker].busy => worker,
                Some(_) => return Err(TtsWorkerError::Capacity),
                None => state
                    .workers
                    .iter()
                    .position(|entry| !entry.busy && !entry.quarantined)
                    .ok_or(TtsWorkerError::Capacity)?,
            },
            None => state
                .workers
                .iter()
                .position(|entry| !entry.busy && entry.stream.is_none() && !entry.quarantined)
                .ok_or(TtsWorkerError::Capacity)?,
        };
        let lease = TtsLease(state.next);
        state.next += 1;
        let cancelled = Arc::new(AtomicBool::new(false));
        let (events_tx, events_rx) = mpsc::sync_channel(self.config.command_capacity);
        state.workers[worker].busy = true;
        if let Some(stream) = stream {
            state.workers[worker].stream = Some(stream);
            state.streams.insert(stream, Some(worker));
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
        self.provider.validate_diagnostic(request)
    }

    pub(super) fn start_diagnostic(
        &self,
        request: TtsDiagnosticRequest,
    ) -> Result<TtsLease, TtsWorkerError> {
        self.start_internal(None, TtsWorkRequest::Diagnostic(request), false)
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
impl Drop for TtsWorkerRuntime {
    fn drop(&mut self) {
        if Arc::strong_count(&self.state) == 1
            && let Ok(state) = self.state.lock()
        {
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
fn worker_loop(provider: Arc<dyn TtsProvider>, commands: mpsc::Receiver<WorkerCommand>) {
    let mut worker = provider
        .open_worker()
        .unwrap_or_else(|_| Box::new(ProviderWorker::new(provider)));
    while let Ok(command) = commands.recv() {
        match command {
            WorkerCommand::Shutdown => break,
            WorkerCommand::Reset => {
                let _ = worker.reset();
            }
            WorkerCommand::Start {
                request,
                cancelled,
                events,
            } => {
                let diagnostic = matches!(&request, TtsWorkRequest::Diagnostic(_));
                if let TtsWorkRequest::Voice(text) = &request {
                    tracing::info!(
                        tts_input = %text,
                        chars = text.chars().count(),
                        delivery = "worker",
                        "TTS synthesis input"
                    );
                }
                let started = Instant::now();
                let result = match request {
                    TtsWorkRequest::Voice(text) => {
                        worker.synthesize(&text, &cancelled, &mut |pcm| {
                            if cancelled.load(Ordering::Acquire) {
                                return Err(TtsError::Failed);
                            }
                            send_pcm_until_cancelled(&events, &cancelled, pcm)
                        })
                    }
                    TtsWorkRequest::Diagnostic(request) => {
                        worker.synthesize_diagnostic(&request, &cancelled, &mut |pcm| {
                            if cancelled.load(Ordering::Acquire) {
                                return Err(TtsError::Failed);
                            }
                            send_pcm_until_cancelled(&events, &cancelled, pcm)
                        })
                    }
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
        text: &str,
        cancelled: &AtomicBool,
        on_pcm: &mut dyn FnMut(PcmF32Mono) -> Result<(), TtsError>,
    ) -> Result<(), TtsError> {
        if cancelled.load(Ordering::Acquire) {
            return Err(TtsError::Failed);
        }
        match &mut self.stream {
            Some(stream) => stream.synthesize(text, on_pcm),
            None => self.provider.synthesize_stream(text, on_pcm),
        }
    }
    fn reset(&mut self) -> Result<(), TtsError> {
        Ok(())
    }

    fn synthesize_diagnostic(
        &mut self,
        request: &TtsDiagnosticRequest,
        cancelled: &AtomicBool,
        on_pcm: &mut dyn FnMut(PcmF32Mono) -> Result<(), TtsError>,
    ) -> Result<(), TtsError> {
        self.provider
            .synthesize_diagnostic(request, cancelled, on_pcm)
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
