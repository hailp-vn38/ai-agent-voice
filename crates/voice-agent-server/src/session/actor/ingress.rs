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
        self.drain_writer_events();
        self.expire_mcp_requests();
        while let Ok(event) = self.asr_events.try_recv() {
            self.on_asr_event(event);
        }
        while let Ok(event) = self.vad_events.try_recv() {
            self.on_vad_event(event);
        }
        self.flush_pending_llm_text();
        while self.pending_llm_delta.is_none() && !self.llm_finish_pending {
            let Ok(event) = self.llm_events.try_recv() else {
                break;
            };
            self.on_llm_event(event);
            self.drain_speech_output();
            self.flush_pending_llm_text();
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
        let Some(turn_id) = self.current_turn_id() else {
            return Err(mpsc::error::TrySendError::Closed(OutboundMessage::Text(
                text,
            )));
        };
        self.control_tx.try_send(OutboundMessage::TurnText {
            generation: self.generation,
            turn_id,
            text,
        })
    }

    pub(super) fn drain_writer_events(&mut self) {
        loop {
            let event = match self.writer_events.as_mut() {
                Some(events) => events.try_recv().ok(),
                None => None,
            };
            let Some(event) = event else { break };
            self.on_writer_event(event);
        }
    }

    pub(super) fn on_writer_event(&mut self, event: WriterEvent) {
        match event {
            WriterEvent::TurnClosed { turn_id, outcome } => {
                if self.current_turn_id() != Some(turn_id) {
                    return;
                }
                if let Some(delivery) = self.pending_delivery.take() {
                    if delivery.turn_id != turn_id {
                        self.pending_delivery = Some(delivery);
                        return;
                    }
                    if matches!(outcome, WriterTurnOutcome::Normal) {
                        self.dialogue_history
                            .commit_assistant(turn_id, delivery.assistant_text);
                    }
                }
                self.tts_started = false;
                self.release_active_turn();
                self.complete_recognition();
            }
            WriterEvent::Failed { .. } => self.fail_closed(),
        }
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
                    info!(phase = ?self.phase, generation = self.generation, "Client abort accepted");
                    self.abort_active_interaction();
                }
            }
            ClientMessage::Mcp {
                session_id,
                payload,
            } => {
                if self.inbound_session_matches(session_id.as_deref()) {
                    self.on_mcp_message(payload);
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
        let armed_vad_capture = self.vad_session.is_some()
            && !self.auto_reset_pending
            && self.phase == SessionPhase::Speaking
            && self.acoustic_barge_in_allowed();
        if self.phase == SessionPhase::Listening || armed_vad_capture {
            match self.uplink_decoder.decode(&payload) {
                DecodeOutcome::Frame(frame) => {
                    let pcm = PcmF32Mono::from_uplink(&frame);
                    if matches!(
                        self.listening_mode,
                        Some(ListenMode::Auto | ListenMode::Realtime)
                    ) {
                        if self.auto_reset_pending {
                            return false;
                        }
                        let Some((lease, _)) = self.vad_session else {
                            return false;
                        };
                        let Some(cycle) = self.vad_cycle else {
                            return false;
                        };
                        self.auto_retention.push(&pcm);
                        match self.vad_runtime.send(
                            lease,
                            VadCommand::Push {
                                cycle,
                                pcm: pcm.clone(),
                            },
                        ) {
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
