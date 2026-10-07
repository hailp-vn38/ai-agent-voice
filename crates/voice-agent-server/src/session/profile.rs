//! Effective Session Profile: the one immutable configuration snapshot a Voice Session may use.
//!
//! Resolution happens once, before the WebSocket upgrade, and reads only the admission graph plus
//! the process-wide Loaded Runtime catalog.  Nothing here keeps a repository, a pool or a row, so
//! Admin mutation and database degradation cannot live-reconfigure an open session.

use super::ConfiguredTemplateProfile;
use crate::services::provider_runtime::{ProviderRuntimeManager, ResourceLease, RuntimeError};
use std::{collections::BTreeMap, sync::Arc};

use tracing::warn;

use crate::{
    config::{AppConfig, EffectiveProviderBindings},
    database::AdmittedAssignment,
    providers::{ResolvedAgentRuntimes, RuntimeCatalog},
    tools::external_mcp::SessionExternalMcp,
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

impl ProfileSource {
    /// The active Template id, or `None` when the session runs on server defaults.
    pub fn template_id(&self) -> Option<i64> {
        match self {
            Self::Template { template_id, .. } => Some(*template_id),
            Self::ServerDefault => None,
        }
    }
}

/// One enabled Template whose bindings all have Loaded Runtime.  Invalid candidates never enter
/// the catalog, so a switch can only ever select a profile that already works.
#[derive(Clone)]
pub struct ResolvedTemplateProfile {
    pub template_id: i64,
    pub template_key: String,
    pub template_name: String,
    pub language: String,
    pub system_prompt: String,
    pub providers: EffectiveProviderBindings,
    /// The already-loaded runtime handles this Template names.  A switch installs these; it
    /// never constructs one, so switching can never hot-load a Provider.
    pub runtimes: ResolvedAgentRuntimes,
    /// The stored Template revision, reported as provenance only.  It never becomes the
    /// Session Profile Revision and never leaves the session.
    template_revision: i64,
}

impl ResolvedTemplateProfile {
    fn source(&self) -> ProfileSource {
        ProfileSource::Template {
            template_id: self.template_id,
            template_key: self.template_key.clone(),
            template_name: self.template_name.clone(),
            template_revision: self.template_revision,
        }
    }
}

/// Runtime handles are process-owned and carry no profile meaning, so diagnostics describe the
/// stored Template only.  A prompt or provider key must never reach a log through this type.
impl std::fmt::Debug for ResolvedTemplateProfile {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ResolvedTemplateProfile")
            .field("template_id", &self.template_id)
            .field("template_key", &self.template_key)
            .field("template_name", &self.template_name)
            .field("language", &self.language)
            .finish_non_exhaustive()
    }
}

/// Immutable candidate list for this session only.  It is a snapshot, never a query.
#[derive(Clone, Default)]
pub struct TemplateSwitchCatalog {
    candidates: Vec<ResolvedTemplateProfile>,
    pub(super) cold: Vec<ConfiguredTemplateProfile>,
    pub(super) manager: Option<Arc<ProviderRuntimeManager>>,
    pub(super) active_leases: Vec<ResourceLease>,
}

impl TemplateSwitchCatalog {
    pub(crate) fn hold_runtime_leases(&mut self, leases: Vec<ResourceLease>) {
        self.active_leases = leases;
    }
    pub fn candidates(&self) -> &[ResolvedTemplateProfile] {
        &self.candidates
    }

    /// The lease that keeps a specific selected speaker runtime resident, if this session holds it.
    /// Observe borrows the session's existing lease rather than acquiring a second one.
    pub(crate) fn lease_for_speaker(
        &self,
        runtime: &Arc<crate::workers::SpeakerRuntime>,
    ) -> Option<ResourceLease> {
        self.active_leases
            .iter()
            .find(|lease| {
                lease
                    .runtimes()
                    .map(|catalog| catalog.holds_speaker(runtime))
                    .unwrap_or(false)
            })
            .cloned()
    }

    /// Membership is the whole authorization rule for a switch: an assignment this session never
    /// admitted, or one it excluded as invalid, simply is not here.
    pub fn find(&self, template_key: &str) -> Option<&ResolvedTemplateProfile> {
        self.candidates
            .iter()
            .find(|candidate| candidate.template_key == template_key)
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
        self.candidates.is_empty() && self.cold.is_empty()
    }

