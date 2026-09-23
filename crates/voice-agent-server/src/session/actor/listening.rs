use super::*;

impl SessionActor {
    pub(super) fn on_listen_command(&mut self, command: ListenCommand) {
        match command {
            ListenCommand::Start { mode } => self.start_listening(mode),
            ListenCommand::Detect { text } => self.accept_detect(text),
            ListenCommand::Stop => {
                if self.listening_mode == Some(ListenMode::Manual)
                    && self.phase == SessionPhase::Listening
                {
                    let outcome = self.manual_capture.stop();
                    if let CaptureOutcome::Utterance(_) = outcome {
                        self.phase = SessionPhase::Processing;
                        self.finish_manual();
                    } else {
                        self.cancel_asr();
                        self.phase = SessionPhase::Ready;
                    }
                }
            }
        }
    }

    pub(super) fn start_listening(&mut self, mode: ListenMode) {
        if matches!(mode, ListenMode::Auto | ListenMode::Realtime)
            && self.listening_mode.as_ref() == Some(&mode)
            && self.vad_session.is_some()
        {
            self.restart_existing_vad_capture_cycle();
            return;
        }
        if self.turn.is_some()
            || matches!(
                self.phase,
                SessionPhase::Processing | SessionPhase::Speaking
            )
        {
            // Capture arm is intentionally separate from interruption. Phase 5's
            // AEC/VAD work decides when this arm may consume microphone PCM; it
            // must never cancel an in-flight response merely by changing mode.
            self.listening_mode = Some(mode);
            self.listen_arm_pending = true;
            return;
        }
        self.replace_listening_mode(mode);
    }

    /// Enters a new listening mode. Leaving Auto is a real VAD lifecycle boundary; the
    /// acknowledgement-driven Close path owns release of its worker capacity.
    pub(super) fn replace_listening_mode(&mut self, mode: ListenMode) {
        self.listen_arm_pending = false;
        self.cancel_speech_delivery();
        self.generation += 1;
        self.cancel_llm();
        self.cancel_asr();
        self.release_active_turn();
        self.close_vad();
        self.auto_speech_active = false;
        self.auto_reset_pending = false;
        self.vad_cycle = None;
        self.pending_vad_cycle = None;
        self.auto_retention.reset();
        self.vad_segmenter.reset();
        self.listening_mode = Some(mode.clone());
        let identity =
            WorkerIdentity::new(self.session_id.clone(), self.generation, self.generation);
        match mode {
            ListenMode::Manual => match self.asr_runtime.open(identity.clone()) {
                Ok(lease) => {
                    self.manual_capture.restart();
                    self.asr_stream = Some((lease, identity));
                    self.phase = SessionPhase::Listening;
                }
                Err(error) => {
                    warn!(?error, "manual ASR start rejected");
                    self.phase = SessionPhase::Ready;
                }
            },
            ListenMode::Auto => match self.vad_runtime.open(identity.clone()) {
                Ok(lease) => {
                    self.vad_session = Some((lease, identity));
                    self.vad_cycle = Some(self.allocate_vad_cycle());
                    self.phase = SessionPhase::Listening;
                }
                Err(error) => {
                    warn!(?error, phase = ?self.phase, generation = self.generation, "auto VAD worker open failed");
                    self.phase = SessionPhase::Closed;
                    let _ = self.urgent_tx.try_send(OutboundMessage::Close(1013));
                }
            },
            ListenMode::Realtime => match self.vad_runtime.open(identity.clone()) {
                Ok(lease) => {
                    self.vad_session = Some((lease, identity));
                    self.vad_cycle = Some(self.allocate_vad_cycle());
                    // Realtime keeps its VAD capture cycle armed across processing
                    // and playback. It is not an interruption path in this ticket.
                    self.phase = SessionPhase::Listening;
                }
                Err(error) => {
                    warn!(?error, "realtime VAD worker open failed");
                    self.phase = SessionPhase::Closed;
                    let _ = self.urgent_tx.try_send(OutboundMessage::Close(1013));
                }
            },
        }
    }

