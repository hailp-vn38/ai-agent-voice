//! Ticket 10: the actor side of Observe. Ingest keeps a bounded copy of the utterance PCM while
//! Observe is installed; the terminal boundary spawns one best-effort scoring task and drops any
//! boundary that arrives while one is already running.
//!
//! Observe is deliberately outside the turn lifecycle: it never returns a value the core path
//! checks, never holds a turn open, and never changes accept behaviour.

use std::sync::atomic::Ordering;

use super::*;
use crate::session::speaker_observe::OBSERVE_MAX_SAMPLES;

impl SessionActor {
    /// Retain the canonical uplink PCM a later Observe scoring pass will consume.
    ///
    /// Called for every Auto/Realtime uplink frame; a session with Observe `off` does no work here.
    pub(super) fn observe_retain(&mut self, samples: &[f32]) {
        if self.speaker_observe.is_none() {
            return;
        }
        let remaining = OBSERVE_MAX_SAMPLES.saturating_sub(self.observe_pcm.len());
        if remaining == 0 {
            return;
        }
        let take = remaining.min(samples.len());
        self.observe_pcm.extend_from_slice(&samples[..take]);
    }

    /// Drop retained PCM at a speech-segment start so each utterance is scored on its own audio.
    pub(super) fn observe_reset(&mut self) {
        self.observe_pcm.clear();
    }

    /// Fire Observe at the utterance terminal boundary, once.
    ///
    /// The identity is captured synchronously (operation id, turn id, generation) so a late result
    /// can be attributed and dropped if the session moved on. The PCM is drained here: whatever was
    /// retained belongs to exactly this boundary.
    pub(super) fn observe_utterance_boundary(&mut self) {
        self.identification_text = None;
        self.identification_diagnostic = None;
        self.identification_deadline = None;
        self.identification_finished = false;
        self.speaker_context_for_turn = None;
        let Some(observe) = self.speaker_observe.as_ref().cloned() else {
            return;
        };
        let samples = std::mem::take(&mut self.observe_pcm);
        if samples.is_empty() || self.observe_in_flight.load(Ordering::Acquire) {
            // Do not wait the full Speaker join timeout when no scoring job was launched.
            let outcome = if samples.is_empty() {
                crate::session::SpeakerStatus::InsufficientAudio
            } else {
                crate::session::SpeakerStatus::Unavailable
            };
            self.identification_diagnostic = Some(ObserveDiagnostic {
                identity: crate::session::ObserveIdentity {
                    operation_id: 0,
                    turn_id: self.current_turn_id().map(TurnId::get).unwrap_or(0),
                    generation: self.generation,
                },
                embedding_space: observe.embedding_space().to_owned(),
                catalog_revision: observe.plan().catalog_revision,
                samples: samples.len(),
                speech_ms: 0,
                queue_ms: 0,
                inference_ms: 0,
                gate_wait_ms: 0,
                outcome,
                best_speaker_id: None,
                best_score: None,
                runner_up_score: None,
            });
            return;
        }
        self.observe_in_flight.store(true, Ordering::Release);
        let generation = self.generation;
        let turn_id = self.current_turn_id().map(TurnId::get).unwrap_or(0);
        let operation_id = self.next_operation_id;
        self.next_operation_id = self.next_operation_id.saturating_add(1);
        let identity = crate::session::ObserveIdentity {
            operation_id,
            turn_id,
            generation,
        };
        if self.speaker_status {
            self.send_speaker_state(crate::session::SpeakerStatus::Verifying);
        }

        let control_tx = self.control_tx.clone();
        let generation_gate = self.generation_gate.clone();
        let in_flight = self.observe_in_flight.clone();
        let status_frames = self.speaker_status;
        let gate_tx = self.gate_tx.clone();
        tokio::spawn(async move {
            let diagnostic = observe.observe(identity, &samples).await;
            let recognized_speaker = (diagnostic.outcome
                == crate::session::SpeakerStatus::Verified)
                .then(|| {
                    observe
                        .plan()
                        .candidates
                        .iter()
                        .find(|candidate| Some(candidate.speaker_id) == diagnostic.best_speaker_id)
                        .map(|candidate| candidate.key.as_str())
                })
                .flatten();
            tracing::info!(
                operation_id = diagnostic.identity.operation_id,
                turn_id = diagnostic.identity.turn_id,
                generation = diagnostic.identity.generation,
                embedding_space = %diagnostic.embedding_space,
                catalog_revision = diagnostic.catalog_revision,
                samples = diagnostic.samples,
                speech_ms = diagnostic.speech_ms,
                inference_ms = diagnostic.inference_ms,
                state = diagnostic.outcome.as_str(),
                recognized = recognized_speaker.is_some(),
                speaker_key = recognized_speaker.unwrap_or("none"),
                best_score = diagnostic.best_score,
                "speaker observe",
            );
            in_flight.store(false, Ordering::Release);
            let _ = gate_tx.send(diagnostic.clone());
            // Stale results are cleanup-only: they never surface, and the latch is already clear.
            if !status_frames || !generation_gate.admits(diagnostic.identity.generation) {
                return;
            }
            let _ = control_tx
                .send(OutboundMessage::SpeakerStatus {
                    text: diagnostic.wire_text(),
                })
                .await;
        });
    }

    /// Fire-and-forget bounded status frame, matching `PipelineStatus` backpressure.
    fn send_speaker_state(&mut self, state: crate::session::SpeakerStatus) {
        let text = serde_json::json!({
            "type": "speaker",
            "state": state.as_str(),
        })
        .to_string();
        if self
            .control_tx
            .try_send(OutboundMessage::SpeakerStatus { text })
            .is_err()
        {
            // Status is advisory; a full control lane must not fail the session closed.
            tracing::debug!("speaker status frame dropped: control lane full");
        }
    }
}
