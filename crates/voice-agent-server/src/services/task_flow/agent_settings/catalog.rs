//! LLM Provider catalog resolution from `[orchestrator]`, `[providers.llm.instances]`,
//! and the SQLite `providers` table (`type = 'llm' AND enabled = 1`), plus mapping into
//! `Orchestrator` `AgentDefinition`s and Admin JSON projections.

use std::time::Duration;

use orchestrator_agent_rs::{
    AgentDefinition, AgentMode, LlmBackend, LlmProviderConfig, ProviderAuth, ReasoningEffort,
    SecretString,
};
use serde_json::{Value, json};

use crate::app::AppState;
use crate::config::{LlmInstanceConfig, OrchestratorSectionConfig};

use super::specs::{AGENT_SPECS, AgentSpec, find_agent_spec};
use super::{DEFAULT_ORCHESTRATOR_PROVIDER_REF, PersistedBackgroundSettings};

#[derive(Clone, Debug)]
pub struct ResolvedLlmProvider {
    pub provider_ref: String,
    pub name: String,
    pub source: &'static str,
    pub adapter: String,
    pub model: String,
    pub base_url: String,
    /// Wrapped in [`SecretString`] so `Debug` output is always redacted (`[REDACTED]`).
    pub api_key: Option<SecretString>,
    pub timeout_ms: u64,
    pub max_tokens: u32,
    pub context_limit_tokens: usize,
    pub reasoning_effort: Option<String>,
    pub temperature_percent: Option<u16>,
    pub tags: Vec<String>,
}

impl ResolvedLlmProvider {
    pub fn supports_tag(&self, tag: &str) -> bool {
        self.tags.iter().any(|t| t == tag)
    }

    pub fn to_json_summary(&self) -> Value {
        json!({
            "ref": self.provider_ref,
            "name": self.name,
            "source": self.source,
            "adapter": self.adapter,
            "model": self.model,
            "base_url": self.base_url,
            "timeout_ms": self.timeout_ms,
            "max_tokens": self.max_tokens,
            "context_limit_tokens": self.context_limit_tokens,
            "reasoning_effort": self.reasoning_effort,
            "temperature_percent": self.temperature_percent,
            "tags": self.tags,
        })
    }