    /// Restarts capture inside an Auto or Realtime cycle without replacing its pinned VAD worker
    /// or interrupting a Conversational Turn that may be speaking.
    pub(super) fn restart_existing_vad_capture_cycle(&mut self) {
        let Some((lease, _)) = self.vad_session else {
            return;
        };
        let was_speaking = self.phase == SessionPhase::Speaking;
        if self.auto_reset_pending {
            info!(
                generation = self.generation,
                "duplicate auto listen:start accepted while VAD reset is pending"
            );
            return;
        }

        if let Some((asr_lease, asr_identity)) = self.asr_stream.take() {
            if self
                .asr_runtime
                .send(asr_lease, AsrCommand::Cancel)
                .is_err()
            {
                self.fail_closed();
                return;
            }
            self.asr_cleanup_pending.insert(asr_identity);
            self.release_active_turn();
        }
        self.auto_speech_active = false;
        self.auto_retention.reset();
        self.vad_segmenter.reset();
        match self.reset_vad_capture_cycle(lease) {
            Ok(()) => {
                self.auto_reset_pending = true;
                if !was_speaking {
                    self.phase = SessionPhase::Processing;
                }
                info!(generation = self.generation, "reusing VAD capture cycle");
            }
            Err(error) => {
                warn!(?error, "failed to reset existing Auto VAD cycle");
                self.fail_closed();
            }
        }
    }

    pub(super) fn abort_current_turn(&mut self) {
        if self.phase == SessionPhase::Closed {
            return;
        }
        self.interrupt_active_turn();
        self.generation += 1;
        self.manual_capture.abort();
        if matches!(
            self.listening_mode,
            Some(ListenMode::Auto | ListenMode::Realtime)
        ) && self.vad_session.is_some()
        {
            self.abort_auto_turn();
            return;
        }
        self.close_vad();
        self.listening_mode = None;
        self.listen_arm_pending = false;
        self.auto_reset_pending = false;
        self.auto_retention.reset();
        self.phase = SessionPhase::Ready;
    }

    pub(super) fn abort_auto_turn(&mut self) {
        self.auto_speech_active = false;
        self.auto_retention.reset();
        self.vad_segmenter.reset();
        if self.auto_reset_pending {
            return;
        }
        let Some((lease, _)) = self.vad_session else {
            self.phase = SessionPhase::Ready;
            return;
        };
        match self.reset_vad_capture_cycle(lease) {
            Ok(()) => {
                self.auto_reset_pending = true;
                self.phase = SessionPhase::Processing;
            }
            Err(error) => {
                warn!(?error, "Auto VAD reset failed after abort");
                self.fail_closed();
            }
        }
    }

    pub(super) fn accept_detect(&mut self, input: String) {
        let auto_rearming = self.listening_mode == Some(ListenMode::Auto)
            && self.auto_reset_pending
            && self.phase == SessionPhase::Processing;
        if self.phase != SessionPhase::Listening && !auto_rearming {
            return;
        }
        let Some(text) = normalize_detect_text(input) else {
            return;
        };
        info!(mode = ?self.listening_mode, "typed detect accepted");
        self.manual_capture.abort();
        match self.listening_mode {
            Some(ListenMode::Manual) => {
                if !self.detach_asr_for_detect() {
                    return;
                }
            }
            // `digital-human` starts an Auto cycle, then sends typed text through detect.
            // Revoke ASR semantic ownership but retain the VAD lease. Drained will reset it
            // and return this client to Listening for its next typed or voice turn.
            Some(ListenMode::Auto) => {
                self.auto_speech_active = false;
                self.auto_retention.reset();
                if self.asr_stream.is_some() && !self.detach_asr_for_detect() {
                    return;
                }
            }
            Some(ListenMode::Realtime) | None => return,
        }
        if !self.active_turn_limiter.try_acquire() {
            self.phase = SessionPhase::Ready;
            return;
        }
        self.has_active_turn_permit = true;
        self.phase = SessionPhase::Processing;
        if let Some(text) = self.commit_user_text(text) {
            self.begin_speech_delivery(text);
        } else {
            self.release_active_turn();
            self.phase = SessionPhase::Ready;
        }
    }

