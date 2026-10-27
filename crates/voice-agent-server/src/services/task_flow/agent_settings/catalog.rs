//! LLM Provider catalog resolution from `[orchestrator]`, `[providers.llm.instances]`,
//! and the SQLite `providers` table (`type = 'llm' AND enabled = 1`), plus mapping into
//! `Orchestrator` `AgentDefinition`s and Admin JSON projections.

use std::time::Duration;

use orchestrator_agent_rs::{
    AgentDefinition, AgentMode, LlmBackend, LlmProviderConfig, ProviderAuth, ReasoningEffort,
};
use serde_json::{Value, json};

use crate::app::AppState;
use crate::config::{LlmInstanceConfig, OrchestratorSectionConfig, SecretString};

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
            || (tag == "chat" && self.tags.iter().any(|t| t == "reasoning"))
    }

    fn clamped_max_tokens(&self) -> u32 {
        self.max_tokens.clamp(1, 65_536)
    }

    fn clamped_context_limit_tokens(&self) -> u32 {
        u32::try_from(self.context_limit_tokens)
            .unwrap_or(128_000)
            .clamp(4_096, 2_000_000)
    }

    pub(crate) fn to_llm_provider(
        &self,
    ) -> Result<Box<dyn crate::providers::LlmProvider>, crate::providers::LlmError> {
        let max_tokens = Some(self.clamped_max_tokens());
        let context_limit_tokens = Some(self.clamped_context_limit_tokens());
        match self.adapter.as_str() {
            "openai" => {
                let base_url = url::Url::parse(&self.base_url)
                    .map_err(|_| crate::providers::LlmError::Failed)?;
                let api_key = self
                    .api_key
                    .as_ref()
                    .map(|k| crate::config::SecretString::new(k.expose()))
                    .unwrap_or_default();
                let cfg = crate::config::OpenAiConfig {
                    api_key,
                    base_url,
                    model: self.model.clone(),
                    timeout_ms: self.timeout_ms.max(1_000),
                    max_tokens,
                    context_limit_tokens,
                    reasoning_effort: self.reasoning_effort.clone(),
                    temperature_percent: self.temperature_percent,
                    tags: Some(self.tags.clone()),
                };
                Ok(Box::new(
                    crate::providers::llm::ConfiguredOpenAiLlm::from_config(&cfg)?,
                ))
            }
            "opencode_free" => {
                let cfg = crate::config::OpenCodeFreeConfig {
                    model: self.model.clone(),
                    timeout_ms: self.timeout_ms.clamp(1_000, 600_000),
                    max_tokens,
                    context_limit_tokens,
                    reasoning_effort: self.reasoning_effort.clone(),
                    temperature_percent: self.temperature_percent,
                    tags: Some(self.tags.clone()),
                };
                Ok(Box::new(
                    crate::providers::llm::opencode_free::OpenCodeFreeProvider::build(&cfg)?,
                ))
            }
            _ => Err(crate::providers::LlmError::Failed),
        }
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
            Some(key) => ProviderAuth::ApiKey(orchestrator_agent_rs::SecretString::new(
                key.expose().to_owned(),
            )),
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
    execution_provider: ResolvedLlmProvider,
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
    let find_chat_provider = |pref: &str| -> ResolvedLlmProvider {
        catalog
            .iter()
            .find(|p| p.provider_ref == pref && p.supports_tag("chat"))
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
            // Subagents in `Orchestrator` always run a chat/tool-calling loop over `/v1/chat/completions`.
            // When `media_creator` is configured with a dedicated `image` or `video` provider (without `chat`),
            // keep `resolved_provider` for UI projection & media backend selection, while using the worker
            // `chat` provider for the Orchestrator's tool-calling session.
            let execution_provider = if resolved_provider.supports_tag("chat") {
                resolved_provider.clone()
            } else {
                find_chat_provider(&settings.worker_llm_provider)
            };
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
                execution_provider,
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
            let resolved = &eff.execution_provider;
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
            } else if resolved.supports_tag("reasoning") {
                def = def.with_reasoning_effort(ReasoningEffort::Medium);
            }
            if eff.spec.mode == AgentMode::Primary {
                def = def.with_allowed_subagents(eff.allowed_subagents);
            }
            def
        })
        .collect();

    (providers, defs)
}

