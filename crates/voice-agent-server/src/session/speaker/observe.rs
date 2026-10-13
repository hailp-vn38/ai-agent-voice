//! Ticket 10: Agent **Observe** — best-effort speaker scoring for an accepted Voice Session.
//!
//! Observe never denies a turn, never grants authority by voice, and never puts speaker identity
//! or a raw score on the wire. When the Agent policy is `off` none of this runs and no utterance
//! PCM is retained.
//!
//! The Agent policy, the active Template's granted candidates and the published voiceprints are
//! resolved once at session admission into an [`ObservePlan`]. Each terminal utterance boundary
//! then scores the utterance against that plan through the exact selected [`SpeakerRuntime`].

use std::sync::Arc;

use tokio_util::sync::CancellationToken;

use crate::audio::PcmF32Mono;
use crate::audio::enrollment::{self, QualityProfile};
use crate::services::provider_runtime::ResourceLease;
use crate::workers::SpeakerRuntime;

/// Sample rate the speaker extractor consumes. Uplink PCM is already canonicalised to this.
pub const OBSERVE_SAMPLE_RATE_HZ: u32 = 16_000;
/// Longest utterance window the extractor will consume (matches `enrollment::MAX_WINDOW_MS`).
pub const OBSERVE_MAX_SAMPLES: usize = 96_000;

/// Conservative Observe decision threshold.
// ponytail: fixed threshold; ticket 14 publishes the calibration threshold. Observe never gates,
// so a wrong value only mislabels the diagnostic state, it cannot change accept behaviour.
pub const OBSERVE_VERIFY_THRESHOLD: f32 = 0.5;

/// The Agent's speaker policy. Only `off` and `observe` are exercised by this ticket.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SpeakerPolicyMode {
    Off,
    Observe,
}

impl SpeakerPolicyMode {
    pub fn parse(raw: &str) -> Option<Self> {
        match raw {
            "off" => Some(Self::Off),
            "observe" => Some(Self::Observe),
            _ => None,
        }
    }

    /// `true` when the session should run best-effort identification.
    pub fn observe_enabled(self) -> bool {
        !matches!(self, Self::Off)
    }
}

/// One granted, enrolled candidate the utterance can be scored against.
#[derive(Clone, Debug, PartialEq)]
pub struct ObserveCandidate {
    pub speaker_id: i64,
    pub key: String,
    /// Optional human-entered profile; never an authorization credential.
    pub description: Option<String>,
    pub vector: Vec<f32>,
}

/// Verified human-facing speaker profile, valid only for the current LLM turn.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SpeakerContext {
    pub name: String,
    pub description: Option<String>,
}

/// Immutable admission-time Observe plan for the active Template.
#[derive(Clone, Debug, PartialEq)]
pub struct ObservePlan {
    pub agent_id: i64,
    pub template_id: i64,
    pub embedding_space: String,
    pub catalog_revision: i64,
    pub policy: SpeakerPolicyMode,
    pub candidates: Vec<ObserveCandidate>,
}

/// One candidate's similarity to the utterance embedding. Internal only — never serialised.
#[derive(Clone, Debug, PartialEq)]
pub struct ObserveScore {
    pub speaker_id: i64,
    pub key: String,
    pub score: f32,
}

impl ObservePlan {
    /// Score every candidate, highest first. Bounded by the candidate count.
    pub fn score(&self, embedding: &[f32]) -> Vec<ObserveScore> {
        let mut scores: Vec<ObserveScore> = self
            .candidates
            .iter()
            .filter_map(|candidate| {
                enrollment::cosine(embedding, &candidate.vector).map(|score| ObserveScore {
                    speaker_id: candidate.speaker_id,
                    key: candidate.key.clone(),
                    score,
                })
            })
            .collect();
        scores.sort_by(|a, b| b.score.total_cmp(&a.score));
        scores
    }
}

/// The bounded speaker state surfaced to an opted-in client. No identity, no score.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SpeakerStatus {
    Verifying,
    Verified,
    Unknown,
    InsufficientAudio,
    Unavailable,
}

impl SpeakerStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Verifying => "verifying",
            Self::Verified => "verified",
            Self::Unknown => "unknown",
            Self::InsufficientAudio => "insufficient_audio",
            Self::Unavailable => "unavailable",
        }
    }
}

/// Identity captured at the terminal boundary so a late Observe result can be attributed and
/// dropped if it belongs to a stale generation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ObserveIdentity {
    pub operation_id: u64,
    pub turn_id: u64,
    pub generation: u64,
}