    pub(super) fn detach_asr_for_detect(&mut self) -> bool {
        let Some((lease, identity)) = self.asr_stream.take() else {
            self.fail_closed();
            return false;
        };
        if self.asr_runtime.send(lease, AsrCommand::Cancel).is_err() {
            self.fail_closed();
            return false;
        }
        self.asr_cleanup_pending.insert(identity);
        true
    }

    pub(super) fn finish_manual(&mut self) {
        if !self.active_turn_limiter.try_acquire() {
            self.cancel_asr();
            self.complete_recognition();
            return;
        }
        self.has_active_turn_permit = true;
        let Some((lease, _)) = self.asr_stream else {
            self.release_active_turn();
            return;
        };
        if self.asr_runtime.send(lease, AsrCommand::Finish).is_err() {
            self.cancel_asr();
            self.release_active_turn();
            self.phase = SessionPhase::Ready;
        }
    }

    pub(super) fn on_asr_event(&mut self, event: AsrWorkerEvent) {
        let identity = match &event {
            AsrWorkerEvent::Opened { identity }
            | AsrWorkerEvent::Final { identity, .. }
            | AsrWorkerEvent::Failed { identity }
            | AsrWorkerEvent::Cancelled { identity }
            | AsrWorkerEvent::FinalTimedOut { identity }
            | AsrWorkerEvent::CleanupTimedOut { identity } => identity,
        };
        if self.asr_cleanup_pending.contains(identity) {
            match event {
                AsrWorkerEvent::Final { .. }
                | AsrWorkerEvent::Failed { .. }
                | AsrWorkerEvent::Cancelled { .. } => {
                    self.asr_cleanup_pending.remove(identity);
                }
                AsrWorkerEvent::FinalTimedOut { .. } | AsrWorkerEvent::CleanupTimedOut { .. } => {
                    self.fail_closed();
                }
                AsrWorkerEvent::Opened { .. } => {}
            }
            return;
        }
        let current = self
            .asr_stream
            .as_ref()
            .is_some_and(|(_, current)| current == identity)
            && identity.generation() == self.generation;
        match event {
            AsrWorkerEvent::Final { text, .. } if current => {
                self.asr_stream = None;
                if let Some(final_text) = self.commit_user_text(text) {
                    self.begin_speech_delivery(final_text);
                } else {
                    self.complete_recognition();
                }
            }
            AsrWorkerEvent::Failed { .. } if current => {
                self.asr_stream = None;
                self.complete_recognition();
            }
            AsrWorkerEvent::FinalTimedOut { .. } | AsrWorkerEvent::CleanupTimedOut { .. }
                if current =>
            {
                self.fail_closed()
            }
            AsrWorkerEvent::Cancelled { .. } if current => self.asr_stream = None,
            _ => {}
        }
    }