    pub fn to_orchestrator_provider(&self) -> LlmProviderConfig {
        let auth = match &self.api_key {
            Some(key) => ProviderAuth::ApiKey(key.clone()),
            None => ProviderAuth::None,
        };
        LlmProviderConfig {
            id: self.provider_ref.clone(),
            backend: LlmBackend::OpenAiCompatible,
            endpoint: Some(self.base_url.clone()),
            auth,
            request_timeout: Duration::from_millis(self.timeout_ms.max(1_000)),
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
    let allowed = find_agent_spec(agent_id)
        .map_or(&["chat"][..], AgentSpec::allowed_capability_tags);
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
        timeout_ms: cfg.request_timeout_ms,
        max_tokens: cfg.max_output_tokens,
        context_limit_tokens: cfg.context_limit_tokens,
        reasoning_effort: None,
        temperature_percent: None,
        tags: vec!["chat".to_owned()],
    }
}

pub async fn resolve_llm_provider_catalog(state: &AppState) -> Vec<ResolvedLlmProvider> {
    let orch_cfg = &state.config.orchestrator;
    let mut out = vec![default_orchestrator_provider(orch_cfg)];

    // 1. Deployment config LLM instances (`config:llm:<key>`)
    let mut config_keys: Vec<&String> = state.config.providers.llm.instances.keys().collect();
    config_keys.sort();
    for key in config_keys {
        let Some(instance) = state.config.providers.llm.instances.get(key) else {
            continue;
        };
        let provider_ref = format!("config:llm:{key}");
        match instance {
            LlmInstanceConfig::Openai(openai) => {
                out.push(ResolvedLlmProvider {
                    provider_ref,
                    name: format!("{key} ({})", openai.model),
                    source: "config",
                    adapter: "openai".to_owned(),
                    model: openai.model.clone(),
                    base_url: openai.base_url.to_string(),
                    api_key: (!openai.api_key.expose().trim().is_empty())
                        .then(|| SecretString::new(openai.api_key.expose().to_owned())),
                    timeout_ms: openai.timeout_ms,
                    max_tokens: openai.max_tokens.unwrap_or(orch_cfg.max_output_tokens),
                    context_limit_tokens: openai
                        .context_limit_tokens
                        .map(|v| v as usize)
                        .unwrap_or(orch_cfg.context_limit_tokens),
                    reasoning_effort: openai.reasoning_effort.clone(),
                    temperature_percent: openai.temperature_percent,
                    tags: crate::providers::llm::effective_llm_tags(openai.tags.as_deref()),
                });
            }
            LlmInstanceConfig::OpenCodeFree(free) => {
                out.push(ResolvedLlmProvider {
                    provider_ref,
                    name: format!("{key} ({})", free.model),
                    source: "config",
                    adapter: "opencode_free".to_owned(),
                    model: free.model.clone(),
                    base_url: crate::providers::llm::opencode_free::BASE_URL.to_owned(),
                    api_key: Some(SecretString::new(
                        crate::providers::llm::opencode_free::PUBLIC_KEY,
                    )),
                    timeout_ms: free.timeout_ms,
                    max_tokens: free.max_tokens.unwrap_or(orch_cfg.max_output_tokens),
                    context_limit_tokens: free
                        .context_limit_tokens
                        .map(|v| v as usize)
                        .unwrap_or(orch_cfg.context_limit_tokens),
                    reasoning_effort: free.reasoning_effort.clone(),
                    temperature_percent: free.temperature_percent,
                    tags: crate::providers::llm::effective_llm_tags(free.tags.as_deref()),
                });
            }
            #[cfg(feature = "qualification-providers")]
            LlmInstanceConfig::QualificationLlm(_) => {}
        }
    }

    // 2. Database LLM providers (`providers` table where `type='llm' AND enabled=1`)
    if let Some(database) = &state.database
        && let Ok(rows) = sqlx::query_as::<_, (String, String, String, String, Option<String>)>(
            "SELECT key, name, adapter, config_json, credential_json FROM providers WHERE type='llm' AND enabled=1 ORDER BY id",
        )
        .fetch_all(database.pool())
        .await
    {
        for (key, name, adapter, config_json, credential_json) in rows {
            let Ok(parsed) = serde_json::from_str::<Value>(&config_json) else {
                continue;
            };
            let Some(model) = parsed
                .get("model")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_owned)
            else {
                continue;
            };
            let timeout_ms = parsed
                .get("timeout_ms")
                .and_then(Value::as_u64)
                .unwrap_or(orch_cfg.request_timeout_ms);
            let max_tokens = parsed
                .get("max_tokens")
                .and_then(Value::as_u64)
                .and_then(|v| u32::try_from(v).ok())
                .unwrap_or(orch_cfg.max_output_tokens);
            let context_limit_tokens = parsed
                .get("context_limit_tokens")
                .and_then(Value::as_u64)
                .and_then(|v| usize::try_from(v).ok())
                .unwrap_or(orch_cfg.context_limit_tokens);
            let reasoning_effort = parsed
                .get("reasoning_effort")
                .and_then(Value::as_str)
                .map(str::to_owned);
            let temperature_percent = parsed
                .get("temperature_percent")
                .and_then(Value::as_u64)
                .and_then(|v| u16::try_from(v).ok());

            let (base_url, api_key) = match adapter.as_str() {
                "openai" => {
                    let Some(url) = parsed
                        .get("base_url")
                        .and_then(Value::as_str)
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                        .map(str::to_owned)
                    else {
                        continue;
                    };
                    let secret_ref = crate::database::credentials::reference(
                        &format!("provider:{key}"),
                        credential_json.as_deref(),
                        crate::database::secrets::provider_secret_env(&key, &adapter),
                    );
                    let resolved_key = secret_ref
                        .and_then(|r| crate::database::secrets::SecretRef::parse(r).ok())
                        .and_then(|s| state.secret_resolver.resolve(&s).ok())
                        .map(|v| v.expose().trim().to_owned())
                        .filter(|s| !s.is_empty())
                        .map(SecretString::new);
                    (url, resolved_key)
                }
                "opencode_free" => (
                    crate::providers::llm::opencode_free::BASE_URL.to_owned(),
                    Some(SecretString::new(
                        crate::providers::llm::opencode_free::PUBLIC_KEY,
                    )),
                ),
                _ => continue,
            };

            let tags = crate::providers::llm::llm_config_tags(&config_json);

            out.push(ResolvedLlmProvider {
                provider_ref: key,
                name,
                source: "database",
                adapter,
                model,
                base_url,
                api_key,
                timeout_ms,
                max_tokens,
                context_limit_tokens,
                reasoning_effort,
                temperature_percent,
                tags,
            });
        }
    }

