use super::pool::release_terminal;
use super::*;

impl TtsWorkerRuntime {
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
            worker.reset_pending.store(true, Ordering::Release);
            let _ = worker.command_tx.try_send(WorkerCommand::Reset {
                pending: worker.reset_pending.clone(),
            });
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
    pub(super) fn start_internal(
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
                refresh_on_pcm: matches!(&request, TtsWorkRequest::Voice(_)),
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
        // Voice synthesis is streaming: a long utterance must not be cancelled while
        // it is making progress. Refresh on consumption so bounded-channel/pacing
        // backpressure is not charged as an inference stall. Diagnostics retain their
        // absolute operation budget, and cancellation never extends cleanup grace.
        if matches!(&event, TtsWorkerEvent::Pcm(pcm) if !pcm.samples().is_empty())
            && slot.refresh_on_pcm
            && slot.cleanup_deadline.is_none()
        {
            slot.deadline = Instant::now() + self.config.final_timeout;
        }
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

impl TtsWorkRequest {
    fn selection(&self) -> &TtsBinding {
        match self {
            Self::Voice(request) | Self::Diagnostic { request, .. } => &request.selection,
        }
    }
}
