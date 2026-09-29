//! Effective Session Profile: the one immutable configuration snapshot a Voice Session may use.
//!
//! Resolution happens once, before the WebSocket upgrade, and reads only the admission graph plus
//! the process-wide Loaded Runtime catalog.  Nothing here keeps a repository, a pool or a row, so
//! Admin mutation and database degradation cannot live-reconfigure an open session.

use std::collections::BTreeMap;

use tracing::warn;

use crate::{
    config::{AppConfig, EffectiveProviderBindings},
    database::AdmittedAssignment,
    providers::RuntimeCatalog,
};

use super::prompt;

/// A Template prompt is a rendered system prompt, not a template: it is bounded by the same limit
/// as a deployment-rendered one so a stored row can never grow the request without bound.
pub const MAX_TEMPLATE_PROMPT_BYTES: usize = crate::config::MAX_RENDERED_SYSTEM_PROMPT_BYTES;

/// Where the session's prompt, language and providers came from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProfileSource {
    /// The Agent has no assignment row at all, so the deployment's server defaults apply.
    ServerDefault,
    Template {
        template_id: i64,
        template_key: String,
        template_name: String,
        template_revision: i64,
    },
}

/// One enabled Template whose bindings all have Loaded Runtime.  Invalid candidates never enter
/// the catalog, so a switch can only ever select a profile that already works.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResolvedTemplateProfile {
    pub template_id: i64,
    pub template_key: String,
    pub template_name: String,
    pub language: String,
    pub system_prompt: String,
    pub providers: EffectiveProviderBindings,
}

/// Immutable candidate list for this session only.  It is a snapshot, never a query.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TemplateSwitchCatalog {
    candidates: Vec<ResolvedTemplateProfile>,
}

impl TemplateSwitchCatalog {
    pub fn candidates(&self) -> &[ResolvedTemplateProfile] {
        &self.candidates
    }

    fn insert(&mut self, candidate: ResolvedTemplateProfile) {
        if !self
            .candidates
            .iter()
            .any(|existing| existing.template_id == candidate.template_id)
        {
            self.candidates.push(candidate);
        }
    }