    out
}

struct ResolvedAgentSettings<'a> {
    spec: &'static AgentSpec,
    enabled: bool,
    explicit_ref: Option<String>,
    resolved_provider: ResolvedLlmProvider,
    steps: u32,
    allowed_tools: Vec<String>,
    allowed_skills: Vec<String>,
    allowed_subagents: Vec<String>,
    description: &'a str,
    system_prompt: &'a str,
}

fn resolve_effective_agents<'a>(
    cfg: &OrchestratorSectionConfig,
    settings: &'a PersistedBackgroundSettings,
    catalog: &[ResolvedLlmProvider],
) -> Vec<ResolvedAgentSettings<'a>> {
    let fallback = find_provider_by_tag(catalog, "chat")
        .cloned()
        .unwrap_or_else(|| default_orchestrator_provider(cfg));
    let find_provider = |agent_id: &str, pref: &str| -> ResolvedLlmProvider {
        catalog
            .iter()
            .find(|p| p.provider_ref == pref && agent_supports_provider(agent_id, p))
            .cloned()
            .unwrap_or_else(|| fallback.clone())
    };

    AGENT_SPECS
        .iter()
        .map(|spec| {
            let ovr = settings.agents.get(spec.id);
            let enabled = if spec.mode == AgentMode::Primary {
                true
            } else {
                ovr.is_none_or(|o| o.enabled)
            };
            let explicit_ref = ovr
                .and_then(|o| o.provider_ref.as_deref())
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_owned);
            let effective_ref = explicit_ref.as_deref().unwrap_or_else(|| match spec.mode {
                AgentMode::Primary => settings.orchestrator_llm_provider.as_str(),
                _ => settings.worker_llm_provider.as_str(),
            });
            let resolved_provider = find_provider(spec.id, effective_ref);
            let steps = ovr
                .and_then(|o| o.steps)
                .unwrap_or(spec.default_steps)
                .clamp(1, 100);
            let allowed_skills: Vec<String> = match ovr.and_then(|o| o.allowed_skills.as_ref()) {
                Some(list) => list
                    .iter()
                    .filter(|s| spec.available_skills.contains(&s.as_str()))
                    .cloned()
                    .collect(),
                None => spec
                    .default_allowed_skills
                    .iter()
                    .map(|s| (*s).to_owned())
                    .collect(),
            };
            let allowed_tools: Vec<String> = if spec.mode == AgentMode::Primary {
                vec!["subagent".to_owned()]
            } else {
                let mut tools: Vec<String> = match ovr.and_then(|o| o.allowed_tools.as_ref()) {
                    Some(list) => list
                        .iter()
                        .filter(|t| spec.available_tools.contains(&t.as_str()))
                        .cloned()
                        .collect(),
                    None => spec
                        .default_allowed_tools
                        .iter()
                        .map(|t| (*t).to_owned())
                        .collect(),
                };
                if !allowed_skills.is_empty() && !tools.iter().any(|t| t == "skill") {
                    tools.insert(0, "skill".to_owned());
                }
                tools
            };
            let allowed_subagents: Vec<String> = if spec.mode == AgentMode::Primary {
                spec.default_subagents
                    .iter()
                    .filter(|sub_id| {
                        settings
                            .agents
                            .get(**sub_id)
                            .is_none_or(|sub_ovr| sub_ovr.enabled)
                    })
                    .map(|s| (*s).to_owned())
                    .collect()
            } else {
                Vec::new()
            };
            let description = ovr
                .and_then(|o| o.description.as_deref())
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .unwrap_or(spec.default_description);
            let system_prompt = ovr
                .and_then(|o| o.system_prompt.as_deref())
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .unwrap_or(spec.default_prompt);

            ResolvedAgentSettings {
                spec,
                enabled,
                explicit_ref,
                resolved_provider,
                steps,
                allowed_tools,
                allowed_skills,
                allowed_subagents,
                description,
                system_prompt,
            }
        })
        .collect()
}