static LIVE_MEDIA_CATALOG: std::sync::OnceLock<
    std::sync::Arc<std::sync::RwLock<Vec<ResolvedLlmProvider>>>,
> = std::sync::OnceLock::new();

static LIVE_MEDIA_SETTINGS: std::sync::OnceLock<
    std::sync::Arc<std::sync::RwLock<Option<PersistedBackgroundSettings>>>,
> = std::sync::OnceLock::new();

pub(crate) fn update_live_media_catalog(
    catalog: &[ResolvedLlmProvider],
    settings: Option<&PersistedBackgroundSettings>,
) {
    if let Some(shared) = LIVE_MEDIA_CATALOG.get() {
        *shared.write().unwrap_or_else(|e| e.into_inner()) = catalog.to_vec();
    }
    if let Some(settings) = settings
        && let Some(shared_settings) = LIVE_MEDIA_SETTINGS.get()
    {
        *shared_settings.write().unwrap_or_else(|e| e.into_inner()) = Some(settings.clone());
    }
}

pub type CatalogImageGeneratorBackend = CatalogMediaBackend;
pub type CatalogVideoGeneratorBackend = CatalogMediaBackend;
pub type CatalogTtsSynthesizerBackend = CatalogMediaBackend;

#[derive(Clone)]
pub struct CatalogMediaBackend {
    store_dir: Option<std::path::PathBuf>,
    catalog: std::sync::Arc<std::sync::RwLock<Vec<ResolvedLlmProvider>>>,
    settings: std::sync::Arc<std::sync::RwLock<Option<PersistedBackgroundSettings>>>,
    default_image: orchestrator_flow_rs::DefaultImageGeneratorBackend,
    default_video: orchestrator_flow_rs::DefaultVideoGeneratorBackend,
    default_tts: orchestrator_flow_rs::DefaultTtsSynthesizerBackend,
}

impl CatalogMediaBackend {
    pub fn new(
        store_dir: impl Into<std::path::PathBuf>,
        catalog: Vec<ResolvedLlmProvider>,
    ) -> Self {
        let dir = store_dir.into();
        let initial_settings = super::load_persisted_settings(&dir);
        let shared = LIVE_MEDIA_CATALOG
            .get_or_init(|| std::sync::Arc::new(std::sync::RwLock::new(catalog.clone())))
            .clone();
        *shared.write().unwrap_or_else(|e| e.into_inner()) = catalog;
        let shared_settings = LIVE_MEDIA_SETTINGS
            .get_or_init(|| std::sync::Arc::new(std::sync::RwLock::new(initial_settings.clone())))
            .clone();
        *shared_settings.write().unwrap_or_else(|e| e.into_inner()) = initial_settings;
        Self {
            store_dir: Some(dir),
            catalog: shared,
            settings: shared_settings,
            default_image: orchestrator_flow_rs::DefaultImageGeneratorBackend,
            default_video: orchestrator_flow_rs::DefaultVideoGeneratorBackend,
            default_tts: orchestrator_flow_rs::DefaultTtsSynthesizerBackend,
        }
    }

    pub fn from_catalog(catalog: Vec<ResolvedLlmProvider>) -> Self {
        Self {
            store_dir: None,
            catalog: std::sync::Arc::new(std::sync::RwLock::new(catalog)),
            settings: std::sync::Arc::new(std::sync::RwLock::new(None)),
            default_image: orchestrator_flow_rs::DefaultImageGeneratorBackend,
            default_video: orchestrator_flow_rs::DefaultVideoGeneratorBackend,
            default_tts: orchestrator_flow_rs::DefaultTtsSynthesizerBackend,
        }
    }

    pub fn with_settings(self, settings: PersistedBackgroundSettings) -> Self {
        *self.settings.write().unwrap_or_else(|e| e.into_inner()) = Some(settings);
        self
    }