    /// The switch tool is advertised only when this session can actually honor it, so the model
    /// is told exactly which Templates it may ask for.
    pub fn template_keys(&self) -> Vec<&str> {
        self.candidates
            .iter()
            .map(|candidate| candidate.template_key.as_str())
            .chain(
                self.cold
                    .iter()
                    .map(ConfiguredTemplateProfile::template_key),
            )
            .collect()
    }
}

/// The one configuration snapshot a Voice Session currently runs with.
///
/// Admission installs it; a successful switch replaces it atomically together with the Session
/// Profile Revision.  It never changes while a Conversational Turn is in flight.  External MCP
/// tools are part of this snapshot but not of this struct: they are fixed for the whole session,
/// including across a switch, so they travel beside the profile rather than inside it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ActiveTemplateProfile {
    pub source: ProfileSource,
    pub language: String,
    pub system_prompt: String,
    pub providers: EffectiveProviderBindings,
    pub revision: u64,
}

impl ActiveTemplateProfile {
    /// The deployment's own agent configuration, used before an admission profile is installed
    /// and by the manual-only constructor seams that never reach a database.
    pub fn server_default() -> Self {
        ActiveTemplateProfile {
            source: ProfileSource::ServerDefault,
            language: String::new(),
            system_prompt: prompt::render_system(&crate::config::EffectiveAgentConfig::default())
                .expect("built-in prompt template is valid"),
            providers: EffectiveProviderBindings {
                vad: String::new(),
                asr: String::new(),
                llm: String::new(),
                tts: String::new(),
                vision: None,
                speaker: None,
            },
            revision: 1,
        }
    }

    /// Replaces this snapshot with a candidate the session already admitted and advances the
    /// Session Profile Revision.  It cannot fail, so no partially switched profile exists: the
    /// caller installs the candidate's runtime handles in the same boundary step.
    pub fn switched_to(&mut self, candidate: &ResolvedTemplateProfile) {
        self.source = candidate.source();
        self.language = candidate.language.clone();
        self.system_prompt = candidate.system_prompt.clone();
        self.providers = candidate.providers.clone();
        self.revision = self.revision.saturating_add(1);
    }
}

#[derive(Clone, Default)]
/// The Device tool review this Voice Session was admitted under.
///
/// Admission resolves it once, beside the External MCP snapshot, and nothing replaces it mid
/// session: a Template switch changes prompt, language and providers, not tool rights.
pub struct SessionDeviceTools {
    pub guard: Option<std::sync::Arc<crate::database::tool_security::DeviceToolGuard>>,
}

#[derive(Clone)]
pub struct EffectiveSessionProfile {
    pub selected_runtimes: Option<ResolvedAgentRuntimes>,
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
    /// Filled in by admission after the External MCP snapshot resolves.  The synchronous
    /// resolver cannot produce it, so it is always empty here and never read from.
    external_mcp: SessionExternalMcp,
    /// Filled in by admission after the Device review resolves, for the same reason.
    device_tools: SessionDeviceTools,
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
            selected_runtimes: None,
            device_db_id: 0,
            agent_id: 0,
            agent_key: String::new(),
            source: ProfileSource::ServerDefault,
            language: agent.language.clone(),
            system_prompt: prompt::render_system(agent).map_err(|_| ProfileUnavailable)?,
            providers: config.provider_defaults.effective_bindings(),
            revision: 1,
            // An Agent without assignments must not advertise a switch capability it cannot honor.
            switch_catalog: TemplateSwitchCatalog::default(),
            external_mcp: SessionExternalMcp::default(),
            device_tools: SessionDeviceTools::default(),
        })
    }

    /// Attaches the External MCP tools this session was admitted with.
    ///
    /// The resolver above is pure and synchronous, so it cannot reach a network or a credential.
    /// Admission adds the snapshot once, right after it resolves, and nothing ever replaces it: a
    /// Template switch changes prompt, language and providers, not the tools a session may call.
    pub fn with_external_mcp(mut self, external_mcp: SessionExternalMcp) -> Self {
        self.external_mcp = external_mcp;
        self
    }

    pub fn external_mcp(&self) -> &SessionExternalMcp {
        &self.external_mcp
    }

    /// Attaches the Device tool review this session was admitted under.
    pub fn with_device_tools(mut self, device_tools: SessionDeviceTools) -> Self {
        self.device_tools = device_tools;
        self
    }

    pub fn device_tools(&self) -> &SessionDeviceTools {
        &self.device_tools
    }

    /// Splits the resolved profile into the snapshot a session installs and the candidate list it
    /// may switch among.  Neither half can reach back into the admission read.
    pub fn into_admitted_profile(self) -> AdmittedSessionProfile {
        AdmittedSessionProfile {
            active: ActiveTemplateProfile {
                source: self.source,
                language: self.language,
                system_prompt: self.system_prompt,
                providers: self.providers,
                revision: self.revision,
            },
            switch_catalog: self.switch_catalog,
            external_mcp: self.external_mcp,
        }
    }
}