pub fn build_agent_definitions(
    cfg: &OrchestratorSectionConfig,
    settings: &PersistedBackgroundSettings,
    catalog: &[ResolvedLlmProvider],
) -> (Vec<LlmProviderConfig>, Vec<AgentDefinition>) {
    let providers: Vec<LlmProviderConfig> = catalog
        .iter()
        .map(ResolvedLlmProvider::to_orchestrator_provider)
        .collect();

    let defs = resolve_effective_agents(cfg, settings, catalog)
        .into_iter()
        .map(|eff| {
            let resolved = &eff.resolved_provider;
            let mut def = AgentDefinition::new(eff.spec.id, &resolved.provider_ref, &resolved.model)
                .with_mode(eff.spec.mode)
                .with_description(eff.description)
                .with_prompt_path(cfg.prompts_dir.join(format!("{}.md", eff.spec.id)))
                .with_context_limit_tokens(resolved.context_limit_tokens)
                .with_max_output_tokens(resolved.max_tokens)
                .with_allowed_tools(eff.allowed_tools)
                .with_allowed_skills(eff.allowed_skills)
                .with_steps(eff.steps);

            if let Some(pct) = resolved.temperature_percent {
                def = def.with_temperature((f32::from(pct) / 100.0).clamp(0.0, 2.0));
            }
            if let Some(ref effort) = resolved.reasoning_effort {
                match effort.as_str() {
                    "low" => def = def.with_reasoning_effort(ReasoningEffort::Low),
                    "medium" => def = def.with_reasoning_effort(ReasoningEffort::Medium),
                    "high" => def = def.with_reasoning_effort(ReasoningEffort::High),
                    _ => {}
                }
            }
            if eff.spec.mode == AgentMode::Primary {
                def = def.with_allowed_subagents(eff.allowed_subagents);
            }
            def
        })
        .collect();

    (providers, defs)
}

pub fn project_agents_json(
    cfg: &OrchestratorSectionConfig,
    settings: &PersistedBackgroundSettings,
    catalog: &[ResolvedLlmProvider],
) -> Vec<Value> {
    resolve_effective_agents(cfg, settings, catalog)
        .into_iter()
        .map(|eff| {
            json!({
                "id": eff.spec.id,
                "flow_id": eff.spec.flow_id,
                "parent_id": eff.spec.parent_id,
                "mode": match eff.spec.mode {
                    AgentMode::Primary => "primary",
                    AgentMode::Subagent => "subagent",
                    AgentMode::All => "all",
                },
                "enabled": eff.enabled,
                "provider_ref": eff.explicit_ref,
                "effective_provider_ref": eff.resolved_provider.provider_ref,
                "effective_model": eff.resolved_provider.model,
                "steps": eff.steps,
                "default_steps": eff.spec.default_steps,
                "allowed_tools": eff.allowed_tools,
                "default_allowed_tools": eff.spec.default_allowed_tools,
                "available_tools": eff.spec.available_tools,
                "allowed_skills": eff.allowed_skills,
                "default_allowed_skills": eff.spec.default_allowed_skills,
                "available_skills": eff.spec.available_skills,
                "allowed_subagents": eff.allowed_subagents,
                "allowed_capability_tags": eff.spec.allowed_capability_tags(),
                "description": eff.description,
                "default_description": eff.spec.default_description,
                "system_prompt": eff.system_prompt,
                "default_system_prompt": eff.spec.default_prompt,
            })
        })
        .collect()
}