    fn effective_settings(&self) -> Option<PersistedBackgroundSettings> {
        if let Some(ref dir) = self.store_dir
            && let Some(loaded) = super::load_persisted_settings(dir)
        {
            return Some(loaded);
        }
        self.settings
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    fn select_provider(&self, tool_name: &str, required_tag: &str) -> Option<ResolvedLlmProvider> {
        let guard = self.catalog.read().unwrap_or_else(|e| e.into_inner());
        let settings = self.effective_settings();
        resolve_media_tool_provider(&guard, settings.as_ref(), tool_name, required_tag, true)
    }
}

fn resolve_media_tool_provider(
    catalog: &[ResolvedLlmProvider],
    settings: Option<&PersistedBackgroundSettings>,
    tool_name: &str,
    required_tag: &str,
    require_api_key: bool,
) -> Option<ResolvedLlmProvider> {
    let matches_prov = |p: &ResolvedLlmProvider, pref: &str| -> bool {
        p.provider_ref == pref
            && p.supports_tag(required_tag)
            && (!require_api_key || p.api_key.is_some())
    };
    if let Some(settings) = settings {
        if let Some(tool_ref) = settings
            .agents
            .get("media_creator")
            .and_then(|o| o.tool_providers.as_ref())
            .and_then(|m| m.get(tool_name))
            .map(String::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            && let Some(found) = catalog.iter().find(|p| matches_prov(p, tool_ref))
        {
            return Some(found.clone());
        }
        if matches!(required_tag, "image" | "video")
            && let Some(explicit_ref) = settings
                .agents
                .get("media_creator")
                .and_then(|o| o.provider_ref.as_deref())
                .map(str::trim)
                .filter(|s| !s.is_empty())
            && let Some(found) = catalog.iter().find(|p| matches_prov(p, explicit_ref))
        {
            return Some(found.clone());
        }
        let worker_ref = settings.worker_llm_provider.trim();
        if !worker_ref.is_empty()
            && let Some(found) = catalog.iter().find(|p| matches_prov(p, worker_ref))
        {
            return Some(found.clone());
        }
    }
    catalog
        .iter()
        .find(|p| p.supports_tag(required_tag) && (!require_api_key || p.api_key.is_some()))
        .cloned()
}

#[async_trait::async_trait]
impl orchestrator_flow_rs::ImageGeneratorBackend for CatalogMediaBackend {
    async fn generate_image(
        &self,
        req: &orchestrator_flow_rs::ImageGenerationRequest,
    ) -> Result<orchestrator_flow_rs::GeneratedImagePayload, String> {
        if req.is_svg() {
            return self.default_image.generate_image(req).await;
        }
        let Some(resolved) = self.select_provider("image_create", "image") else {
            return self.default_image.generate_image(req).await;
        };
        let (width, height) = req.effective_dimensions();
        let provider = resolved.to_llm_provider().map_err(|e| e.to_string())?;
        let output = provider
            .invoke(crate::providers::LlmOperation::ImageGeneration {
                prompt: req.prompt.clone(),
                width,
                height,
            })
            .await
            .map_err(|e| e.to_string())?;
        let crate::providers::LlmOutput::Media { bytes, mime_type } = output else {
            return Err("image provider returned non-media output".to_owned());
        };
        let stem = llm_wiki::slugify(
            req.filename
                .trim_end_matches(".png")
                .trim_end_matches(".svg"),
            64,
        );
        let safe_name = format!("{}.png", if stem.is_empty() { "image" } else { &stem });
        Ok(orchestrator_flow_rs::GeneratedImagePayload {
            filename: safe_name,
            mime_type,
            bytes,
            width,
            height,
            provider: resolved.provider_ref.clone(),
        })
    }
}

#[async_trait::async_trait]
impl orchestrator_flow_rs::VideoGeneratorBackend for CatalogMediaBackend {
    async fn generate_video(
        &self,
        req: &orchestrator_flow_rs::VideoGenerationRequest,
    ) -> Result<orchestrator_flow_rs::GeneratedVideoPayload, String> {
        let Some(resolved) = self.select_provider("video_create", "video") else {
            return self.default_video.generate_video(req).await;
        };
        let (width, height) = req.effective_dimensions();
        let duration_secs = req.duration_secs.clamp(1, 60);
        let provider = resolved.to_llm_provider().map_err(|e| e.to_string())?;
        let output = provider
            .invoke(crate::providers::LlmOperation::VideoGeneration {
                prompt: req.prompt.clone(),
                width,
                height,
                duration_secs,
            })
            .await
            .map_err(|e| e.to_string())?;
        let crate::providers::LlmOutput::Media { bytes, mime_type } = output else {
            return Err("video provider returned non-media output".to_owned());
        };
        let stem = llm_wiki::slugify(
            req.filename
                .trim_end_matches(".mp4")
                .trim_end_matches(".webm")
                .trim_end_matches(".mov"),
            64,
        );
        let safe_name = format!("{}.mp4", if stem.is_empty() { "video" } else { &stem });
        Ok(orchestrator_flow_rs::GeneratedVideoPayload {
            filename: safe_name,
            mime_type,
            bytes,
            width,
            height,
            duration_secs,
            provider: resolved.provider_ref.clone(),
        })
    }
}

#[async_trait::async_trait]
impl orchestrator_flow_rs::TtsSynthesizerBackend for CatalogMediaBackend {
    async fn synthesize_tts(
        &self,
        req: &orchestrator_flow_rs::TtsSynthesisRequest,
    ) -> Result<orchestrator_flow_rs::SynthesizedAudioPayload, String> {
        let Some(resolved) = self.select_provider("tts_synthesize", "audio") else {
            return self.default_tts.synthesize_tts(req).await;
        };
        let provider = resolved.to_llm_provider().map_err(|e| e.to_string())?;
        let speed = req.speed.clamp(0.5, 2.0);
        let output = provider
            .invoke(crate::providers::LlmOperation::AudioSpeech {
                text: req.text.clone(),
                voice: (!req.voice.trim().is_empty()).then(|| req.voice.clone()),
                speed: Some(speed),
            })
            .await
            .map_err(|e| e.to_string())?;
        let crate::providers::LlmOutput::Media { bytes, mime_type } = output else {
            return Err("audio provider returned non-media output".to_owned());
        };
        let word_count = req.text.split_whitespace().count().max(1);
        let duration_ms = (((word_count as f32) * 120.0 / speed) as u64).clamp(250, 60_000);
        let stem = llm_wiki::slugify(
            req.filename
                .trim_end_matches(".wav")
                .trim_end_matches(".mp3"),
            64,
        );
        let safe_name = format!("{}.wav", if stem.is_empty() { "tts-audio" } else { &stem });
        Ok(orchestrator_flow_rs::SynthesizedAudioPayload {
            filename: safe_name,
            mime_type,
            bytes,
            sample_rate_hz: 24_000,
            duration_ms,
            provider: resolved.provider_ref.clone(),
        })
    }
}

pub fn project_agents_json(
    cfg: &OrchestratorSectionConfig,
    settings: &PersistedBackgroundSettings,
    catalog: &[ResolvedLlmProvider],
) -> Vec<Value> {
    resolve_effective_agents(cfg, settings, catalog)
        .into_iter()
        .map(|eff| {
            let tool_capability_tags: std::collections::BTreeMap<&str, &str> = eff
                .spec
                .tool_capability_tags()
                .iter()
                .copied()
                .collect();
            let tool_providers: std::collections::BTreeMap<String, String> = settings
                .agents
                .get(eff.spec.id)
                .and_then(|o| o.tool_providers.as_ref())
                .map(|m| {
                    m.iter()
                        .filter(|(t, p)| {
                            !p.trim().is_empty() && eff.spec.tool_required_tag(t).is_some()
                        })
                        .map(|(t, p)| (t.clone(), p.trim().to_owned()))
                        .collect()
                })
                .unwrap_or_default();
            let effective_tool_providers: std::collections::BTreeMap<&str, Option<String>> = eff
                .spec
                .tool_capability_tags()
                .iter()
                .map(|(tool_name, required_tag)| {
                    let resolved = resolve_media_tool_provider(
                        catalog,
                        Some(settings),
                        tool_name,
                        required_tag,
                        false,
                    )
                    .map(|p| p.provider_ref);
                    (*tool_name, resolved)
                })
                .collect();
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
                "tool_capability_tags": tool_capability_tags,
                "tool_providers": tool_providers,
                "effective_tool_providers": effective_tool_providers,
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
