//! Ticket 15: the **Required** gate. Observe scores every terminal voice turn; Required turns that
//! score into an authorization decision and holds the transcript until it resolves.
//!
//! The first turn identifies 1:N (top-1 over the Agent's exact candidate set plus an identification
//! margin); later turns verify 1:1 against the identity locked on the WebSocket. A pass resets the
//! consecutive-mismatch counter, a denial counts, and a short/unavailable/runtime turn is a
//! bounded non-answer that never counts. Three consecutive denials close the session.
//!
//! This module is pure: it never touches the actor, the wire, or the runtime.

use crate::session::speaker_observe::{ObserveDiagnostic, SpeakerStatus};

/// The top candidate must clear this over the runner-up on the identifying (1:N) turn. A single
/// candidate skips the margin and only has to clear the verify threshold.
pub const SPEAKER_GATE_MARGIN: f32 = 0.15;
/// Consecutive denials that close the session with 1008.
pub const SPEAKER_GATE_MAX_MISMATCHES: u8 = 3;

/// Why a Required turn was refused. Every one maps to a bounded client state, never a prompt.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GateReject {
    /// No candidate cleared the verify threshold.
    Unknown,
    /// Two candidates cleared the threshold but the top did not clear the margin.
    Ambiguous,
    /// The utterance was too short to score.
    InsufficientAudio,
    /// The speaker runtime was busy, timed out, or otherwise unavailable.
    Unavailable,
    /// A locked identity exists and this turn did not match it.
    Mismatch,
    /// A Required session does not accept typed Detect input.
    DetectRequiresAudio,
}

impl GateReject {
    /// The bounded client state surfaced for this refusal.
    pub fn state(self) -> &'static str {
        match self {
            Self::Unknown => "unknown",
            Self::Ambiguous => "ambiguous",
            Self::InsufficientAudio => "insufficient_audio",
            Self::Unavailable => "unavailable",
            Self::Mismatch | Self::DetectRequiresAudio => "denied",
        }
    }

    /// Short, content-free close reason for a refusal that ends the session.
    pub fn close_reason(self) -> &'static str {
        match self {
            Self::DetectRequiresAudio => "speaker_audio_required",
            _ => "speaker_policy_denied",
        }
    }

    /// Whether this refusal counts toward the consecutive-mismatch close. Short audio, runtime
    /// availability, and typed Detect are bounded non-answers and never count.
    pub fn counts_as_mismatch(self) -> bool {
        matches!(self, Self::Unknown | Self::Ambiguous | Self::Mismatch)
    }
}

/// The authorization outcome of one terminal voice turn.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GateDecision {
    /// Authorized for this turn; the identity is (re)locked.
    Accept {
        speaker_id: i64,
    },
    Reject(GateReject),
}

/// Per-WebSocket Required state: the locked identity and the consecutive-denial counter.
#[derive(Clone, Copy, Debug, Default)]
pub struct SpeakerGate {
    locked: Option<i64>,
    mismatches: u8,
}

impl SpeakerGate {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn locked(&self) -> Option<i64> {
        self.locked
    }

    pub fn mismatches(&self) -> u8 {
        self.mismatches
    }

    /// Decide one terminal turn from its Observe diagnostic. Pure; the lock is only mutated by
    /// [`Self::record`] once the decision is applied.
    pub fn decide(&self, diagnostic: &ObserveDiagnostic) -> GateDecision {
        match diagnostic.outcome {
            SpeakerStatus::Unavailable => GateDecision::Reject(GateReject::Unavailable),
            SpeakerStatus::InsufficientAudio => GateDecision::Reject(GateReject::InsufficientAudio),
            SpeakerStatus::Unknown | SpeakerStatus::Verifying => {
                GateDecision::Reject(GateReject::Unknown)
            }
            SpeakerStatus::Verified => {
                let Some(best) = diagnostic.best_speaker_id else {
                    return GateDecision::Reject(GateReject::Unknown);
                };
                match self.locked {
                    Some(locked) if locked != best => GateDecision::Reject(GateReject::Mismatch),
                    Some(_) => GateDecision::Accept { speaker_id: best },
                    None => {
                        if let (Some(best_score), Some(runner_up)) =
                            (diagnostic.best_score, diagnostic.runner_up_score)
                            && best_score - runner_up < SPEAKER_GATE_MARGIN
                        {
                            return GateDecision::Reject(GateReject::Ambiguous);
                        }
                        GateDecision::Accept { speaker_id: best }
                    }
                }
            }
        }
    }

