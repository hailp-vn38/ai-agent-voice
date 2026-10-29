//! LLM and TTS provider catalog discovery from `[orchestrator]`, `[providers.llm.instances]`,
//! `[providers.tts.instances]`, resident TTS runtimes, and the SQLite `providers` table.

mod discovery;
mod resolved;
mod speech;

use crate::config::{OrchestratorSectionConfig, SecretString};

use super::DEFAULT_ORCHESTRATOR_PROVIDER_REF;
use super::specs::{AgentSpec, find_agent_spec};

pub use discovery::resolve_llm_provider_catalog;
pub use resolved::ResolvedLlmProvider;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MuseCatalogOptions {
    pub account_concurrency: usize,
    pub cdp_port: u16,
    pub headless: bool,
}

impl Default for MuseCatalogOptions {
    fn default() -> Self {
        Self {
            account_concurrency: 2,
            cdp_port: 19222,
            headless: true,
        }
    }
}

pub fn find_provider_by_tag<'a>(
    catalog: &'a [ResolvedLlmProvider],
    tag: &str,
) -> Option<&'a ResolvedLlmProvider> {
    catalog.iter().find(|p| p.supports_tag(tag))
}

pub(crate) fn agent_supports_provider(agent_id: &str, provider: &ResolvedLlmProvider) -> bool {
    let allowed = find_agent_spec(agent_id).map_or(
        &["chat", "reasoning"][..],
        AgentSpec::allowed_capability_tags,
    );
    allowed.iter().any(|tag| provider.supports_tag(tag))
}

pub fn default_orchestrator_provider(cfg: &OrchestratorSectionConfig) -> ResolvedLlmProvider {
    ResolvedLlmProvider {
        provider_ref: DEFAULT_ORCHESTRATOR_PROVIDER_REF.to_owned(),
        name: cfg.model.clone(),
        source: "config",
        adapter: "openai".to_owned(),
        model: cfg.model.clone(),
        base_url: cfg.endpoint.clone(),
        api_key: (!cfg.api_key.trim().is_empty()).then(|| SecretString::new(cfg.api_key.clone())),
        gateway_binding: None,
        timeout_ms: cfg.request_timeout_ms,
        max_tokens: cfg.max_output_tokens,
        context_limit_tokens: cfg.context_limit_tokens,
        reasoning_effort: None,
        temperature_percent: None,
        tags: vec!["chat".to_owned()],
        muse_options: None,
    }
}
