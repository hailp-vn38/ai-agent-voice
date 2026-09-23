use super::*;

impl SessionActor {
    pub(super) fn complete_recognition(&mut self) {
        self.release_active_turn();
        if matches!(
            self.listening_mode,
            Some(ListenMode::Auto | ListenMode::Realtime)
        ) && self.vad_session.is_some()
        {
            if self.auto_reset_pending {
                return;
            }
            if let Some((lease, _)) = self.vad_session {
                if self.reset_vad_capture_cycle(lease).is_err() {
                    self.fail_closed();
                } else {
                    self.auto_reset_pending = true;
                }
            }
        } else {
            self.activate_pending_listen_arm();
        }
    }

    fn activate_pending_listen_arm(&mut self) {
        if self.listen_arm_pending {
            let mode = self
                .listening_mode
                .clone()
                .expect("pending listen arm has a listening mode");
            self.replace_listening_mode(mode);
        } else {
            self.phase = SessionPhase::Ready;
        }
    }

    pub(super) fn fail_closed(&mut self) {
        if self.phase == SessionPhase::Closed {
            return;
        }
        warn!(phase = ?self.phase, "voice session failed closed");
        self.interrupt_active_turn();
        self.generation += 1;
        self.close_vad();
        self.auto_reset_pending = false;
        self.asr_stream = None;
        self.phase = SessionPhase::Closed;
        let _ = self.urgent_tx.try_send(OutboundMessage::Close(1011));
    }

    /// An interruption stop that cannot enter the bounded urgent lane leaves playback state
    /// unknowable.  A close cannot be relied on either, so terminate the writer after the
    /// regular session cleanup has run.
    pub(super) fn fail_closed_after_urgent_stop_admission_failure(&mut self) {
        self.fail_closed();
        let _ = self.shutdown_tx.send(true);
    }
}

impl Drop for SessionActor {
    fn drop(&mut self) {
        // The application-owned supervisor continues to observe the acknowledgement or timeout
        // after this actor and its WebSocket have gone away.
        self.cancel_asr();
        self.cancel_llm();
        self.speech_output.cancel();
        self.release_active_turn();
        self.close_vad();
        self.asr_runtime.unregister_session(&self.session_id);
        self.vad_runtime.unregister_session(&self.session_id);
        self.llm_runtime.unregister_session(&self.session_id);
    }
}
