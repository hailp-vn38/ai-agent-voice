use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread,
    time::Instant,
};

use crate::{
    audio::PcmF32Mono,
    providers::{TtsError, TtsProvider, TtsStream, TtsSynthesisRequest, TtsWorker},
};

use super::*;

pub(super) struct TtsPoolOwner(pub(super) Arc<Mutex<State>>);
impl Drop for TtsPoolOwner {
    fn drop(&mut self) {
        if let Ok(state) = self.0.lock() {
            for worker in &state.workers {
                let _ = worker.command_tx.try_send(WorkerCommand::Shutdown);
            }
        }
    }
}
pub(super) fn release_terminal(state: &mut State, lease: TtsLease) {
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
        worker.reset_pending.store(true, Ordering::Release);
        let _ = worker.command_tx.try_send(WorkerCommand::Reset {
            pending: worker.reset_pending.clone(),
        });
    }
}
pub(super) fn stop_and_join(workers: &mut [WorkerRecord]) -> bool {
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

pub(super) fn worker_loop(
    provider: Arc<dyn TtsProvider>,
    commands: mpsc::Receiver<WorkerCommand>,
    ready: mpsc::SyncSender<Option<super::super::NativeReadiness>>,
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
        .send(Some(super::super::NativeReadiness {
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
            WorkerCommand::Reset { pending } => {
                if worker.reset().is_err() {
                    healthy.store(false, Ordering::Release);
                    break;
                }
                pending.store(false, Ordering::Release);
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