    pub fn is_empty(&self) -> bool {
        self.candidates.is_empty()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EffectiveSessionProfile {
    pub device_db_id: i64,
    pub agent_id: i64,
    pub agent_key: String,
    pub source: ProfileSource,
    pub language: String,
    pub system_prompt: String,
    pub providers: EffectiveProviderBindings,
    /// Session Profile Revision starts at one and only advances on a successful switch.
    pub revision: u64,
    pub switch_catalog: TemplateSwitchCatalog,
}

/// Coarse admission outcome.  The client only ever learns "unavailable"; the reason stays internal
/// so a broken default Template can never leak its configuration.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[error("agent_profile_unavailable")]
pub struct ProfileUnavailable;

impl EffectiveSessionProfile {
    /// The deployment's own agent configuration, used whenever no database Template applies.
    pub fn server_default(config: &AppConfig) -> Result<Self, ProfileUnavailable> {
        let agent = config.effective_agent();
        Ok(Self {
            device_db_id: 0,
            agent_id: 0,
            agent_key: String::new(),
            source: ProfileSource::ServerDefault,
            language: agent.language.clone(),
            system_prompt: prompt::render_system(agent).map_err(|_| ProfileUnavailable)?,
            providers: agent.providers.clone(),
            revision: 1,
            // An Agent without assignments must not advertise a switch capability it cannot honor.
            switch_catalog: TemplateSwitchCatalog::default(),
        })
    }
}

/// Resolves exactly one Effective Session Profile for an admitted Device.
///
/// Any assignment row at all means the Agent entered the Template mechanism, so there is no path
/// back to server defaults: a missing, disabled or incomplete default Template is `Unavailable`.
pub fn resolve_effective_session_profile(
    device_db_id: i64,
    agent_id: i64,
    agent_key: &str,
    assignments: &[AdmittedAssignment],
    config: &AppConfig,
    runtimes: &RuntimeCatalog,
) -> Result<EffectiveSessionProfile, ProfileUnavailable> {
    if assignments.is_empty() {
        let mut profile = EffectiveSessionProfile::server_default(config)?;
        profile.device_db_id = device_db_id;
        profile.agent_id = agent_id;
        profile.agent_key = agent_key.to_owned();
        return Ok(profile);
    }

    let default = assignments
        .iter()
        .find(|assignment| assignment.is_default && assignment.assignment_enabled)
        .ok_or(ProfileUnavailable)?;
    let active = resolve_template(default, runtimes).map_err(|error| {
        warn!(
            agent_key,
            template_key = %default.template_key,
            reason = %error,
            "the default template cannot be resolved; refusing to fall back to server defaults"
        );
        ProfileUnavailable
    })?;

    let mut switch_catalog = TemplateSwitchCatalog::default();
    for assignment in assignments.iter().filter(|entry| entry.assignment_enabled) {
        match resolve_template(assignment, runtimes) {
            Ok(candidate) => switch_catalog.insert(candidate),
            Err(reason) => warn!(
                agent_key,
                template_key = %assignment.template_key,
                reason = %reason,
                "excluded an invalid non-default template candidate from the session catalog"
            ),
        }
    }

    Ok(EffectiveSessionProfile {
        device_db_id,
        agent_id,
        agent_key: agent_key.to_owned(),
        source: ProfileSource::Template {
            template_id: default.template_id,
            template_key: default.template_key.clone(),
            template_name: default.template_name.clone(),
            template_revision: default.template_revision,
        },
        language: active.language,
        system_prompt: active.system_prompt,
        providers: active.providers,
        revision: 1,
        switch_catalog,
    })
}

/// A candidate is valid only when its Template is enabled, its prompt is bounded, all four
/// provider slots are bound to enabled providers, and every one of them has a Loaded Runtime.
fn resolve_template(
    assignment: &AdmittedAssignment,
    runtimes: &RuntimeCatalog,
) -> Result<ResolvedTemplateProfile, TemplateCandidateError> {
    if !assignment.template_enabled {
        return Err(TemplateCandidateError::TemplateDisabled);
    }
    if assignment.language.trim().is_empty() {
        return Err(TemplateCandidateError::LanguageMissing);
    }
    if assignment.prompt.is_empty() || assignment.prompt.len() > MAX_TEMPLATE_PROMPT_BYTES {
        return Err(TemplateCandidateError::PromptInvalid);
    }
    let mut bound: BTreeMap<&str, &str> = BTreeMap::new();
    for binding in &assignment.bindings {
        if !binding.provider_enabled {
            return Err(TemplateCandidateError::ProviderDisabled);
        }
        bound.insert(
            binding.provider_type.as_str(),
            binding.provider_key.as_str(),
        );
    }
    let providers = EffectiveProviderBindings {
        vad: bound_key(bound.get("vad").copied())?,
        asr: bound_key(bound.get("asr").copied())?,
        llm: bound_key(bound.get("llm").copied())?,
        tts: bound_key(bound.get("tts").copied())?,
        vision: None,
    };
    // One resolution proves all four slots resolve to runtimes this process actually loaded.
    runtimes
        .resolve(&providers)
        .map_err(|_| TemplateCandidateError::RuntimeMissing)?;
    Ok(ResolvedTemplateProfile {
        template_id: assignment.template_id,
        template_key: assignment.template_key.clone(),
        template_name: assignment.template_name.clone(),
        language: assignment.language.clone(),
        system_prompt: assignment.prompt.clone(),
        providers,
    })
}

/// Every required slot must be bound; a Template with a missing slot is invalid rather than
/// partially defaulted.
fn bound_key(key: Option<&str>) -> Result<String, TemplateCandidateError> {
    key.map(str::to_owned)
        .ok_or(TemplateCandidateError::BindingMissing)
}

/// Bounded reason classes.  Only these strings reach telemetry; no key, prompt or config content
/// is logged.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
enum TemplateCandidateError {
    #[error("template_disabled")]
    TemplateDisabled,
    #[error("language_missing")]
    LanguageMissing,
    #[error("prompt_invalid")]
    PromptInvalid,
    #[error("provider_binding_missing")]
    BindingMissing,
    #[error("provider_disabled")]
    ProviderDisabled,
    #[error("provider_runtime_unavailable")]
    RuntimeMissing,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{audio::VadSegmenterConfig, database::AdmittedProviderBinding};

    fn config() -> AppConfig {
        toml::from_str(
            r#"
            [server]
            bind = "127.0.0.1:0"
            public_ws_url = "ws://127.0.0.1:0/voice/v1/"

            [provider_defaults]
            vad = "vad"
            asr = "asr"
            llm = "llm"
            tts = "tts"
            "#,
        )
        .unwrap()
    }

    fn assignment(default: bool, enabled: bool, bindings: Vec<(&str, &str)>) -> AdmittedAssignment {
        AdmittedAssignment {
            template_id: 4,
            template_key: "support".into(),
            template_name: "Support".into(),
            language: "vi-VN".into(),
            prompt: "stored system prompt".into(),
            template_enabled: true,
            template_revision: 2,
            is_default: default,
            assignment_enabled: enabled,
            bindings: bindings
                .into_iter()
                .map(|(provider_type, provider_key)| AdmittedProviderBinding {
                    provider_type: provider_type.into(),
                    provider_key: provider_key.into(),
                    provider_enabled: true,
                })
                .collect(),
        }
    }

    fn full_bindings() -> Vec<(&'static str, &'static str)> {
        vec![
            ("vad", "vad"),
            ("asr", "asr"),
            ("llm", "llm"),
            ("tts", "tts"),
        ]
    }

    #[test]
    fn only_an_agent_without_any_assignment_row_uses_server_defaults() {
        let profile = resolve_effective_session_profile(
            1,
            2,
            "agent",
            &[assignment(true, false, full_bindings())],
            &config(),
            &RuntimeCatalog::default(),
        );
        assert_eq!(profile, Err(ProfileUnavailable));

        let server_defaults = resolve_effective_session_profile(
            1,
            2,
            "agent",
            &[],
            &config(),
            &RuntimeCatalog::default(),
        )
        .expect("an unassigned agent keeps deployment defaults");
        assert_eq!(server_defaults.source, ProfileSource::ServerDefault);
        assert!(server_defaults.switch_catalog.is_empty());
    }

    #[test]
    fn a_default_template_without_loaded_runtimes_fails_closed() {
        let profile = resolve_effective_session_profile(
            1,
            2,
            "agent",
            &[assignment(true, true, full_bindings())],
            &config(),
            &RuntimeCatalog::default(),
        );
        assert_eq!(profile, Err(ProfileUnavailable));
    }

    #[test]
    fn an_oversized_stored_prompt_is_never_accepted() {
        let mut oversized = assignment(true, true, full_bindings());
        oversized.prompt = "p".repeat(MAX_TEMPLATE_PROMPT_BYTES + 1);
        assert_eq!(
            resolve_template(&oversized, &RuntimeCatalog::default()).unwrap_err(),
            TemplateCandidateError::PromptInvalid
        );
    }

    #[test]
    fn a_missing_binding_never_produces_a_mixed_profile() {
        let partial = assignment(true, true, vec![("asr", "asr"), ("llm", "llm")]);
        assert_eq!(
            resolve_template(&partial, &loaded_catalog()).unwrap_err(),
            TemplateCandidateError::BindingMissing
        );
    }

    /// One loaded runtime per server default instance id, so a stored binding to any of them
    /// resolves and a binding to anything else cannot.
    fn loaded_catalog() -> RuntimeCatalog {
        use crate::providers::{
            LoadedVad, asr::UnavailableAsr, llm::UnavailableLlm, tts::UnavailableTts,
            vad::UnavailableVad,
        };
        use crate::workers::{
            AsrWorkerRuntime, LlmRuntime, TtsWorkerRuntime, VadWorkerRuntime, WorkerRuntimeConfig,
        };
        use std::{collections::HashMap, sync::Arc, time::Duration};

        let worker = WorkerRuntimeConfig {
            max_workers: 1,
            command_capacity: 1,
            final_timeout: Duration::from_secs(1),
            cleanup_grace: Duration::from_secs(1),
        };
        RuntimeCatalog {
            vad: HashMap::from([(
                "vad".to_owned(),
                LoadedVad {
                    runtime: Arc::new(VadWorkerRuntime::new(
                        Arc::new(UnavailableVad),
                        worker.clone(),
                    )),
                    segmenter: VadSegmenterConfig::default(),
                    pre_roll_samples: 4_800,
                },
            )]),
            asr: HashMap::from([(
                "asr".to_owned(),
                Arc::new(AsrWorkerRuntime::new(
                    Arc::new(UnavailableAsr),
                    worker.clone(),
                )),
            )]),
            llm: HashMap::from([(
                "llm".to_owned(),
                Arc::new(LlmRuntime::new(
                    Arc::new(UnavailableLlm),
                    1,
                    Duration::from_secs(1),
                )),
            )]),
            tts: HashMap::from([(
                "tts".to_owned(),
                Arc::new(TtsWorkerRuntime::new(Arc::new(UnavailableTts), worker)),
            )]),
            vision: HashMap::new(),
        }
    }

    fn named(id: i64, key: &str, default: bool, enabled: bool) -> AdmittedAssignment {
        AdmittedAssignment {
            template_id: id,
            template_key: key.into(),
            ..assignment(default, enabled, full_bindings())
        }
    }

    #[test]
    fn the_switch_catalog_excludes_every_invalid_candidate() {
        let mut unloaded = named(2, "unloaded", false, true);
        unloaded
            .bindings
            .retain(|binding| binding.provider_type != "llm");
        unloaded.bindings.push(AdmittedProviderBinding {
            provider_type: "llm".into(),
            provider_key: "never-loaded".into(),
            provider_enabled: true,
        });
        let mut incomplete = named(3, "incomplete", false, true);
        incomplete
            .bindings
            .retain(|binding| binding.provider_type != "tts");
        let mut disabled_provider = named(4, "disabled_provider", false, true);
        disabled_provider.bindings[0].provider_enabled = false;
        let mut disabled_template = named(5, "disabled_template", false, true);
        disabled_template.template_enabled = false;
        let disabled_assignment = named(6, "disabled_assignment", false, false);

        let profile = resolve_effective_session_profile(
            1,
            2,
            "agent",
            &[
                named(1, "primary", true, true),
                named(7, "usable", false, true),
                unloaded,
                incomplete,
                disabled_provider,
                disabled_template,
                disabled_assignment,
            ],
            &config(),
            &loaded_catalog(),
        )
        .expect("a valid default template still admits the session");
        let keys = profile
            .switch_catalog
            .candidates()
            .iter()
            .map(|candidate| candidate.template_key.clone())
            .collect::<Vec<_>>();
        assert_eq!(keys, vec!["primary".to_owned(), "usable".to_owned()]);
    }

    #[test]
    fn a_default_template_resolves_its_own_prompt_language_and_bindings() {
        let profile = resolve_effective_session_profile(
            1,
            2,
            "agent",
            &[named(1, "primary", true, true)],
            &config(),
            &loaded_catalog(),
        )
        .expect("the default template has every binding loaded");
        assert_eq!(profile.system_prompt, "stored system prompt");
        assert_eq!(profile.language, "vi-VN");
        assert_eq!(profile.providers.llm, "llm");
        assert_eq!(profile.revision, 1);
        assert_eq!(
            profile.source,
            ProfileSource::Template {
                template_id: 1,
                template_key: "primary".into(),
                template_name: "Support".into(),
                template_revision: 2,
            }
        );
    }
}