    pub(super) fn on_vad_event(&mut self, event: VadWorkerEvent) {
        let identity = match &event {
            VadWorkerEvent::Opened { identity }
            | VadWorkerEvent::Probability { identity, .. }
            | VadWorkerEvent::SpeechStart { identity, .. }
            | VadWorkerEvent::SpeechEnd { identity, .. }
            | VadWorkerEvent::ResetDone { identity, .. }
            | VadWorkerEvent::Closed { identity }
            | VadWorkerEvent::Failed { identity }
            | VadWorkerEvent::ResetTimedOut { identity }
            | VadWorkerEvent::CleanupTimedOut { identity } => identity,
        };
        let current_worker = self
            .vad_session
            .as_ref()
            .is_some_and(|(_, active)| active == identity);
        let current_cycle = |cycle| current_worker && self.vad_cycle == Some(cycle);
        match event {
            VadWorkerEvent::Probability {
                cycle, probability, ..
            } if current_cycle(cycle) => match self.vad_segmenter.observe(probability) {
                Ok(Some(VadBoundary::SpeechStart { start_sample })) => {
                    self.on_vad_event(VadWorkerEvent::SpeechStart {
                        identity: identity.clone(),
                        cycle,
                        start_sample,
                    })
                }
                Ok(Some(VadBoundary::SpeechEnd { end_sample })) => {
                    self.on_vad_event(VadWorkerEvent::SpeechEnd {
                        identity: identity.clone(),
                        cycle,
                        end_sample,
                    })
                }
                Ok(None) => {}
                Err(_) => {
                    warn!("VAD stream integrity failure");
                    self.fail_closed();
                }
            },
            VadWorkerEvent::SpeechStart {
                cycle,
                start_sample,
                ..
            } if current_cycle(cycle) && !self.auto_speech_active => {
                // Realtime remains VAD-armed while an existing turn is processing or
                // speaking, but ticket 04 does not yet permit acoustic interruption.
                if self.turn.is_some() {
                    return;
                }
                self.auto_speech_active = true;
                let identity =
                    WorkerIdentity::new(self.session_id.clone(), self.generation, self.generation);
                match self.asr_runtime.open(identity.clone()) {
                    Ok(lease) => {
                        self.asr_stream = Some((lease, identity));
                        let feed_start = start_sample.saturating_sub(self.pre_roll_samples);
                        let Some(retained) = self.auto_retention.range(feed_start) else {
                            self.fail_closed();
                            return;
                        };
                        if self.push_asr(retained).is_err() {
                            self.cancel_asr();
                            self.asr_stream = None;
                        }
                    }
                    Err(_) => self.auto_retention.reset(),
                }
            }
            VadWorkerEvent::SpeechEnd { cycle, .. }
                if current_cycle(cycle) && self.auto_speech_active =>
            {
                self.auto_speech_active = false;
                self.phase = SessionPhase::Processing;
                if self.asr_stream.is_some() {
                    self.finish_manual();
                } else {
                    // ASR capacity/queue denial has no transcript, but the VAD cycle remains
                    // usable after its reset acknowledgement.
                    self.complete_recognition();
                }
            }
            VadWorkerEvent::ResetDone { cycle, .. }
                if current_worker && self.pending_vad_cycle == Some(cycle) =>
            {
                self.auto_reset_pending = false;
                self.vad_cycle = Some(cycle);
                self.pending_vad_cycle = None;
                self.auto_retention.reset();
                self.vad_segmenter.reset();
                if self.llm_operation.is_none() && !self.tts_started {
                    self.phase = SessionPhase::Listening;
                }
            }
            VadWorkerEvent::Closed { .. } if current_worker => self.vad_session = None,
            VadWorkerEvent::Failed { .. }
            | VadWorkerEvent::ResetTimedOut { .. }
            | VadWorkerEvent::CleanupTimedOut { .. }
                if current_worker =>
            {
                warn!("VAD worker failed or cleanup timed out");
                self.fail_closed()
            }
            _ => {}
        }
    }

    pub(super) fn allocate_vad_cycle(&mut self) -> VadCaptureCycleId {
        let cycle = VadCaptureCycleId::new(self.next_vad_cycle);
        self.next_vad_cycle = self.next_vad_cycle.saturating_add(1);
        cycle
    }

    pub(super) fn reset_vad_capture_cycle(
        &mut self,
        lease: VadWorkerLease,
    ) -> Result<(), crate::workers::VadWorkerError> {
        let cycle = self.allocate_vad_cycle();
        self.vad_cycle = None;
        self.pending_vad_cycle = Some(cycle);
        self.vad_runtime.send(lease, VadCommand::Reset { cycle })
    }
}
