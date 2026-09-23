use super::*;
use crate::providers::llm::ToolDefinition;

impl SessionActor {
    pub(super) fn commit_user_text(&mut self, final_text: String) -> Option<String> {
        let text = final_text.trim();
        if text.is_empty() {
            return None;
        }
        let text = text.to_owned();
        let turn_id = self.current_turn_id()?;
        self.dialogue_history.commit_user(turn_id, text.clone());
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
        let Some(turn_id) = self.current_turn_id() else {
            self.fail_closed();
            return;
        };
        let Some(history) = self.dialogue_history.messages_for_prompt(turn_id) else {
            self.terminalize_llm_failure();
            return;
        };
        debug_assert!(
            matches!(history.last(), Some(ChatMessage::User { content }) if content == &user_text)
        );
        self.llm_messages = Vec::with_capacity(history.len() + 1);
        self.llm_messages.push(ChatMessage::System {
            content: self.system_prompt.clone(),
        });
        self.llm_messages.extend(history);
        self.tool_depth = 0;
        self.start_llm_round(true);
    }

    pub(super) fn begin_tool_continuation(&mut self) {
        if self.tool_depth >= self.max_tool_depth {
            self.terminalize_turn_failure(TurnFailure::ToolDepthExceeded);
            return;
        }
        self.tool_depth += 1;
        tracing::info!(
            event = "llm_tool_continuation_started",
            tool_depth = self.tool_depth,
            "LLM tool continuation started"
        );
        self.start_llm_round(true);
    }

    pub(super) fn begin_direct_tool_speech(&mut self, text: String) {
        self.generated_response = text.clone();
        self.pending_llm_delta = Some((text, 0));
        self.llm_finish_pending = true;
        self.flush_pending_llm_text();
    }

    fn start_llm_round(&mut self, allow_tools: bool) {
        let Some(cancellation) = self.turn.as_ref().map(|turn| turn.cancellation.clone()) else {
            self.fail_closed();
            return;
        };
        let Some(identity) = self.next_worker_identity() else {
            self.fail_closed();
            return;
        };
        self.generated_response.clear();
        self.pending_llm_delta = None;
        self.llm_finish_pending = false;
        self.llm_round =
            (allow_tools && !self.mcp.visible.is_empty()).then(LlmRoundBuffer::default);
        let request = crate::providers::llm::LlmRequest {
            messages: self.llm_messages.clone(),
            tools: if allow_tools {
                self.mcp
                    .visible
                    .iter()
                    .map(|tool| ToolDefinition {
                        name: tool.llm_name.clone(),
                        description: tool.description.clone(),
                        parameters: tool.input_schema.clone(),
                    })
                    .collect()
            } else {
                Vec::new()
            },
        };
        if crate::session::prompt::llm_request_size_bytes(&request).is_err() {
            self.terminalize_turn_failure(TurnFailure::LlmRequestTooLarge);
            return;
        }
        if self
            .llm_runtime
            .start(identity.clone(), request, cancellation)
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

    fn terminalize_llm_failure(&mut self) {
        self.llm_operation = None;
        self.pending_llm_delta = None;
        self.llm_finish_pending = false;
        self.generated_response.clear();
        self.complete_recognition();
    }

    fn terminalize_turn_failure(&mut self, failure: TurnFailure) {
        warn!(code = failure.code(), "conversational turn terminalized");
        self.terminalize_llm_failure();
    }

    pub(super) fn on_llm_event(&mut self, event: LlmRuntimeEvent) {
        let identity = match &event {
            LlmRuntimeEvent::TextDelta { identity, .. }
            | LlmRuntimeEvent::ToolCall { identity, .. }
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
                if self.llm_round.is_some() {
                    self.llm_round
                        .as_mut()
                        .expect("checked")
                        .prose
                        .push_str(&text);
                    return;
                }
                self.generated_response.push_str(&text);
                self.pending_llm_delta = Some((text, 0));
                self.flush_pending_llm_text();
            }
            LlmRuntimeEvent::Finished { .. } => {
                self.llm_operation = None;
                if let Some(round) = self.llm_round.take() {
                    if !round.calls.is_empty() {
                        self.start_tool_batch(round.calls);
                        return;
                    }
                    tracing::info!(
                        event = "llm_final_round_finished",
                        "LLM final no-tool round finished"
                    );
                    self.generated_response = round.prose.clone();
                    self.pending_llm_delta = Some((round.prose, 0));
                }
                info!("LLM operation finished");
                self.llm_finish_pending = true;
                self.flush_pending_llm_text();
            }
            LlmRuntimeEvent::ToolCall { call, .. } => {
                let Some(round) = self.llm_round.as_mut() else {
                    self.fail_speech_delivery();
                    return;
                };
                round.calls.push(call);
            }
            LlmRuntimeEvent::Failed { .. } | LlmRuntimeEvent::Cancelled { .. } => {
                self.llm_operation = None;
                warn!("LLM operation ended without a deliverable response");
                self.fail_speech_delivery();
            }
        }
    }