/// Everything one admission hands a Voice Session.  It travels as one value so a session can never
/// be installed with a profile and without the candidates and tools it was admitted alongside.
#[derive(Clone, Debug)]
pub struct AdmittedSessionProfile {
    pub active: ActiveTemplateProfile,
    pub switch_catalog: TemplateSwitchCatalog,
    pub external_mcp: SessionExternalMcp,
}

/// Resolves exactly one Effective Session Profile for an admitted Device.
///
/// An Agent with no enabled assignment and no Device override uses server defaults. An enabled
/// assignment graph without a selectable default, or an override that is no longer assigned,
/// remains `Unavailable`.
pub fn resolve_effective_session_profile(
    device_db_id: i64,
    agent_id: i64,
    agent_key: &str,
    assignments: &[AdmittedAssignment],
    config: &AppConfig,
    runtimes: &RuntimeCatalog,
) -> Result<EffectiveSessionProfile, ProfileUnavailable> {
    resolve_effective_session_profile_with_override(
        device_db_id,
        None,
        agent_id,
        agent_key,
        assignments,
        config,
        runtimes,
    )
}

/// Same immutable resolution seam with the admitted Device's optional Template selection.
pub fn resolve_effective_session_profile_with_override(
    device_db_id: i64,
    template_override_id: Option<i64>,
    agent_id: i64,
    agent_key: &str,
    assignments: &[AdmittedAssignment],
    config: &AppConfig,
    runtimes: &RuntimeCatalog,
) -> Result<EffectiveSessionProfile, ProfileUnavailable> {
    if uses_server_defaults(template_override_id, assignments) {
        let mut profile = EffectiveSessionProfile::server_default(config)?;
        profile.device_db_id = device_db_id;
        profile.agent_id = agent_id;
        profile.agent_key = agent_key.to_owned();
        return Ok(profile);
    }

    let default = match template_override_id {
        Some(template_id) => assignments.iter().find(|assignment| {
            assignment.template_id == template_id && assignment.assignment_enabled
        }),
        None => assignments
            .iter()
            .find(|assignment| assignment.is_default && assignment.assignment_enabled),
    };
    let Some(default) = default else {
        // Fail-closed is only safe while it is legible. This branch used to return through a bare
        // `ok_or`, ahead of the `warn!` below, so an Agent holding assignments but no enabled
        // default refused every one of its devices with an unexplained 503 and nothing else.
        warn!(
            agent_key,
            assignment_count = assignments.len(),
            template_override_id = ?template_override_id,
            "no enabled default template is selectable; refusing to fall back to server defaults"
        );
        return Err(ProfileUnavailable);
    };
    let defaults = config.provider_defaults.effective_bindings();
    let active = resolve_template(default, runtimes, &defaults).map_err(|error| {
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
        match resolve_template(assignment, runtimes, &defaults) {
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
        selected_runtimes: None,
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
        external_mcp: SessionExternalMcp::default(),
        device_tools: SessionDeviceTools::default(),
    })
}

/// A soft-unlinked assignment remains in SQLite for audit/history, but it is not an active
/// Template choice. Device overrides deliberately bypass this fallback so a stale override still
/// fails closed instead of silently selecting server defaults.
fn uses_server_defaults(
    template_override_id: Option<i64>,
    assignments: &[AdmittedAssignment],
) -> bool {
    template_override_id.is_none()
        && !assignments
            .iter()
            .any(|assignment| assignment.assignment_enabled)
}

/// A candidate is valid only when its Template is enabled, its prompt is bounded, and every
/// selected provider (explicit binding or deployment default) has a Loaded Runtime.
fn resolve_template(
    assignment: &AdmittedAssignment,
    runtimes: &RuntimeCatalog,
    defaults: &EffectiveProviderBindings,
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
        vad: bound_key(bound.get("vad").copied(), &defaults.vad)?,
        asr: bound_key(bound.get("asr").copied(), &defaults.asr)?,
        llm: bound_key(bound.get("llm").copied(), &defaults.llm)?,
        tts: bound_key(bound.get("tts").copied(), &defaults.tts)?,
        vision: None,
        speaker: bound.get("speaker").map(|key| (*key).to_owned()),
    };
    // One resolution proves all four slots resolve to runtimes this process actually loaded, and
    // hands the session the exact handles a later switch installs.
    let runtimes = runtimes
        .resolve(&providers)
        .map_err(|_| TemplateCandidateError::RuntimeMissing)?;
    Ok(ResolvedTemplateProfile {
        template_id: assignment.template_id,
        template_key: assignment.template_key.clone(),
        template_name: assignment.template_name.clone(),
        language: assignment.language.clone(),
        system_prompt: assignment.prompt.clone(),
        providers,
        runtimes,
        template_revision: assignment.template_revision,
    })
}