    /// Fold an applied decision into the lock/counter. Returns the close reason when the session
    /// must close (three consecutive counting denials), `None` otherwise.
    pub fn record(&mut self, decision: GateDecision) -> Option<&'static str> {
        match decision {
            GateDecision::Accept { speaker_id } => {
                self.locked = Some(speaker_id);
                self.mismatches = 0;
                None
            }
            GateDecision::Reject(reason) => {
                if !reason.counts_as_mismatch() {
                    return None;
                }
                self.mismatches = self.mismatches.saturating_add(1);
                (self.mismatches >= SPEAKER_GATE_MAX_MISMATCHES).then_some(reason.close_reason())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::speaker_observe::ObserveIdentity;

    fn diagnostic(
        outcome: SpeakerStatus,
        best: Option<i64>,
        best_score: Option<f32>,
        runner_up: Option<f32>,
    ) -> ObserveDiagnostic {
        ObserveDiagnostic {
            identity: ObserveIdentity {
                operation_id: 1,
                turn_id: 1,
                generation: 1,
            },
            embedding_space: "test".into(),
            catalog_revision: 1,
            samples: 16000,
            speech_ms: 900,
            queue_ms: 0,
            inference_ms: 0,
            gate_wait_ms: 0,
            outcome,
            best_speaker_id: best,
            best_score,
            runner_up_score: runner_up,
        }
    }

    #[test]
    fn first_turn_identifies_and_locks() {
        let gate = SpeakerGate::new();
        let decision = gate.decide(&diagnostic(
            SpeakerStatus::Verified,
            Some(7),
            Some(0.91),
            Some(0.42),
        ));
        assert_eq!(decision, GateDecision::Accept { speaker_id: 7 });
    }

    #[test]
    fn single_candidate_skips_margin() {
        let gate = SpeakerGate::new();
        let decision = gate.decide(&diagnostic(
            SpeakerStatus::Verified,
            Some(3),
            Some(0.6),
            None,
        ));
        assert_eq!(decision, GateDecision::Accept { speaker_id: 3 });
    }

    #[test]
    fn close_top_two_is_ambiguous() {
        let gate = SpeakerGate::new();
        let decision = gate.decide(&diagnostic(
            SpeakerStatus::Verified,
            Some(7),
            Some(0.80),
            Some(0.70),
        ));
        assert_eq!(decision, GateDecision::Reject(GateReject::Ambiguous));
    }

    #[test]
    fn below_threshold_is_unknown() {
        let gate = SpeakerGate::new();
        let decision = gate.decide(&diagnostic(SpeakerStatus::Unknown, None, None, None));
        assert_eq!(decision, GateDecision::Reject(GateReject::Unknown));
    }

    #[test]
    fn locked_turn_must_match() {
        let mut gate = SpeakerGate::new();
        gate.record(GateDecision::Accept { speaker_id: 7 });
        assert_eq!(gate.locked(), Some(7));
        assert_eq!(
            gate.decide(&diagnostic(
                SpeakerStatus::Verified,
                Some(7),
                Some(0.9),
                Some(0.1)
            )),
            GateDecision::Accept { speaker_id: 7 }
        );
        assert_eq!(
            gate.decide(&diagnostic(
                SpeakerStatus::Verified,
                Some(9),
                Some(0.9),
                Some(0.1)
            )),
            GateDecision::Reject(GateReject::Mismatch)
        );
    }

    #[test]
    fn short_and_unavailable_never_count() {
        let mut gate = SpeakerGate::new();
        for _ in 0..5 {
            assert_eq!(
                gate.record(GateDecision::Reject(GateReject::InsufficientAudio)),
                None
            );
            assert_eq!(
                gate.record(GateDecision::Reject(GateReject::Unavailable)),
                None
            );
        }
        assert_eq!(gate.mismatches(), 0);
    }

    #[test]
    fn three_consecutive_denials_close_and_pass_resets() {
        let mut gate = SpeakerGate::new();
        assert_eq!(gate.record(GateDecision::Reject(GateReject::Unknown)), None);
        assert_eq!(
            gate.record(GateDecision::Reject(GateReject::Ambiguous)),
            None
        );
        assert_eq!(
            gate.record(GateDecision::Reject(GateReject::Mismatch)),
            Some("speaker_policy_denied")
        );
        assert_eq!(gate.mismatches(), 3);

        // A pass resets the streak.
        let mut gate = SpeakerGate::new();
        gate.record(GateDecision::Reject(GateReject::Unknown));
        gate.record(GateDecision::Accept { speaker_id: 4 });
        assert_eq!(gate.mismatches(), 0);
        assert_eq!(gate.locked(), Some(4));
    }

    #[test]
    fn detect_denial_reports_audio_required() {
        assert_eq!(
            GateReject::DetectRequiresAudio.close_reason(),
            "speaker_audio_required"
        );
        assert!(!GateReject::DetectRequiresAudio.counts_as_mismatch());
    }
}
