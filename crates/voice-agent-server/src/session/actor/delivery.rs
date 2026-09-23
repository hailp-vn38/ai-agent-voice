use super::*;

impl SessionActor {
    pub(super) fn commit_user_text(&mut self, final_text: String) -> Option<String> {
        let text = final_text.trim();
        if text.is_empty() {
            return None;
        }
        let text = text.to_owned();
        self.dialogue_history.commit_user(text.clone());
        let payload = serde_json::json!({
            "session_id": self.session_id,
            "type": "stt",
            "text": text,
        });
        let Ok(payload) = serde_json::to_string(&payload) else {
            return None;
        };
        let _ = self.send_turn_control(payload);
        Some(text)
    }

    pub(super) fn begin_speech_delivery(&mut self, user_text: String) {
        let identity =
            WorkerIdentity::new(self.session_id.clone(), self.generation, self.generation);
        let turn = TurnContext {
            generation: self.generation,
            cancellation: tokio_util::sync::CancellationToken::new(),
        };
        let cancellation = turn.cancellation.clone();
        self.turn = Some(turn);
        self.generated_response.clear();
        if self
            .llm_runtime
            .start(identity.clone(), user_text, cancellation)
            .is_err()
        {
            self.turn = None;
            warn!("LLM operation could not start");
            self.complete_recognition();
            return;
        }
        info!("LLM operation started");
        self.llm_operation = Some(identity);
    }

    pub(super) fn on_llm_event(&mut self, event: LlmRuntimeEvent) {
        let identity = match &event {
            LlmRuntimeEvent::TextDelta { identity, .. }
            | LlmRuntimeEvent::UnexpectedToolCall { identity }
            | LlmRuntimeEvent::Finished { identity }
            | LlmRuntimeEvent::Failed { identity }
            | LlmRuntimeEvent::Cancelled { identity } => identity,
        };
        if self.llm_operation.as_ref() != Some(identity)
            || self.turn.as_ref().is_none_or(|turn| {
                turn.generation != identity.generation() || turn.generation != self.generation
            })
        {
            return;
        }
        match event {
            LlmRuntimeEvent::TextDelta { text, .. } => {
                self.generated_response.push_str(&text);
                if self.speech_output.push_delta(&text).is_err() {
                    self.fail_speech_delivery();
                }
            }
            LlmRuntimeEvent::Finished { .. } => {
                self.llm_operation = None;
                info!("LLM operation finished");
                if self.generated_response.trim().is_empty()
                    || self.speech_output.finish_input().is_err()
                {
                    self.fail_speech_delivery();
                }
            }
            LlmRuntimeEvent::UnexpectedToolCall { .. }
            | LlmRuntimeEvent::Failed { .. }
            | LlmRuntimeEvent::Cancelled { .. } => {
                self.llm_operation = None;
                warn!("LLM operation ended without a deliverable response");
                self.fail_speech_delivery();
            }
        }
    }

    pub(super) fn drain_speech_output(&mut self) {
        if !self.flush_pending_audio() {
            return;
        }
        loop {
            let event = match self.speech_output.poll() {
                Ok(Some(event)) => event,
                Ok(None) => break,
                Err(_) => {
                    self.fail_speech_delivery();
                    break;
                }
            };
            match event {
                SpeechOutputEvent::SegmentReady { text } => {
                    let payload = serde_json::json!({
                        "session_id": self.session_id,
                        "type": "llm",
                        "text": &text,
                    });
                    if serde_json::to_string(&payload)
                        .ok()
                        .is_none_or(|payload| self.send_turn_control(payload).is_err())
                    {
                        self.fail_speech_delivery();
                        break;
                    }
                }
                SpeechOutputEvent::Started => {
                    self.tts_started = true;
                    info!("TTS delivery started");
                    let payload = serde_json::json!({
                        "session_id": self.session_id,
                        "type": "tts",
                        "state": "start",
                    });
                    if serde_json::to_string(&payload)
                        .ok()
                        .is_none_or(|payload| self.send_turn_control(payload).is_err())
                    {
                        self.fail_speech_delivery();
                        break;
                    }
                }
                SpeechOutputEvent::AudioPacket(packet) => {
                    let message = OutboundMessage::Binary {
                        generation: self.generation,
                        packet,
                    };
                    match self.audio_tx.try_send(message) {
                        Ok(()) => {}
                        Err(mpsc::error::TrySendError::Full(message)) => {
                            self.pending_audio = Some(message);
                            break;
                        }
                        // A closed queue means the connection writer has already gone away;
                        // session teardown owns the resulting close path.
                        Err(mpsc::error::TrySendError::Closed(_)) => break,
                    }
                }
                SpeechOutputEvent::Drained => {
                    info!("TTS delivery drained");
                    if self.tts_started {
                        let payload = serde_json::json!({
                            "session_id": self.session_id,
                            "type": "tts",
                            "state": "stop",
                        });
                        if let Ok(payload) = serde_json::to_string(&payload) {
                            let _ = self.send_turn_control(payload);
                        }
                    }
                    // A Delivered Assistant Response exists only after all its audio drained.
                    self.dialogue_history
                        .commit_assistant(std::mem::take(&mut self.generated_response));
                    self.tts_started = false;
                    self.turn = None;
                    self.complete_recognition();
                }
            }
        }
    }

