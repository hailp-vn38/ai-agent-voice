use super::*;

impl SessionActor {
    /// Deterministic test helper: advances the runtime router then drains this session mailbox.
    pub fn pump_workers(&mut self) {
        self.asr_runtime.supervise_pending();
        self.vad_runtime.supervise_pending();
        self.drain_worker_events();
        self.drain_speech_output();
    }

    pub(super) fn drain_worker_events(&mut self) {
        while let Ok(event) = self.asr_events.try_recv() {
            self.on_asr_event(event);
        }
        while let Ok(event) = self.vad_events.try_recv() {
            self.on_vad_event(event);
        }
        while let Ok(event) = self.llm_events.try_recv() {
            self.on_llm_event(event);
            self.drain_speech_output();
        }
    }

    pub fn send_control(
        &self,
        text: String,
    ) -> Result<(), mpsc::error::TrySendError<OutboundMessage>> {
        self.control_tx.try_send(OutboundMessage::Text(text))
    }

    pub(super) fn send_turn_control(
        &self,
        text: String,
    ) -> Result<(), mpsc::error::TrySendError<OutboundMessage>> {
        self.control_tx.try_send(OutboundMessage::TurnText {
            generation: self.generation,
            text,
        })
    }

    pub async fn run(mut self, mut ingress: mpsc::Receiver<SessionEvent>) {
        let mut worker_tick = tokio::time::interval(std::time::Duration::from_millis(1));
        loop {
            tokio::select! {
                _ = worker_tick.tick() => {
                    self.drain_worker_events();
                    self.drain_speech_output();
                }
                event = ingress.recv() => match event {
                    Some(SessionEvent::ClientMessage(message)) => self.on_client_message(message),
                    Some(SessionEvent::ClientAudio(payload)) => { self.on_binary(payload); }
                    None => break,
                }
            }
        }
        self.phase = SessionPhase::Closed;
    }

    pub fn on_client_message(&mut self, message: ClientMessage) {
        match message {
            ClientMessage::Listen {
                session_id,
                command,
            } => {
                if self.inbound_session_matches(session_id.as_deref()) {
                    self.on_listen_command(command);
                }
            }
            ClientMessage::Abort { session_id } => {
                if self.inbound_session_matches(session_id.as_deref()) {
                    self.abort_current_turn();
                }
            }
            ClientMessage::Hello(_) | ClientMessage::Unknown => {}
        }
    }

    pub(super) fn inbound_session_matches(&self, session_id: Option<&str>) -> bool {
        matches!(session_id, None | Some("")) || session_id == Some(&self.session_id)
    }
}

impl SessionActor {
    pub fn on_binary(&mut self, payload: Vec<u8>) -> bool {
        if self.phase == SessionPhase::Listening {
            match self.uplink_decoder.decode(&payload) {
                DecodeOutcome::Frame(frame) => {
                    let pcm = PcmF32Mono::from_uplink(&frame);
                    if self.listening_mode == Some(ListenMode::Auto) {
                        if self.auto_reset_pending {
                            return false;
                        }
                        let Some((lease, _)) = self.vad_session else {
                            return false;
                        };
                        self.auto_retention.push(&pcm);
                        match self.vad_runtime.send(lease, VadCommand::Push(pcm.clone())) {
                            Ok(()) => {}
                            Err(crate::workers::VadWorkerError::QueueFull) => {
                                self.fail_closed();
                                return false;
                            }
                            Err(_) => {
                                self.fail_closed();
                                return false;
                            }
                        }
                        if self.auto_speech_active && self.push_asr(pcm).is_err() {
                            self.cancel_asr();
                            // Recognition is terminally invalid once a Push was not admitted;
                            // wait for worker cleanup, but do not attempt Finish at SpeechEnd.
                            self.asr_stream = None;
                        }
                        self.accepted_binary_frames += 1;
                        return true;
                    }
                    if !self.manual_capture.push(frame.clone()) {
                        self.cancel_asr();
                        return false;
                    }
                    let Some((lease, _)) = self.asr_stream else {
                        self.phase = SessionPhase::Ready;
                        return false;
                    };
                    if self.asr_runtime.send(lease, AsrCommand::Push(pcm)).is_err() {
                        self.cancel_asr();
                        self.asr_stream = None;
                        self.manual_capture.abort();
                        self.phase = SessionPhase::Ready;
                        return false;
                    }
                    self.accepted_binary_frames += 1;
                    true
                }
                DecodeOutcome::Dropped(reason) => {
                    tracing::debug!(
                        event = "audio_frame_dropped",
                        ?reason,
                        "audio frame dropped"
                    );
                    false
                }
            }
        } else {
            false
        }
    }
}
