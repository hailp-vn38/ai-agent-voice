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
}
pub struct PreparedTemplateProfile {
    pub configuration: ConfiguredTemplateProfile,
    pub runtimes: ResolvedAgentRuntimes,
    pub leases: Vec<ResourceLease>,
}
impl ConfiguredTemplateProfile {
    pub fn from_assignment(assignment: &AdmittedAssignment) -> Result<Self, ProfileUnavailable> {
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
                || !["vad", "asr", "llm", "tts"].contains(&snapshot.kind.as_str())
                || snapshots
                    .insert(snapshot.kind.as_str(), Arc::clone(snapshot))
                    .is_some()
            {
                return Err(ProfileUnavailable);
            }
        }
        let key = |kind| {
            snapshots
                .get(kind)
                .map(|row| row.key.clone())
                .ok_or(ProfileUnavailable)
        };
        let providers = EffectiveProviderBindings {
            vad: key("vad")?,
            asr: key("asr")?,
            llm: key("llm")?,
            tts: key("tts")?,
            vision: None,
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
        self.snapshots
            .iter()
            .map(|row| ProviderVersion::database(row.id, row.revision))
            .collect()
    }
    /// All four slots use one admission deadline. Failure drops every acquired lease before
    /// returning, leaving the current profile untouched. Only the manager may retain warm cache.
    pub async fn prepare(
        &self,
        manager: &Arc<ProviderRuntimeManager>,
        deadline: tokio::time::Instant,
    ) -> Result<PreparedTemplateProfile, RuntimeError> {
        let mut leases = Vec::with_capacity(4);
        let mut catalog = RuntimeCatalog::default();
        for snapshot in &self.snapshots {
            let lease = manager
                .acquire_until(snapshot.as_ref().clone(), deadline)
                .await?;
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

impl std::fmt::Debug for ConfiguredTemplateProfile {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ConfiguredTemplateProfile")
            .field("versions", &self.provider_versions())
            .finish_non_exhaustive()
    }
}
