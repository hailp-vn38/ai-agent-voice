//! Immutable cold candidates. Preparing acquires exact admitted versions outside the actor.
use super::{ProfileSource, ProfileUnavailable, profile::MAX_TEMPLATE_PROMPT_BYTES};
use crate::{
    config::EffectiveProviderBindings,
    database::{AdmittedAssignment, DesiredProvider},
    providers::{ResolvedAgentRuntimes, RuntimeCatalog},
    services::provider_runtime::{
        ProviderRuntimeManager, ProviderVersion, ResourceLease, RuntimeError,
    },
};
use std::{collections::BTreeMap, sync::Arc};

#[derive(Clone)]
pub struct ConfiguredTemplateProfile {
    pub(super) source: ProfileSource,
    pub(super) language: String,
    pub(super) system_prompt: String,
    pub(super) providers: EffectiveProviderBindings,
    snapshots: Vec<Arc<DesiredProvider>>,
    deployment_snapshots: Vec<DesiredProvider>,
}
pub struct PreparedTemplateProfile {
    pub configuration: ConfiguredTemplateProfile,
    pub runtimes: ResolvedAgentRuntimes,
    pub leases: Vec<ResourceLease>,
}
impl ConfiguredTemplateProfile {
    pub fn from_assignment(assignment: &AdmittedAssignment) -> Result<Self, ProfileUnavailable> {
        Self::from_assignment_internal(assignment, None)
    }

    pub fn from_assignment_with_defaults(
        assignment: &AdmittedAssignment,
        defaults: &EffectiveProviderBindings,
        deployment: &[DesiredProvider],
    ) -> Result<Self, ProfileUnavailable> {
        Self::from_assignment_internal(assignment, Some((defaults, deployment)))
    }

    fn from_assignment_internal(
        assignment: &AdmittedAssignment,
        fallback: Option<(&EffectiveProviderBindings, &[DesiredProvider])>,
    ) -> Result<Self, ProfileUnavailable> {
        if !assignment.assignment_enabled
            || !assignment.template_enabled
            || assignment.language.trim().is_empty()
            || assignment.prompt.is_empty()
            || assignment.prompt.len() > MAX_TEMPLATE_PROMPT_BYTES
        {
            return Err(ProfileUnavailable);
        }
        let mut snapshots = BTreeMap::new();
        for binding in &assignment.bindings {
            if !binding.provider_enabled {
                return Err(ProfileUnavailable);
            }
            let snapshot = binding.snapshot.as_ref().ok_or(ProfileUnavailable)?;
            if snapshot.id <= 0
                || snapshot.revision <= 0
                || snapshot.key != binding.provider_key
                || snapshot.kind != binding.provider_type
                || !["vad", "asr", "llm", "tts", "speaker"].contains(&snapshot.kind.as_str())
                || snapshots
                    .insert(snapshot.kind.as_str(), Arc::clone(snapshot))
                    .is_some()
            {
                return Err(ProfileUnavailable);
            }
        }
        let mut deployment_snapshots = Vec::new();
        let mut key = |kind: &str, default: Option<&str>| {
            if let Some(row) = snapshots.get(kind) {
                return Ok(row.key.clone());
            }
            let (_, deployment) = fallback.ok_or(ProfileUnavailable)?;
            let key = default
                .filter(|key| !key.is_empty())
                .ok_or(ProfileUnavailable)?;
            let snapshot = deployment
                .iter()
                .find(|row| row.kind == kind && row.key == key)
                .ok_or(ProfileUnavailable)?;
            deployment_snapshots.push(snapshot.clone());
            Ok(key.to_owned())
        };
        let defaults = fallback.map(|(defaults, _)| defaults);
        let providers = EffectiveProviderBindings {
            vad: key("vad", defaults.map(|value| value.vad.as_str()))?,
            asr: key("asr", defaults.map(|value| value.asr.as_str()))?,
            llm: key("llm", defaults.map(|value| value.llm.as_str()))?,
            tts: key("tts", defaults.map(|value| value.tts.as_str()))?,
            vision: None,
            speaker: snapshots.get("speaker").map(|row| row.key.clone()),
        };
        Ok(Self {
            source: ProfileSource::Template {
                template_id: assignment.template_id,
                template_key: assignment.template_key.clone(),
                template_name: assignment.template_name.clone(),
                template_revision: assignment.template_revision,
            },
            language: assignment.language.clone(),
            system_prompt: assignment.prompt.clone(),
            providers,
            snapshots: snapshots.into_values().collect(),
            deployment_snapshots,
        })
    }
    pub fn template_key(&self) -> &str {
        match &self.source {
            ProfileSource::Template { template_key, .. } => template_key,
            ProfileSource::ServerDefault => {
                unreachable!("configured candidates require assignments")
            }
        }
    }
    pub fn provider_versions(&self) -> Vec<ProviderVersion> {
        let mut versions: Vec<_> = self
            .snapshots
            .iter()
            .map(|row| ProviderVersion::database(row.id, row.revision))
            .collect();
        versions.extend(self.deployment_snapshots.iter().map(|row| ProviderVersion {
            identity: crate::services::provider_runtime::ProviderIdentity::Deployment {
                kind: row.kind.clone(),
                key: row.key.clone(),
            },
            revision: row.revision,
        }));
        versions
    }
    /// All four slots use one admission deadline. Failure drops every acquired lease before
    /// returning, leaving the current profile untouched. Only the manager may retain warm cache.
    pub async fn prepare(
        &self,
        manager: &Arc<ProviderRuntimeManager>,
        deadline: tokio::time::Instant,
    ) -> Result<PreparedTemplateProfile, RuntimeError> {
        let mut leases = Vec::with_capacity(5);
        let mut catalog = RuntimeCatalog::default();
        for snapshot in &self.snapshots {
            let lease = manager
                .acquire_until(snapshot.as_ref().clone(), deadline)
                .await?;
            log_provider_loaded(snapshot, lease.version(), "database");
            let resident = lease.runtimes().ok_or(RuntimeError::Unavailable)?;
            catalog.vad.extend(resident.vad);
            catalog.asr.extend(resident.asr);
            catalog.llm.extend(resident.llm);
            catalog.tts.extend(resident.tts);
            catalog.speaker.extend(resident.speaker);
            leases.push(lease);
        }
        for snapshot in &self.deployment_snapshots {
            let lease = manager
                .acquire_deployment_until(snapshot.clone(), deadline)
                .await?;
            log_provider_loaded(snapshot, lease.version(), "deployment");
            let resident = lease.runtimes().ok_or(RuntimeError::Unavailable)?;
            catalog.vad.extend(resident.vad);
            catalog.asr.extend(resident.asr);
            catalog.llm.extend(resident.llm);
            catalog.tts.extend(resident.tts);
            leases.push(lease);
        }
        let runtimes = catalog
            .resolve(&self.providers)
            .map_err(|_| RuntimeError::Unavailable)?;
        Ok(PreparedTemplateProfile {
            configuration: self.clone(),
            runtimes,
            leases,
        })
    }
}

