//! Ticket 17: admission-time authorization for switching a locked Speaker onto another Template.
//!
//! The actor keeps no database handle, so a target Template's grant, Voiceprint and exact-set
//! qualification cannot be re-read mid-session. They are resolved once at admission into a
//! [`SwitchSpeakerAuthority`] per candidate Template and carried beside the session. The actor
//! then only compares the locked identity and the security epoch against that frozen authority,
//! at arm and again at apply. A target with no authority fails closed: no fallback, no auto-enroll
//! and no downgrade to a Template that would drop the gate.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use tokio_util::sync::CancellationToken;

use super::speaker_observe::{ObservePlan, SpeakerPolicyMode};

/// Why a locked-Speaker switch onto a target Template was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SwitchReject {
    /// No admission-time Speaker authority exists for the target Template.
    Unavailable,
    /// The target's exact candidate set has no qualification evidence.
    Unqualified,
    /// The target Template does not grant the locked Speaker a target-space Voiceprint.
    NotGranted,
    /// The target scores in a different embedding space than this session's runtime.
    IncompatibleSpace,
}

impl SwitchReject {
    /// Stable reason code for the tool result and the switch log line.
    pub fn reason(self) -> &'static str {
        match self {
            Self::Unavailable => "target_template_unavailable",
            Self::Unqualified => "target_candidate_set_unqualified",
            Self::NotGranted => "locked_speaker_not_granted",
            Self::IncompatibleSpace => "target_embedding_space_incompatible",
        }
    }
}

/// The frozen authority for keeping the locked Speaker gated on one target Template.
#[derive(Clone, Debug, PartialEq)]
pub struct SwitchSpeakerAuthority {
    /// Target Template id, for the switched profile.
    pub template_id: i64,
    /// Embedding space the target's candidates were resolved in.
    pub embedding_space: Arc<str>,
    /// Digest of the Agent's exact candidate set the qualification was published against.
    pub candidate_set_digest: Option<String>,
    /// Whether that exact candidate set has calibration evidence.
    pub qualified: bool,
    /// Speaker ids granted on the target with a published Voiceprint in its space.
    pub candidates: BTreeSet<i64>,
    /// The target's frozen plan, installed when the switch applies.
    pub plan: ObservePlan,
}

impl SwitchSpeakerAuthority {
    /// Refuse unless the target keeps gating `locked` exactly as the active Template does.
    pub fn authorize(&self, locked: Option<i64>) -> Result<(), SwitchReject> {
        if !self.qualified {
            return Err(SwitchReject::Unqualified);
        }
        if locked.is_some_and(|locked| !self.candidates.contains(&locked)) {
            return Err(SwitchReject::NotGranted);
        }
        Ok(())
    }
}

/// The session's frozen switch authority: the active policy and space, plus one authority per
/// candidate Template. `None` on the actor means the session is speaker-free and membership is
/// the whole switch rule.
#[derive(Clone, Debug)]
pub struct SpeakerSwitchGuard {
    policy: SpeakerPolicyMode,
    embedding_space: Arc<str>,
    authorities: BTreeMap<String, Arc<SwitchSpeakerAuthority>>,
    security: Arc<CancellationToken>,
}

impl SpeakerSwitchGuard {
    pub fn new(
        policy: SpeakerPolicyMode,
        embedding_space: Arc<str>,
        authorities: BTreeMap<String, Arc<SwitchSpeakerAuthority>>,
        security: Arc<CancellationToken>,
    ) -> Self {
        Self {
            policy,
            embedding_space,
            authorities,
            security,
        }
    }

    pub fn authorities(&self) -> &BTreeMap<String, Arc<SwitchSpeakerAuthority>> {
        &self.authorities
    }

    /// Observe recognition is advisory: a target without a usable Speaker plan remains a valid
    /// anonymous Template switch. Required keeps the authority checks below.
    pub fn allows_anonymous_target(&self) -> bool {
        self.policy == SpeakerPolicyMode::Observe
    }