/// The Observe outcome plus the timing/quality diagnostic. `best_*` stay in-process only.
#[derive(Clone, Debug, PartialEq)]
pub struct ObserveDiagnostic {
    pub identity: ObserveIdentity,
    pub embedding_space: String,
    pub catalog_revision: i64,
    pub samples: usize,
    pub speech_ms: u64,
    pub queue_ms: u64,
    pub inference_ms: u64,
    pub gate_wait_ms: u64,
    pub outcome: SpeakerStatus,
    pub best_speaker_id: Option<i64>,
    pub best_score: Option<f32>,
    /// Second-best similarity, when the candidate set had more than one scoreable entry.
    pub runner_up_score: Option<f32>,
}

impl ObserveDiagnostic {
    /// Bounded wire frame: state only, never speaker key, identity, score or candidate list.
    pub fn wire_text(&self) -> String {
        serde_json::json!({
            "type": "speaker",
            "state": self.outcome.as_str(),
        })
        .to_string()
    }
}

/// Session-scoped Observe runtime: the exact selected extractor, the lease that keeps it
/// resident, the admission-time plan and the quality profile.
pub struct SpeakerObserve {
    runtime: Arc<SpeakerRuntime>,
    lease: Option<ResourceLease>,
    plan: ObservePlan,
    profile: QualityProfile,
    threshold: f32,
    join_timeout: std::time::Duration,
    security: Arc<CancellationToken>,
}

impl SpeakerObserve {
    pub fn new(
        runtime: Arc<SpeakerRuntime>,
        lease: ResourceLease,
        plan: ObservePlan,
        profile: QualityProfile,
    ) -> Self {
        Self {
            runtime,
            lease: Some(lease),
            plan,
            profile,
            threshold: OBSERVE_VERIFY_THRESHOLD,
            join_timeout: std::time::Duration::from_secs(10),
            // A session admitted without the registry (tests, Observe-only harnesses) is never
            // revoked; real admission attaches the registry token via `with_security`.
            security: Arc::new(CancellationToken::new()),
        }
    }

    /// Built-in speaker engine is process-owned; no database Provider lease is required.
    pub fn new_builtin(
        runtime: Arc<SpeakerRuntime>,
        plan: ObservePlan,
        profile: QualityProfile,
        threshold: f32,
        join_timeout_ms: u64,
    ) -> Self {
        Self {
            runtime,
            lease: None,
            plan,
            profile,
            threshold,
            join_timeout: std::time::Duration::from_millis(join_timeout_ms),
            security: Arc::new(CancellationToken::new()),
        }
    }

    /// Attaches the registry token that revokes this session when a Speaker/grant/policy/template
    /// mutation invalidates its snapshot.
    pub fn with_security(mut self, security: Arc<CancellationToken>) -> Self {
        self.security = security;
        self
    }

    pub fn security_token(&self) -> Arc<CancellationToken> {
        Arc::clone(&self.security)
    }

    pub fn security_cancelled(&self) -> bool {
        self.security.is_cancelled()
    }

    pub fn join_timeout(&self) -> std::time::Duration {
        self.join_timeout
    }

    pub fn plan(&self) -> &ObservePlan {
        &self.plan
    }

    pub fn embedding_space(&self) -> &str {
        &self.plan.embedding_space
    }

    /// Ticket 17: the same runtime, lease and security epoch, scoring against a target Template's
    /// frozen plan. Used only after the switch authority accepted the target in this space.
    pub fn retargeted(&self, plan: ObservePlan) -> Self {
        Self {
            runtime: Arc::clone(&self.runtime),
            lease: self.lease.clone(),
            plan,
            profile: self.profile,
            threshold: self.threshold,
            join_timeout: self.join_timeout,
            security: Arc::clone(&self.security),
        }
    }