/// Only a genuinely absent binding uses the server default. An explicitly bound provider that
/// is disabled or unavailable remains an error.
fn bound_key(key: Option<&str>, default: &str) -> Result<String, TemplateCandidateError> {
    let selected = key.unwrap_or(default);
    if selected.is_empty() {
        return Err(TemplateCandidateError::BindingMissing);
    }
    Ok(selected.to_owned())
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
#[allow(clippy::items_after_test_module)]
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
                    snapshot: None,
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
    fn only_disabled_assignments_use_server_defaults_without_an_override() {
        let server_defaults = resolve_effective_session_profile(
            1,
            2,
            "agent",
            &[assignment(true, false, full_bindings())],
            &config(),
            &RuntimeCatalog::default(),
        )
        .expect("a soft-unlinked agent keeps deployment defaults");
        assert_eq!(server_defaults.source, ProfileSource::ServerDefault);
        assert!(server_defaults.switch_catalog.is_empty());

        let stale_override = resolve_effective_session_profile_with_override(
            1,
            Some(4),
            2,
            "agent",
            &[assignment(true, false, full_bindings())],
            &config(),
            &RuntimeCatalog::default(),
        );
        assert_eq!(stale_override.err(), Some(ProfileUnavailable));
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
        assert_eq!(profile.err(), Some(ProfileUnavailable));
    }

    #[test]
    fn an_oversized_stored_prompt_is_never_accepted() {
        let mut oversized = assignment(true, true, full_bindings());
        oversized.prompt = "p".repeat(MAX_TEMPLATE_PROMPT_BYTES + 1);
        assert_eq!(
            resolve_template(
                &oversized,
                &RuntimeCatalog::default(),
                &config().provider_defaults.effective_bindings(),
            )
            .unwrap_err(),
            TemplateCandidateError::PromptInvalid
        );
    }

    #[test]
    fn a_missing_binding_uses_the_matching_server_default() {
        let partial = assignment(true, true, vec![("asr", "asr"), ("llm", "llm")]);
        let defaults = EffectiveProviderBindings {
            vad: "vad".into(),
            asr: "asr".into(),
            llm: "llm".into(),
            tts: "tts".into(),
            vision: None,
            speaker: None,
        };
        let profile = resolve_template(&partial, &loaded_catalog(), &defaults).unwrap();
        assert_eq!(profile.providers, defaults);
    }

    #[test]
    fn a_bound_speaker_slot_resolves_without_a_deployment_default() {
        let mut with_speaker = assignment(true, true, full_bindings());
        with_speaker.bindings.push(AdmittedProviderBinding {
            provider_type: "speaker".into(),
            provider_key: "speaker".into(),
            provider_enabled: true,
            snapshot: None,
        });
        let profile = resolve_template(
            &with_speaker,
            &loaded_catalog(),
            &config().provider_defaults.effective_bindings(),
        )
        .unwrap();
        assert_eq!(profile.providers.speaker.as_deref(), Some("speaker"));
        assert!(profile.runtimes.speaker.is_some());
    }

    #[test]
    fn an_absent_speaker_slot_has_no_implicit_fallback() {
        let profile = resolve_template(
            &assignment(true, true, full_bindings()),
            &loaded_catalog(),
            &config().provider_defaults.effective_bindings(),
        )
        .unwrap();
        assert_eq!(profile.providers.speaker, None);
        assert!(profile.runtimes.speaker.is_none());
    }

    #[test]
    fn a_bound_speaker_slot_without_a_loaded_runtime_fails_closed() {
        let mut with_speaker = assignment(true, true, full_bindings());
        with_speaker.bindings.push(AdmittedProviderBinding {
            provider_type: "speaker".into(),
            provider_key: "missing".into(),
            provider_enabled: true,
            snapshot: None,
        });
        assert_eq!(
            resolve_template(
                &with_speaker,
                &loaded_catalog(),
                &config().provider_defaults.effective_bindings(),
            )
            .err(),
            Some(TemplateCandidateError::RuntimeMissing)
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
            voice_reserved_capacity: 1,
            command_capacity: 1,
            final_timeout: Duration::from_secs(1),
            cleanup_grace: Duration::from_secs(1),
        };
        RuntimeCatalog {
            speaker: HashMap::from([("speaker".to_owned(), qualification_speaker_runtime())]),
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
            // Two distinct LLM instances so a switch between already-loaded runtimes is provable
            // by runtime identity rather than by binding name alone.
            llm: HashMap::from([
                (
                    "llm".to_owned(),
                    Arc::new(LlmRuntime::new(
                        Arc::new(UnavailableLlm),
                        1,
                        Duration::from_secs(1),
                    )),
                ),
                (
                    "alternate".to_owned(),
                    Arc::new(LlmRuntime::new(
                        Arc::new(UnavailableLlm),
                        1,
                        Duration::from_secs(1),
                    )),
                ),
            ]),
            tts: HashMap::from([(
                "tts".to_owned(),
                Arc::new(TtsWorkerRuntime::new(Arc::new(UnavailableTts), worker)),
            )]),
            vision: HashMap::new(),
        }
    }
    /// Minimal extraction-only speaker instance so a bound Template Speaker slot can resolve in a
    /// unit test without the native CAM++ model.
    fn qualification_speaker_runtime() -> Arc<crate::providers::speaker::SpeakerRuntime> {
        struct Qualification;
        impl crate::providers::speaker::SpeakerProvider for Qualification {
            fn dimension(&self) -> usize {
                4
            }
            fn extract(
                &mut self,
                _: &crate::audio::PcmF32Mono,
            ) -> Result<Vec<f32>, crate::providers::speaker::SpeakerError> {
                Ok(vec![1.0, 0.0, 0.0, 0.0])
            }
        }
        Arc::new(
            crate::providers::speaker::SpeakerRuntime::new(
                Box::new(Qualification),
                crate::workers::ProviderRuntimeAdmission::new(1, 1),
            )
            .expect("qualification speaker runtime"),
        )
    }

    fn named(id: i64, key: &str, default: bool, enabled: bool) -> AdmittedAssignment {
        AdmittedAssignment {
            template_id: id,
            template_key: key.into(),
            ..assignment(default, enabled, full_bindings())
        }
    }

    /// Binds the LLM slot to a different already-loaded instance so a switch is observable as a
    /// runtime change, not only as a prompt change.
    fn alternate_llm(mut candidate: AdmittedAssignment) -> AdmittedAssignment {
        candidate
            .bindings
            .iter_mut()
            .find(|binding| binding.provider_type == "llm")
            .expect("the full binding set always carries an LLM slot")
            .provider_key = "alternate".into();
        candidate
    }

    #[test]
    fn the_switch_catalog_keeps_candidates_with_missing_bindings_on_server_defaults() {
        let mut unloaded = named(2, "unloaded", false, true);
        unloaded
            .bindings
            .retain(|binding| binding.provider_type != "llm");
        unloaded.bindings.push(AdmittedProviderBinding {
            provider_type: "llm".into(),
            provider_key: "never-loaded".into(),
            provider_enabled: true,
            snapshot: None,
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
        assert_eq!(
            keys,
            vec![
                "primary".to_owned(),
                "usable".to_owned(),
                "incomplete".to_owned(),
            ]
        );
    }

    #[test]
    fn a_switch_targets_only_a_candidate_this_session_admitted() {
        let profile = resolve_effective_session_profile(
            1,
            2,
            "agent",
            &[
                named(1, "primary", true, true),
                named(7, "usable", false, true),
            ],
            &config(),
            &loaded_catalog(),
        )
        .expect("a valid default template still admits the session");
        assert!(
            profile.switch_catalog.find("usable").is_some(),
            "an admitted candidate must be selectable"
        );
        assert!(
            profile.switch_catalog.find("unloaded").is_none(),
            "a candidate that never entered the catalog is not selectable"
        );
    }

    #[test]
    fn a_successful_switch_replaces_the_active_snapshot_and_advances_the_revision() {
        let mut usable = alternate_llm(named(7, "usable", false, true));
        usable.prompt = "secondary system prompt".into();
        let profile = resolve_effective_session_profile(
            1,
            2,
            "agent",
            &[named(1, "primary", true, true), usable],
            &config(),
            &loaded_catalog(),
        )
        .expect("a valid default template still admits the session");
        let admitted = profile.into_admitted_profile();
        let (mut active, catalog) = (admitted.active, admitted.switch_catalog);
        assert_eq!(active.revision, 1);
        assert_eq!(active.system_prompt, "stored system prompt");

        let Some(candidate) = catalog.find("usable").cloned() else {
            panic!("the catalog holds the candidate");
        };
        active.switched_to(&candidate);

        assert_eq!(active.system_prompt, "secondary system prompt");
        assert_eq!(active.language, "vi-VN");
        assert_eq!(
            active.source,
            ProfileSource::Template {
                template_id: 7,
                template_key: "usable".into(),
                template_name: "Support".into(),
                template_revision: 2,
            }
        );
        assert_eq!(active.providers.llm, "alternate");
        assert_eq!(active.revision, 2);
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

impl std::fmt::Debug for TemplateSwitchCatalog {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TemplateSwitchCatalog")
            .field("ready_candidates", &self.candidates.len())
            .field("cold_candidates", &self.cold.len())
            .finish_non_exhaustive()
    }
}

/// Immutable input collected during Device admission for managed runtime resolution.
///
/// Grouping it keeps the public resolution seam aligned with the atomic admission snapshot: a
/// caller cannot pair an Agent's assignments with another Device or deployment configuration.
pub struct ManagedSessionProfileInput<'a> {
    pub device_db_id: i64,
    pub template_override_id: Option<i64>,
    pub agent_id: i64,
    pub agent_key: &'a str,
    pub assignments: &'a [AdmittedAssignment],
    pub config: &'a AppConfig,
    pub manager: &'a Arc<ProviderRuntimeManager>,
    pub deployment_snapshots: &'a [crate::database::DesiredProvider],
}

/// Acquires only the selected template; all switch candidates retain immutable configuration.
pub async fn resolve_managed_session_profile(
    input: ManagedSessionProfileInput<'_>,
) -> Result<EffectiveSessionProfile, RuntimeError> {
    let ManagedSessionProfileInput {
        device_db_id,
        template_override_id,
        agent_id,
        agent_key,
        assignments,
        config,
        manager,
        deployment_snapshots,
    } = input;
    if uses_server_defaults(template_override_id, assignments) {
        let mut profile = EffectiveSessionProfile::server_default(config)
            .map_err(|_| RuntimeError::Configuration)?;
        profile.device_db_id = device_db_id;
        profile.agent_id = agent_id;
        profile.agent_key = agent_key.to_owned();
        return Ok(profile);
    }
    let selected = assignments.iter().find(|assignment| {
        assignment.assignment_enabled
            && template_override_id.map_or(assignment.is_default, |id| assignment.template_id == id)
    });
    let Some(selected) = selected else {
        // Same legibility requirement as the unmanaged seam: this refusal reaches the client as a
        // 503 whose body names a configuration error, so the reason has to be logged here or the
        // operator sees only the status.
        warn!(
            agent_key,
            assignment_count = assignments.len(),
            template_override_id = ?template_override_id,
            "no enabled default template is selectable; refusing to fall back to server defaults"
        );
        return Err(RuntimeError::Configuration);
    };
    let defaults = config.provider_defaults.effective_bindings();
    let configuration = ConfiguredTemplateProfile::from_assignment_with_defaults(
        selected,
        &defaults,
        deployment_snapshots,
    )
    .map_err(|_| RuntimeError::Configuration)?;
    let prepared = configuration
        .prepare(manager, manager.admission_deadline())
        .await?;
    let cold = assignments
        .iter()
        .filter_map(|assignment| {
            ConfiguredTemplateProfile::from_assignment_with_defaults(
                assignment,
                &defaults,
                deployment_snapshots,
            )
            .ok()
        })
        .collect();
    Ok(EffectiveSessionProfile {
        selected_runtimes: Some(prepared.runtimes),
        device_db_id,
        agent_id,
        agent_key: agent_key.to_owned(),
        source: configuration.source,
        language: configuration.language,
        system_prompt: configuration.system_prompt,
        providers: configuration.providers,
        revision: 1,
        switch_catalog: TemplateSwitchCatalog {
            candidates: vec![],
            cold,
            manager: Some(Arc::clone(manager)),
            active_leases: prepared.leases,
        },
        external_mcp: SessionExternalMcp::default(),
        device_tools: SessionDeviceTools::default(),
    })
}