    pub(super) fn fail_speech_delivery(&mut self) {
        self.cancel_llm();
        self.cancel_speech_delivery();
        self.complete_recognition();
    }

    /// Cancels a Conversational Turn in its required order. The outbound gate is
    /// invalidated before producer cancellation, and releasing the Active Turn is
    /// idempotent through its permit ownership flag.
    pub(super) fn interrupt_active_turn(&mut self) {
        self.cancel_speech_delivery();
        self.cancel_llm();
        self.cancel_asr();
        self.release_active_turn();
    }

    pub(super) fn cancel_speech_delivery(&mut self) {
        // This is the outbound linearization point for the current Conversational Turn.
        // It must precede every producer cancellation, including cancellation before TTS starts:
        // `llm` or `tts:start` may already be waiting at the writer.
        self.generation_gate.invalidate(self.generation);
        self.speech_output.cancel();
        self.pending_audio = None;
        if self.tts_started {
            self.tts_started = false;
            let payload = serde_json::json!({
                "session_id": self.session_id,
                "type": "tts",
                "state": "stop",
            });
            if let Ok(payload) = serde_json::to_string(&payload) {
                if self
                    .urgent_tx
                    .try_send(OutboundMessage::Text(payload))
                    .is_err()
                {
                    // Continuing playback without an admitted stop leaves the client state
                    // unknowable.  Tear the Voice Session down instead of silently retrying.
                    self.fail_closed_after_urgent_stop_admission_failure();
                }
            }
        }
    }

    /// Returns false while the writer is backpressured. Keeping exactly one pending packet
    /// bounds memory and, together with SpeechOutput's own queue, preserves packet order.
    pub(super) fn flush_pending_audio(&mut self) -> bool {
        let Some(message) = self.pending_audio.take() else {
            return true;
        };
        match self.audio_tx.try_send(message) {
            Ok(()) => true,
            Err(mpsc::error::TrySendError::Full(message)) => {
                self.pending_audio = Some(message);
                false
            }
            Err(mpsc::error::TrySendError::Closed(_)) => false,
        }
    }

    pub(super) fn cancel_asr(&mut self) {
        if let Some((lease, _)) = self.asr_stream {
            let _ = self.asr_runtime.send(lease, AsrCommand::Cancel);
        }
    }

    pub(super) fn cancel_llm(&mut self) {
        if let Some(turn) = &self.turn {
            turn.cancellation.cancel();
        }
        if let Some(identity) = self.llm_operation.take() {
            self.llm_runtime.cancel(&identity);
        }
        self.generated_response.clear();
        self.turn = None;
    }

    pub(super) fn release_active_turn(&mut self) {
        if self.has_active_turn_permit {
            self.active_turn_limiter.release();
            self.has_active_turn_permit = false;
        }
    }

    pub(super) fn close_vad(&mut self) {
        if let Some((lease, _)) = self.vad_session {
            let _ = self.vad_runtime.send(lease, VadCommand::Close);
        }
    }

    pub(super) fn push_asr(&mut self, pcm: PcmF32Mono) -> Result<(), ()> {
        let Some((lease, _)) = self.asr_stream else {
            return Err(());
        };
        self.asr_runtime
            .send(lease, AsrCommand::Push(pcm))
            .map_err(|_| ())
    }
}