    /// Score one utterance. Best-effort: any runtime/quality failure becomes a bounded state,
    /// never an error the caller has to handle.
    pub async fn observe(&self, identity: ObserveIdentity, pcm: &[f32]) -> ObserveDiagnostic {
        let samples = pcm.len();
        let mut diagnostic = ObserveDiagnostic {
            identity,
            embedding_space: self.plan.embedding_space.clone(),
            catalog_revision: self.plan.catalog_revision,
            samples,
            speech_ms: 0,
            queue_ms: 0,
            inference_ms: 0,
            gate_wait_ms: 0,
            outcome: SpeakerStatus::Unknown,
            best_speaker_id: None,
            best_score: None,
            runner_up_score: None,
        };
        if samples < 1 {
            diagnostic.outcome = SpeakerStatus::InsufficientAudio;
            return diagnostic;
        }
        let duration_ms = (samples as u64 * 1000) / u64::from(OBSERVE_SAMPLE_RATE_HZ);
        let clip = PcmF32Mono::new(pcm.to_vec(), OBSERVE_SAMPLE_RATE_HZ);
        let analyzed = match enrollment::analyze(&clip, duration_ms, &self.profile) {
            Ok(analyzed) => analyzed,
            Err(enrollment::Reject::TooShort | enrollment::Reject::InsufficientAudio) => {
                diagnostic.outcome = SpeakerStatus::InsufficientAudio;
                return diagnostic;
            }
            Err(_) => return diagnostic,
        };
        diagnostic.speech_ms = analyzed.quality.speech_ms;
        // The extractor needs a full window; a shorter utterance is a quality outcome, not a
        // runtime failure, so do not spend an extraction on it.
        if analyzed.window.samples().len() < self.runtime.min_window_samples() {
            diagnostic.outcome = SpeakerStatus::InsufficientAudio;
            return diagnostic;
        }

        let started = std::time::Instant::now();
        let extracted = match &self.lease {
            Some(lease) => self.runtime.extract(analyzed.window, lease.clone()).await,
            None => self.runtime.extract_builtin(analyzed.window).await,
        };
        match extracted {
            Ok(embedding) => {
                diagnostic.inference_ms = started.elapsed().as_millis() as u64;
                let scored = self.plan.score(&embedding);
                if let Some(best) = scored.first() {
                    diagnostic.best_speaker_id = Some(best.speaker_id);
                    diagnostic.best_score = Some(best.score);
                    diagnostic.runner_up_score = scored.get(1).map(|second| second.score);
                    diagnostic.outcome = if best.score >= self.threshold {
                        SpeakerStatus::Verified
                    } else {
                        SpeakerStatus::Unknown
                    };
                }
            }
            Err(_) => diagnostic.outcome = SpeakerStatus::Unavailable,
        }
        diagnostic
    }
}

/// The outcome of resolving an Agent's speaker policy for a session.
pub enum ObserveResolution {
    /// Nothing to run: policy `off`, no policy, or `observe` with no candidate set.
    Off,
    /// A frozen plan to score against.
    Plan(ObservePlan),
    /// A persisted policy value outside the supported contract. Admission must reject rather
    /// than silently weakening it to `off`.
    InvalidPolicy,
}

/// Resolve persisted policy before any runtime dependency. An unrecognised stored value is a
/// security error at the admission boundary, never an implicit `off` policy.
pub use crate::database::speakers::observations::{resolve_observe_plan, resolve_speaker_policy};

#[cfg(test)]
mod tests {
    use super::*;

    fn plan() -> ObservePlan {
        ObservePlan {
            agent_id: 7,
            template_id: 3,
            embedding_space: "speaker:abc".into(),
            catalog_revision: 42,
            policy: SpeakerPolicyMode::Observe,
            candidates: vec![
                ObserveCandidate {
                    speaker_id: 1,
                    key: "alice".into(),
                    description: Some("Alice profile".into()),
                    vector: vec![1.0, 0.0],
                },
                ObserveCandidate {
                    speaker_id: 2,
                    key: "bob".into(),
                    description: None,
                    vector: vec![0.0, 1.0],
                },
            ],
        }
    }

    #[test]
    fn policy_parses_and_gates_off() {
        assert_eq!(
            SpeakerPolicyMode::parse("off"),
            Some(SpeakerPolicyMode::Off)
        );
        assert_eq!(
            SpeakerPolicyMode::parse("observe"),
            Some(SpeakerPolicyMode::Observe)
        );
        assert_eq!(SpeakerPolicyMode::parse("unexpected"), None);
        assert!(!SpeakerPolicyMode::Off.observe_enabled());
        assert!(SpeakerPolicyMode::Observe.observe_enabled());
    }

    #[test]
    fn score_is_ranked_and_bounded_by_candidates() {
        let scores = plan().score(&[1.0, 0.0]);
        assert_eq!(scores.len(), 2);
        assert_eq!(scores[0].speaker_id, 1);
        assert!((scores[0].score - 1.0).abs() < 1e-6);
        assert_eq!(scores[1].speaker_id, 2);
        assert!(scores[1].score.abs() < 1e-6);
    }

    #[test]
    fn wire_text_never_leaks_identity_or_score() {
        let diagnostic = ObserveDiagnostic {
            identity: ObserveIdentity {
                operation_id: 9,
                turn_id: 4,
                generation: 2,
            },
            embedding_space: "speaker:abc".into(),
            catalog_revision: 42,
            samples: 16_000,
            speech_ms: 900,
            queue_ms: 1,
            inference_ms: 5,
            gate_wait_ms: 0,
            outcome: SpeakerStatus::Verified,
            best_speaker_id: Some(1),
            best_score: Some(0.93),
            runner_up_score: Some(0.10),
        };
        let text = diagnostic.wire_text();
        let value: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(value["type"], "speaker");
        assert_eq!(value["state"], "verified");
        let object = value.as_object().unwrap();
        assert_eq!(object.len(), 2, "unexpected fields in {text}");
        for forbidden in ["alice", "score", "0.93", "speaker_id", "best", "embedding"] {
            assert!(
                !text.contains(forbidden),
                "wire frame leaked {forbidden}: {text}"
            );
        }
    }
}
