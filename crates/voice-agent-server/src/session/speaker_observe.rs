//! Ticket 10: Agent **Observe** — best-effort speaker scoring for an accepted Voice Session.
//!
//! Observe never gates a turn, never grants authority by voice, and never puts speaker identity
//! or a raw score on the wire. When the Agent policy is `off` none of this runs and no utterance
//! PCM is retained.
//!
//! The Agent policy, the active Template's granted candidates and the published voiceprints are
//! resolved once at session admission into an [`ObservePlan`]. Each terminal utterance boundary
//! then scores the utterance against that plan through the exact selected [`SpeakerRuntime`].

use std::sync::Arc;

use sqlx::SqlitePool;
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
    Required,
}

impl SpeakerPolicyMode {
    pub fn parse(raw: &str) -> Self {
        match raw {
            "observe" => Self::Observe,
            "required" => Self::Required,
            _ => Self::Off,
        }
    }

    /// `true` when the session must run Observe inference (observe or required).
    pub fn observe_enabled(self) -> bool {
        !matches!(self, Self::Off)
    }
}

/// One granted, enrolled candidate the utterance can be scored against.
#[derive(Clone, Debug, PartialEq)]
pub struct ObserveCandidate {
    pub speaker_id: i64,
    pub key: String,
    pub vector: Vec<f32>,
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
    lease: ResourceLease,
    plan: ObservePlan,
    profile: QualityProfile,
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
            lease,
            plan,
            profile,
            // A session admitted without the registry (tests, Observe-only harnesses) is never
            // revoked; real admission attaches the registry token via `with_security`.
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
        match self
            .runtime
            .extract(analyzed.window, self.lease.clone())
            .await
        {
            Ok(embedding) => {
                diagnostic.inference_ms = started.elapsed().as_millis() as u64;
                let scored = self.plan.score(&embedding);
                if let Some(best) = scored.first() {
                    diagnostic.best_speaker_id = Some(best.speaker_id);
                    diagnostic.best_score = Some(best.score);
                    diagnostic.runner_up_score = scored.get(1).map(|second| second.score);
                    diagnostic.outcome = if best.score >= OBSERVE_VERIFY_THRESHOLD {
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
    /// Policy is `required` but no candidate set resolved. Admission MUST fail closed rather
    /// than admit the session ungated.
    RequiredUnavailable,
}

/// Resolve the Agent policy and, when Observe is enabled, the active Template's granted
/// candidates with a published voiceprint in the selected runtime's embedding space.
///
/// Returns [`ObserveResolution::Off`] for `off`, a missing policy, or an `observe` policy with
/// an empty candidate set. A `required` policy with no candidate set resolves to
/// [`ObserveResolution::RequiredUnavailable`] so the caller can refuse admission instead of
/// silently admitting ungated.
pub async fn resolve_observe_plan(
    pool: &SqlitePool,
    agent_id: i64,
    template_id: i64,
    embedding_space: &str,
) -> Result<ObserveResolution, sqlx::Error> {
    use sqlx::Row;

    let mode: Option<String> =
        sqlx::query_scalar("SELECT mode FROM agent_speaker_policies WHERE agent_id = ?")
            .bind(agent_id)
            .fetch_optional(pool)
            .await?;
    let Some(mode) = mode.map(|raw| SpeakerPolicyMode::parse(&raw)) else {
        return Ok(ObserveResolution::Off);
    };
    if !mode.observe_enabled() {
        return Ok(ObserveResolution::Off);
    }

    let rows = sqlx::query(
        "SELECT s.id AS speaker_id, s.key AS key, v.vector AS vector,
                v.browser_validation_status AS validation_status
           FROM agent_speaker_candidates c
           JOIN speakers s ON s.id = c.speaker_id AND s.enabled = 1
           JOIN agent_speaker_template_grants g
             ON g.agent_id = c.agent_id AND g.speaker_id = c.speaker_id AND g.template_id = ?
           JOIN speaker_voiceprints v
             ON v.speaker_id = c.speaker_id AND v.embedding_space = ?
          WHERE c.agent_id = ?",
    )
    .bind(template_id)
    .bind(embedding_space)
    .bind(agent_id)
    .fetch_all(pool)
    .await?;

    let required_has_provisional = mode == SpeakerPolicyMode::Required
        && rows
            .iter()
            .any(|row| row.get::<String, _>("validation_status") != "passed");
    let candidates: Vec<ObserveCandidate> = rows
        .into_iter()
        .map(|row| ObserveCandidate {
            speaker_id: row.get("speaker_id"),
            key: row.get("key"),
            vector: enrollment::decode_embedding(&row.get::<Vec<u8>, _>("vector")),
        })
        .collect();
    if candidates.is_empty() || required_has_provisional {
        return Ok(if mode == SpeakerPolicyMode::Required {
            ObserveResolution::RequiredUnavailable
        } else {
            ObserveResolution::Off
        });
    }

    let catalog_revision: i64 =
        sqlx::query_scalar("SELECT revision FROM speaker_catalog WHERE id = 1")
            .fetch_one(pool)
            .await?;

    Ok(ObserveResolution::Plan(ObservePlan {
        agent_id,
        template_id,
        embedding_space: embedding_space.to_owned(),
        catalog_revision,
        policy: mode,
        candidates,
    }))
}
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
                    vector: vec![1.0, 0.0],
                },
                ObserveCandidate {
                    speaker_id: 2,
                    key: "bob".into(),
                    vector: vec![0.0, 1.0],
                },
            ],
        }
    }

    #[test]
    fn policy_parses_and_gates_off() {
        assert_eq!(SpeakerPolicyMode::parse("off"), SpeakerPolicyMode::Off);
        assert_eq!(
            SpeakerPolicyMode::parse("observe"),
            SpeakerPolicyMode::Observe
        );
        assert_eq!(
            SpeakerPolicyMode::parse("required"),
            SpeakerPolicyMode::Required
        );
        assert!(!SpeakerPolicyMode::Off.observe_enabled());
        assert!(SpeakerPolicyMode::Observe.observe_enabled());
        assert!(SpeakerPolicyMode::Required.observe_enabled());
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

    /// A `required` policy with no candidate set must not degrade to "nothing to run"; it must
    /// signal that admission has to fail closed.
    #[tokio::test]
    async fn required_without_candidates_fails_closed() {
        let pool = sqlx::SqlitePool::connect("sqlite::memory:").await.unwrap();
        for ddl in [
            "CREATE TABLE agent_speaker_policies (agent_id INTEGER, mode TEXT)",
            "CREATE TABLE agent_speaker_candidates (agent_id INTEGER, speaker_id INTEGER)",
            "CREATE TABLE speakers (id INTEGER, key TEXT, enabled INTEGER)",
            "CREATE TABLE agent_speaker_template_grants (agent_id INTEGER, speaker_id INTEGER, template_id INTEGER)",
            "CREATE TABLE speaker_voiceprints (speaker_id INTEGER, embedding_space TEXT, vector BLOB, browser_validation_status TEXT NOT NULL DEFAULT 'passed')",
        ] {
            sqlx::query(ddl).execute(&pool).await.unwrap();
        }

        let set_mode = |mode: &'static str| {
            let pool = pool.clone();
            async move {
                sqlx::query("INSERT INTO agent_speaker_policies (agent_id, mode) VALUES (7, ?)")
                    .bind(mode)
                    .execute(&pool)
                    .await
                    .unwrap();
            }
        };

        // Missing policy → nothing to run.
        assert!(matches!(
            resolve_observe_plan(&pool, 7, 3, "speaker:abc")
                .await
                .unwrap(),
            ObserveResolution::Off
        ));

        // `off` → nothing to run.
        set_mode("off").await;
        assert!(matches!(
            resolve_observe_plan(&pool, 7, 3, "speaker:abc")
                .await
                .unwrap(),
            ObserveResolution::Off
        ));

        // `observe` with no candidates → nothing to run (unchanged behaviour).
        sqlx::query("UPDATE agent_speaker_policies SET mode = 'observe' WHERE agent_id = 7")
            .execute(&pool)
            .await
            .unwrap();
        assert!(matches!(
            resolve_observe_plan(&pool, 7, 3, "speaker:abc")
                .await
                .unwrap(),
            ObserveResolution::Off
        ));

        // `required` with no candidates → admission must fail closed.
        sqlx::query("UPDATE agent_speaker_policies SET mode = 'required' WHERE agent_id = 7")
            .execute(&pool)
            .await
            .unwrap();
        assert!(matches!(
            resolve_observe_plan(&pool, 7, 3, "speaker:abc")
                .await
                .unwrap(),
            ObserveResolution::RequiredUnavailable
        ));
    }

    #[tokio::test]
    async fn required_with_a_provisional_candidate_fails_closed() {
        let pool = sqlx::SqlitePool::connect("sqlite::memory:").await.unwrap();
        for ddl in [
            "CREATE TABLE agent_speaker_policies (agent_id INTEGER, mode TEXT)",
            "CREATE TABLE agent_speaker_candidates (agent_id INTEGER, speaker_id INTEGER)",
            "CREATE TABLE speakers (id INTEGER, key TEXT, enabled INTEGER)",
            "CREATE TABLE agent_speaker_template_grants (agent_id INTEGER, speaker_id INTEGER, template_id INTEGER)",
            "CREATE TABLE speaker_voiceprints (speaker_id INTEGER, embedding_space TEXT, vector BLOB, browser_validation_status TEXT)",
            "CREATE TABLE speaker_catalog (id INTEGER, revision INTEGER)",
        ] {
            sqlx::query(ddl).execute(&pool).await.unwrap();
        }
        for sql in [
            "INSERT INTO agent_speaker_policies VALUES (7, 'required')",
            "INSERT INTO speakers VALUES (1, 'owner', 1)",
            "INSERT INTO agent_speaker_candidates VALUES (7, 1)",
            "INSERT INTO agent_speaker_template_grants VALUES (7, 1, 3)",
            "INSERT INTO speaker_catalog VALUES (1, 1)",
        ] {
            sqlx::query(sql).execute(&pool).await.unwrap();
        }
        sqlx::query("INSERT INTO speaker_voiceprints VALUES (1, 'speaker:abc', ?, 'pending')")
            .bind(enrollment::encode_embedding(&[1.0, 0.0]))
            .execute(&pool)
            .await
            .unwrap();
        assert!(matches!(
            resolve_observe_plan(&pool, 7, 3, "speaker:abc")
                .await
                .unwrap(),
            ObserveResolution::RequiredUnavailable
        ));
    }
}
