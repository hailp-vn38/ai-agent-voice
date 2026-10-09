//! Identification-only join for the current conversational turn.

use super::*;

impl SessionActor {
    pub(super) fn on_identification_final(&mut self, text: String) {
        if self.identification_finished {
            return;
        }
        if let Some(diagnostic) = self.identification_diagnostic.take() {
            self.deliver_identified(text, Some(diagnostic));
        } else {
            self.identification_text = Some(text);
            let timeout = self.speaker_observe.as_ref().map_or(
                std::time::Duration::from_secs(10),
                |observe| observe.join_timeout(),
            );
            self.identification_deadline = Some(std::time::Instant::now() + timeout);
        }
    }

    pub(super) fn on_identification_diagnostic(&mut self, diagnostic: ObserveDiagnostic) {
        if self.identification_finished
            || self.speaker_observe.is_none()
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
        if self
            .identification_deadline
            .is_some_and(|deadline| std::time::Instant::now() >= deadline)
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
        self.speaker_context_for_turn = diagnostic.and_then(|diagnostic| {
            if diagnostic.outcome != crate::session::SpeakerStatus::Verified {
                return None;
            }
            self.speaker_observe
                .as_ref()?
                .plan()
                .candidates
                .iter()
                .find(|candidate| Some(candidate.speaker_id) == diagnostic.best_speaker_id)
                .map(|candidate| crate::session::SpeakerContext {
                    name: candidate.key.clone(),
                    description: candidate.description.clone(),
                })
        });
        match self.commit_user_text(text) {
            Some(text) => self.begin_speech_delivery(text),
            None => self.complete_recognition(),
        }
    }
}
