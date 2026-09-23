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

use super::WorkerRuntimeConfig;
use crate::{
    audio::PcmF32Mono,
    providers::{TtsError, TtsProvider, TtsStream, TtsWorker},
};

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
        text: String,
        cancelled: Arc<AtomicBool>,
        events: mpsc::SyncSender<TtsWorkerEvent>,
    },
    Reset,
    Shutdown,
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
    config: WorkerRuntimeConfig,
    state: Arc<Mutex<State>>,
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
        Self {
            config,
            state: Arc::new(Mutex::new(State {
                next: 1,
                next_stream: 1,
                slots: HashMap::new(),
                streams: HashMap::new(),
                closed_streams: HashSet::new(),
                workers,
            })),
        }
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
        self.start_internal(None, text)
    }
    pub fn start_in_stream(
        &self,
        stream: TtsStreamId,
        text: String,
    ) -> Result<TtsLease, TtsWorkerError> {
        self.start_internal(Some(stream), text)
    }
    fn start_internal(
        &self,
        stream: Option<TtsStreamId>,
        text: String,
    ) -> Result<TtsLease, TtsWorkerError> {
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
            },
        );
        let command = state.workers[worker].command_tx.clone();
        if command
            .send(WorkerCommand::Start {
                text,
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
                text,
                cancelled,
                events,
            } => {
                tracing::info!(
                    tts_input = %text,
                    chars = text.chars().count(),
                    delivery = "worker",
                    "TTS synthesis input"
                );
                let result = worker.synthesize(&text, &cancelled, &mut |pcm| {
                    if cancelled.load(Ordering::Acquire) {
                        return Err(TtsError::Failed);
                    }
                    send_pcm_until_cancelled(&events, &cancelled, pcm)
                });
                let event = if cancelled.load(Ordering::Acquire) {
                    TtsWorkerEvent::Cancelled
                } else if result.is_ok() {
                    TtsWorkerEvent::Finished
                } else {
                    TtsWorkerEvent::Failed
                };
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
}