    /// Keep at most one LLM delta outside SpeechOutput. Pausing mailbox reads propagates
    /// pressure through the bounded LLM route instead of cancelling a healthy spoken turn.
    pub(super) fn flush_pending_llm_text(&mut self) {
        while self.speech_output.has_pending_capacity() {
            let Some((text, offset)) = self.pending_llm_delta.as_mut() else {
                break;
            };
            if *offset == text.len() {
                self.pending_llm_delta = None;
                break;
            }
            let next = text[*offset..]
                .chars()
                .next()
                .expect("offset is a character boundary");
            let end = *offset + next.len_utf8();
            if let Err(error) = self.speech_output.push_delta(&text[*offset..end]) {
                warn!(?error, "LLM text could not enter speech output");
                self.fail_speech_delivery();
                return;
            }
            *offset = end;
        }
        if self
            .pending_llm_delta
            .as_ref()
            .is_some_and(|(text, offset)| *offset == text.len())
        {
            self.pending_llm_delta = None;
        }
        if self.llm_finish_pending
            && self.pending_llm_delta.is_none()
            && self.speech_output.has_pending_capacity()
        {
            self.llm_finish_pending = false;
            if self.generated_response.trim().is_empty() {
                warn!("LLM response has no deliverable text");
                self.fail_speech_delivery();
            } else if let Err(error) = self.speech_output.finish_input() {
                warn!(?error, "LLM response could not finish speech input");
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
                Err(error) => {
                    warn!(?error, "TTS delivery failed");
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
                        warn!("LLM segment control could not enter outbound queue");
                        self.fail_speech_delivery();
                        break;
                    }
                }
                SpeechOutputEvent::Started => {
                    self.tts_started = true;
                    self.phase = SessionPhase::Speaking;
                    info!("TTS delivery started");
                    let Some(turn_id) = self.current_turn_id() else {
                        self.fail_speech_delivery();
                        break;
                    };
                    let payload = serde_json::json!({
                        "session_id": self.session_id,
                        "type": "tts",
                        "state": "start",
                    });
                    if serde_json::to_string(&payload).ok().is_none_or(|payload| {
                        self.control_tx
                            .try_send(OutboundMessage::BeginTurn {
                                generation: self.generation,
                                turn_id,
                                text: payload,
                            })
                            .is_err()
                    }) {
                        warn!("TTS start control could not enter outbound queue");
                        self.fail_speech_delivery();
                        break;
                    }
                }
                SpeechOutputEvent::AudioPacket(packet) => {
                    let Some(turn_id) = self.current_turn_id() else {
                        self.fail_speech_delivery();
                        break;
                    };
                    let message = OutboundMessage::Binary {
                        generation: self.generation,
                        turn_id,
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
                    let Some(turn_id) = self.current_turn_id() else {
                        self.fail_speech_delivery();
                        break;
                    };
                    let payload = serde_json::json!({
                        "session_id": self.session_id,
                        "type": "tts",
                        "state": "stop",
                    });
                    let Some(payload) = serde_json::to_string(&payload).ok() else {
                        self.fail_speech_delivery();
                        break;
                    };
                    if self
                        .control_tx
                        .try_send(OutboundMessage::FinishTurn {
                            turn_id,
                            text: payload,
                        })
                        .is_err()
                    {
                        self.fail_closed();
                        break;
                    }
                    self.pending_delivery = Some(PendingDelivery {
                        turn_id,
                        assistant_text: std::mem::take(&mut self.generated_response),
                    });
                    if self.writer_events.is_none() {
                        self.on_writer_event(WriterEvent::TurnClosed {
                            turn_id,
                            outcome: WriterTurnOutcome::Normal,
                        });
                    }
                    break;
                }
            }
        }
    }

    pub(super) fn fail_speech_delivery(&mut self) {
        let writer_owns_terminal_outcome = self.tts_started;
        let failed_generation = self.generation;
        self.cancel_speech_delivery();
        self.cancel_llm();
        self.cancel_mcp_turn();
        if !self.advance_generation() {
            return;
        }
        warn!(
            event = "speech_delivery_failed",
            failed_generation,
            next_generation = self.generation,
            "Speech delivery failed; generation advanced"
        );
        if !writer_owns_terminal_outcome {
            self.complete_recognition();
        }
    }

    /// Cancels a Conversational Turn in its required order. The outbound gate is
    /// invalidated before producer cancellation, and releasing the Active Turn is
    /// idempotent through its permit ownership flag.
    pub(super) fn interrupt_active_turn(&mut self) {
        self.cancel_speech_delivery();
        self.cancel_llm();
        self.cancel_asr();
    }

    pub(super) fn cancel_speech_delivery(&mut self) {
        // This is the outbound linearization point for the current Conversational Turn.
        // It must precede every producer cancellation, including cancellation before TTS starts:
        // `llm` or `tts:start` may already be waiting at the writer.
        self.generation_gate.invalidate(self.generation);
        self.pending_llm_delta = None;
        self.llm_finish_pending = false;
        self.speech_output.cancel();
        self.pending_audio = None;
        let playback_requested = self.tts_started;
        if playback_requested && let Some(turn_id) = self.current_turn_id() {
            // Writer owns whether start crossed the wire. This merely prevents a recursive
            // urgent-admission failure from attempting the same abort command again.
            self.tts_started = false;
            let payload = serde_json::json!({
                "session_id": self.session_id,
                "type": "tts",
                "state": "stop",
            });
            if let Ok(payload) = serde_json::to_string(&payload)
                && self
                    .urgent_tx
                    .try_send(OutboundMessage::AbortTurn {
                        turn_id,
                        text: payload,
                    })
                    .is_err()
            {
                // Continuing playback without an admitted stop leaves the client state
                // unknowable. Tear the Voice Session down instead of silently retrying.
                self.fail_closed_after_urgent_stop_admission_failure();
            }
        }
        if !playback_requested {
            self.release_active_turn();
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
        if let Some((lease, identity)) = self.asr_stream.take()
            && self.asr_runtime.send(lease, AsrCommand::Cancel).is_ok()
        {
            self.asr_cleanup_pending.insert(identity);
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
        // Writer owns the terminal result.  Keep the turn and its permit until it reports
        // TurnClosed, so global Active Turn capacity remains truthful while a stop is pending.
    }

    pub(super) fn release_active_turn(&mut self) {
        if let Some(turn) = self.turn.take() {
            drop(turn.permit);
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
