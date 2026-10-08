//! Ticket 15: actor wiring for the **Required** gate. The pure decision lives in
//! [`crate::session::SpeakerGate`]; this module holds the transcript until the speaker operation
//! for that same turn resolves, then admits or refuses it. No behavior changes when the policy is
//! not `required` (`speaker_gate` is `None`).

use super::*;

impl SessionActor {
    /// Whether this session runs the Required gate.
    pub(super) fn speaker_gate_active(&self) -> bool {
        self.speaker_gate.is_some()
    }

    /// Decide and apply one terminal turn. Returns `true` when the transcript may proceed.
    pub(super) fn apply_speaker_gate(&mut self, diagnostic: &ObserveDiagnostic) -> bool {
        let (decision, close_reason) = {
            let Some(gate) = self.speaker_gate.as_mut() else {
                return true;
            };
            let decision = gate.decide(diagnostic);
            let close_reason = gate.record(decision);
            (decision, close_reason)
        };
        match decision {
            GateDecision::Accept { .. } => {
                self.report_gate_state("verified");
                true
            }
            GateDecision::Reject(reason) => {
                self.report_gate_state(reason.state());
                match close_reason {
                    Some(reason) => self.refuse_and_close(reason),
                    None => self.complete_recognition(),
                }
                false
            }
        }
    }

    /// The speaker diagnostic for a Required turn arrived. Resolves the turn when the final is
    /// already held; otherwise the diagnostic waits for the final.
    pub(super) fn on_gate_diagnostic(&mut self, diagnostic: ObserveDiagnostic) {
        if self.speaker_gate.is_none() {
            return;
        }
        if diagnostic.identity.generation != self.generation
            || Some(diagnostic.identity.turn_id) != self.current_turn_id().map(TurnId::get)
        {
            // Stale output for a turn this session no longer owns: it never authorizes anything.
            return;
        }
        if let Some(text) = self.required_text.take() {
            if self.apply_speaker_gate(&diagnostic) {
                self.admit_required_text(text);
            }
        } else {
            self.required_diagnostic = Some(diagnostic);
        }
    }

    /// The ASR final for a Required turn arrived. Resolves the turn when the speaker diagnostic is
    /// already held; otherwise the final waits for the diagnostic (a diagnostic is always emitted
    /// for a Required boundary, even for scoreable-audio-less turns).
    pub(super) fn on_required_final(&mut self, text: String) {
        if let Some(diagnostic) = self.required_diagnostic.take() {
            if self.apply_speaker_gate(&diagnostic) {
                self.admit_required_text(text);
            }
            return;
        }
        self.required_text = Some(text);
    }

    fn admit_required_text(&mut self, text: String) {
        match self.commit_user_text(text) {
            Some(final_text) => self.begin_speech_delivery(final_text),
            None => self.complete_recognition(),
        }
    }

    /// Send the bounded client state for a Required turn. Never a prompt.
    fn report_gate_state(&mut self, state: &str) {
        if !self.speaker_status {
            return;
        }
        let text = serde_json::json!({ "type": "speaker", "state": state }).to_string();
        if self
            .control_tx
            .try_send(OutboundMessage::SpeakerStatus { text })
            .is_err()
        {
            tracing::debug!("speaker state frame dropped: control lane full");
        }
    }

    /// Close a Required session after a fatal refusal (three consecutive counting denials).
    pub(super) fn refuse_and_close(&mut self, reason: &'static str) {
        self.begin_application_shutdown();
        if let Err(error) = self.urgent_tx.try_send(OutboundMessage::CloseWithReason {
            code: 1008,
            reason: reason.to_owned(),
        }) {
            tracing::debug!(%error, "speaker close frame dropped: urgent lane full");
        }
    }

    /// A Required session rejects typed Detect; audio is the only accepted input.
    pub(super) fn required_refuses_detect(&mut self) {
        if self.speaker_gate.is_none() {
            return;
        }
        self.report_gate_state(GateReject::DetectRequiresAudio.state());
    }
}

/// Identification-only join. Unlike the legacy Required gate, an unknown or
/// delayed result never refuses a turn or authorizes any tool.
impl SessionActor {
    pub(super) fn on_identification_final(&mut self, text: String) {
        if self.identification_finished {
            return;
        }
        if let Some(diagnostic) = self.identification_diagnostic.take() {
            self.deliver_identified(text, Some(diagnostic));
        } else {
            self.identification_text = Some(text);
            self.identification_deadline = Some(
                std::time::Instant::now() + std::time::Duration::from_millis(750)
            );
        }
    }

    pub(super) fn on_identification_diagnostic(&mut self, diagnostic: ObserveDiagnostic) {
        if self.identification_finished || self.speaker_observe.is_none()
            || diagnostic.identity.generation != self.generation
            || Some(diagnostic.identity.turn_id) != self.current_turn_id().map(TurnId::get)
        {
            return;
        }
        if let Some(text) = self.identification_text.take() {
            self.deliver_identified(text, Some(diagnostic));
        } else {
            self.identification_diagnostic = Some(diagnostic);
        }
    }

    pub(super) fn expire_identification_wait(&mut self) {
        if self.identification_deadline.is_some_and(|deadline|
            std::time::Instant::now() >= deadline)
        {
            self.identification_deadline = None;
            if let Some(text) = self.identification_text.take() {
                tracing::debug!(turn_id = ?self.current_turn_id(), "speaker identification timeout");
                self.deliver_identified(text, None);
            }
        }
    }

    fn deliver_identified(&mut self, text: String, diagnostic: Option<ObserveDiagnostic>) {
        self.identification_finished = true;
        self.identification_text = None;
        self.identification_diagnostic = None;
        self.identification_deadline = None;
        self.speaker_name_for_turn = diagnostic.and_then(|diagnostic| {
            if diagnostic.outcome != crate::session::SpeakerStatus::Verified {
                return None;
            }
            self.speaker_observe.as_ref()?.plan().candidates.iter()
                .find(|candidate| Some(candidate.speaker_id) == diagnostic.best_speaker_id)
                .map(|candidate| candidate.key.clone())
        });
        match self.commit_user_text(text) {
            Some(text) => self.begin_speech_delivery(text),
            None => self.complete_recognition(),
        }
    }
}

pub(super) fn insufficient_audio_diagnostic(
    identity: crate::session::ObserveIdentity,
) -> ObserveDiagnostic {
    ObserveDiagnostic {
        identity,
        embedding_space: String::new(),
        catalog_revision: 0,
        samples: 0,
        speech_ms: 0,
        queue_ms: 0,
        inference_ms: 0,
        gate_wait_ms: 0,
        outcome: crate::session::SpeakerStatus::InsufficientAudio,
        best_speaker_id: None,
        best_score: None,
        runner_up_score: None,
    }
}