/// Names the provider a Voice Session just acquired, by kind and instance key.
///
/// A session loads up to four providers, and several sessions can share one physical runtime, so
/// without the kind and key an operator reading the log cannot tell which provider a line refers
/// to, nor whether this connection was the one that paid for the load.
fn log_provider_loaded(snapshot: &DesiredProvider, version: &ProviderVersion, source: &str) {
    tracing::info!(
        provider_kind = %snapshot.kind,
        provider_key = %snapshot.key,
        adapter = %snapshot.adapter,
        provider_revision = snapshot.revision,
        source,
        ?version,
        "voice session loaded provider"
    );
}

impl std::fmt::Debug for ConfiguredTemplateProfile {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ConfiguredTemplateProfile")
            .field("versions", &self.provider_versions())
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::database::AdmittedProviderBinding;

    fn provider(id: i64, kind: &str) -> Arc<DesiredProvider> {
        Arc::new(DesiredProvider {
            id,
            key: kind.to_owned(),
            kind: kind.to_owned(),
            adapter: "test".into(),
            config_json: "{}".into(),
            secret_ref: None,
            revision: 1,
        })
    }

    fn assignment(kinds: &[&str]) -> AdmittedAssignment {
        AdmittedAssignment {
            template_id: 1,
            template_key: "t".into(),
            template_name: "T".into(),
            language: "en".into(),
            prompt: "hello".into(),
            template_enabled: true,
            template_revision: 1,
            is_default: true,
            assignment_enabled: true,
            bindings: kinds
                .iter()
                .map(|kind| AdmittedProviderBinding {
                    provider_type: (*kind).to_owned(),
                    provider_key: (*kind).to_owned(),
                    provider_enabled: true,
                    snapshot: Some(provider(1, kind)),
                })
                .collect(),
        }
    }

    #[test]
    fn a_speaker_binding_is_admitted_as_an_optional_slot() {
        let assignment = assignment(&["vad", "asr", "llm", "tts", "speaker"]);
        let profile = ConfiguredTemplateProfile::from_assignment(&assignment).unwrap();
        assert_eq!(profile.providers.speaker.as_deref(), Some("speaker"));
    }

    #[test]
    fn an_absent_speaker_slot_is_not_filled_from_a_deployment_default() {
        let assignment = assignment(&["vad", "asr", "llm", "tts"]);
        let profile = ConfiguredTemplateProfile::from_assignment(&assignment).unwrap();
        assert_eq!(profile.providers.speaker, None);
    }

    #[test]
    fn a_speaker_binding_must_match_the_provider_kind() {
        let mut assignment = assignment(&["vad", "asr", "llm", "tts"]);
        assignment.bindings.push(AdmittedProviderBinding {
            provider_type: "speaker".into(),
            provider_key: "vad".into(),
            provider_enabled: true,
            snapshot: Some(provider(9, "vad")),
        });
        assert!(ConfiguredTemplateProfile::from_assignment(&assignment).is_err());
    }
}