    /// Refuse unless the target Template keeps the locked Speaker gated. The security epoch is
    /// rechecked here so a revocation between arm and apply still drops the switch.
    pub fn authorize(
        &self,
        template_key: &str,
        locked: Option<i64>,
    ) -> Result<Arc<SwitchSpeakerAuthority>, SwitchReject> {
        if !self.policy.observe_enabled() || self.security.is_cancelled() {
            return Err(SwitchReject::Unavailable);
        }
        let authority = self
            .authorities
            .get(template_key)
            .ok_or(SwitchReject::Unavailable)?;
        authority.authorize(locked)?;
        if authority.embedding_space.as_ref() != self.embedding_space.as_ref() {
            return Err(SwitchReject::IncompatibleSpace);
        }
        Ok(Arc::clone(authority))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::speaker_observe::ObserveCandidate;

    fn plan(template_id: i64, space: &str, speakers: &[i64]) -> ObservePlan {
        ObservePlan {
            agent_id: 1,
            template_id,
            embedding_space: space.to_owned(),
            catalog_revision: 1,
            policy: SpeakerPolicyMode::Required,
            candidates: speakers
                .iter()
                .map(|id| ObserveCandidate {
                    speaker_id: *id,
                    key: format!("speaker-{id}"),
                    vector: vec![1.0],
                })
                .collect(),
        }
    }

    fn authority(
        template_id: i64,
        space: &str,
        qualified: bool,
        speakers: &[i64],
    ) -> SwitchSpeakerAuthority {
        SwitchSpeakerAuthority {
            template_id,
            embedding_space: Arc::from(space),
            candidate_set_digest: Some("digest".into()),
            qualified,
            candidates: speakers.iter().copied().collect(),
            plan: plan(template_id, space, speakers),
        }
    }

    fn guard(authorities: Vec<(&str, SwitchSpeakerAuthority)>) -> SpeakerSwitchGuard {
        SpeakerSwitchGuard::new(
            SpeakerPolicyMode::Required,
            Arc::from("space-a"),
            authorities
                .into_iter()
                .map(|(key, authority)| (key.to_owned(), Arc::new(authority)))
                .collect(),
            Arc::new(CancellationToken::new()),
        )
    }

    #[test]
    fn a_granted_qualified_same_space_target_is_authorized() {
        let guard = guard(vec![("b", authority(2, "space-a", true, &[7]))]);
        assert!(guard.authorize("b", Some(7)).is_ok());
    }

    #[test]
    fn a_target_that_does_not_grant_the_locked_speaker_is_refused() {
        let guard = guard(vec![("b", authority(2, "space-a", true, &[9]))]);
        assert_eq!(guard.authorize("b", Some(7)), Err(SwitchReject::NotGranted));
    }

    #[test]
    fn an_unqualified_target_candidate_set_is_refused() {
        let guard = guard(vec![("b", authority(2, "space-a", false, &[7]))]);
        assert_eq!(
            guard.authorize("b", Some(7)),
            Err(SwitchReject::Unqualified)
        );
    }

    #[test]
    fn a_target_without_an_authority_fails_closed() {
        let guard = guard(vec![]);
        assert_eq!(
            guard.authorize("b", Some(7)),
            Err(SwitchReject::Unavailable)
        );
    }

    #[test]
    fn a_different_embedding_space_is_refused() {
        let guard = guard(vec![("b", authority(2, "space-b", true, &[7]))]);
        assert_eq!(
            guard.authorize("b", Some(7)),
            Err(SwitchReject::IncompatibleSpace)
        );
    }

    #[test]
    fn a_cancelled_security_epoch_fails_closed() {
        let security = Arc::new(CancellationToken::new());
        security.cancel();
        let guard = SpeakerSwitchGuard::new(
            SpeakerPolicyMode::Required,
            Arc::from("space-a"),
            [(
                "b".to_owned(),
                Arc::new(authority(2, "space-a", true, &[7])),
            )]
            .into_iter()
            .collect(),
            security,
        );
        assert_eq!(
            guard.authorize("b", Some(7)),
            Err(SwitchReject::Unavailable)
        );
    }

    #[test]
    fn an_unlocked_speaker_needs_only_a_qualified_same_space_target() {
        let guard = guard(vec![("b", authority(2, "space-a", true, &[]))]);
        assert!(guard.authorize("b", None).is_ok());
    }

    #[test]
    fn observe_can_switch_to_an_anonymous_target() {
        let guard = SpeakerSwitchGuard::new(
            SpeakerPolicyMode::Observe,
            Arc::from("space-a"),
            BTreeMap::new(),
            Arc::new(CancellationToken::new()),
        );

        assert!(guard.allows_anonymous_target());
    }
}
