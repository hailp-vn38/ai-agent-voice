//! Persistent & live configuration for Flow 1 (`daily_root`) & Flow 2 (`wiki_root`) Root Orchestrators,
//! Built-in Sub-Agents, and Custom Sub-Agents, integrated with the Server LLM/TTS Provider Catalog.

mod media_backend;
mod persistence;
mod provider_catalog;
mod resolution;
mod specs;
mod validation;

pub(crate) use media_backend::install_server_tts_runtime;
pub use media_backend::{
    CatalogImageGeneratorBackend, CatalogMediaBackend, CatalogTtsSynthesizerBackend,
    CatalogVideoGeneratorBackend,
};
pub use persistence::{
    AgentConfigOverride, PersistedBackgroundSettings, ValidCustomSubagentRef,
    load_persisted_settings, save_persisted_settings, write_agent_prompts,
};
pub use provider_catalog::{
    MuseCatalogOptions, ResolvedLlmProvider, default_orchestrator_provider, find_provider_by_tag,
    resolve_llm_provider_catalog,
};
pub use resolution::{build_agent_definitions, project_agents_json};
pub use specs::{
    AGENT_SPECS, AgentSpec, CUSTOM_DAILY_SUBAGENT_TOOLS, CUSTOM_WIKI_SUBAGENT_TOOLS,
    DEFAULT_USER_ROOT_AGENT_ID, WIKI_AUTOFLOW_ROOT_AGENT_ID, custom_subagent_flow_policy,
    find_agent_spec, is_user_routable_root_agent, normalize_user_target_agent,
    valid_custom_subagent_id,
};
pub use validation::{
    CreateCustomSubagentInput, SubagentMutationError, create_custom_subagent,
    delete_custom_subagent, sync_orchestrator_from_state, validate_agent_overrides,
    validate_agent_overrides_with_skills,
};

pub const DEFAULT_ORCHESTRATOR_PROVIDER_REF: &str = "orchestrator:default";
