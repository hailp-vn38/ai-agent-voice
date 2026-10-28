//! LLM and TTS provider catalog discovery from `[orchestrator]`, `[providers.llm.instances]`,
//! `[providers.tts.instances]`, resident TTS runtimes, and the SQLite `providers` table.

use std::time::Duration;

use orchestrator_agent_rs::{LlmBackend, LlmProviderConfig, ProviderAuth};
use serde_json::{Value, json};

use crate::app::AppState;
use crate::config::{LlmInstanceConfig, OrchestratorSectionConfig, SecretString};

use super::DEFAULT_ORCHESTRATOR_PROVIDER_REF;
use super::specs::{AgentSpec, find_agent_spec};

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
    pub muse_options: Option<MuseCatalogOptions>,
}

impl ResolvedLlmProvider {
    pub fn is_tts_adapter(&self) -> bool {
        matches!(
            self.adapter.as_str(),
            "zerotts_onnx"
                | "chillaudio_ws"
                | "edge_tts"
                | "vieneu"
                | "kokoro_vi_onnx"
                | "vifasttts_onnx"
                | "qualification_tts"
        ) || self.provider_ref.starts_with("config:tts:")
    }

    pub fn requires_api_key(&self) -> bool {
        if self.adapter == "llm_gateway" || self.gateway_binding().is_some() {
            return false;
        }
        !self.is_tts_adapter() || self.adapter == "chillaudio_ws"
    }

    pub fn gateway_binding(&self) -> Option<crate::services::llm_gateway::GatewayBinding> {
        let raw = self.api_key.as_ref()?.expose();
        let binding_str = raw.strip_prefix("gateway_binding:")?;
        crate::services::llm_gateway::GatewayBinding::parse(binding_str).ok()
    }

    pub fn supports_tag(&self, tag: &str) -> bool {
        let normalized = tag.trim().to_ascii_lowercase();
        if self.adapter == "muse" && !matches!(normalized.as_str(), "image" | "video") {
            return false;
        }
        self.tags.iter().any(|t| {
            let candidate = t.trim().to_ascii_lowercase();
            candidate == normalized
                || (normalized == "chat" && candidate == "reasoning")
                || ((normalized == "tts" || normalized == "audio")
                    && (candidate == "tts" || candidate == "audio"))
        }) || ((normalized == "tts" || normalized == "audio") && self.is_tts_adapter())
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
                let gateway_binding = self.gateway_binding();
                let api_key = if gateway_binding.is_some() {
                    crate::config::SecretString::default()
                } else {
                    self.api_key
                        .as_ref()
                        .map(|k| crate::config::SecretString::new(k.expose()))
                        .unwrap_or_default()
                };
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
                    gateway_binding,
                };
                if cfg.gateway_binding.is_some() {
                    return Ok(Box::new(
                        crate::providers::llm::LlmGatewayProvider::from_openai_config(&cfg)?,
                    ));
                }
                Ok(Box::new(
                    crate::providers::llm::ConfiguredOpenAiLlm::from_config(&cfg)?,
                ))
            }
            "llm_gateway" => {
                let Some(gateway_binding) = self.gateway_binding() else {
                    return Err(crate::providers::LlmError::Failed);
                };
                let base_url = (!self.base_url.trim().is_empty())
                    .then(|| url::Url::parse(&self.base_url))
                    .transpose()
                    .map_err(|_| crate::providers::LlmError::Failed)?;
                let cfg = crate::config::LlmGatewayConfig {
                    gateway_binding,
                    model: self.model.clone(),
                    base_url,
                    timeout_ms: self.timeout_ms.max(1_000),
                    max_tokens,
                    context_limit_tokens,
                    reasoning_effort: self.reasoning_effort.clone(),
                    temperature_percent: self.temperature_percent,
                    tags: Some(self.tags.clone()),
                };
                Ok(Box::new(
                    crate::providers::llm::LlmGatewayProvider::from_config(&cfg)?,
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
            "muse" => {
                let api_key = self
                    .api_key
                    .as_ref()
                    .map(|k| crate::config::SecretString::new(k.expose()))
                    .unwrap_or_default();
                let opts = self.muse_options.clone().unwrap_or_default();
                let cfg = crate::config::MuseConfig {
                    api_key,
                    model: self.model.clone(),
                    timeout_ms: self.timeout_ms.clamp(1_000, 600_000),
                    max_tokens,
                    context_limit_tokens,
                    reasoning_effort: self.reasoning_effort.clone(),
                    temperature_percent: self.temperature_percent,
                    tags: Some(self.tags.clone()),
                    account_concurrency: opts.account_concurrency.clamp(1, 8),
                    cdp_port: opts.cdp_port.max(1024),
                    headless: opts.headless,
                };
                Ok(Box::new(
                    crate::providers::llm::MuseLlmProvider::from_config(&cfg)?,
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
        let auth = if let Some(binding) = self.gateway_binding() {
            match crate::services::llm_gateway::SharedGatewayHub::global().acquire_lease(&binding) {
                Ok(lease) => ProviderAuth::ApiKey(orchestrator_agent_rs::SecretString::new(
                    lease.access_token.expose().to_owned(),
                )),
                Err(_) => ProviderAuth::None,
            }
        } else {
            match &self.api_key {
                Some(key) => ProviderAuth::ApiKey(orchestrator_agent_rs::SecretString::new(
                    key.expose().to_owned(),
                )),
                None => ProviderAuth::None,
            }
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
        .map_or(&["chat", "reasoning"][..], AgentSpec::allowed_capability_tags);
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
        muse_options: None,
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
                let api_key = if let Some(ref binding) = openai.gateway_binding {
                    Some(SecretString::new(format!(
                        "gateway_binding:{}",
                        binding.as_config_string()
                    )))
                } else {
                    (!openai.api_key.expose().trim().is_empty())
                        .then(|| SecretString::new(openai.api_key.expose().to_owned()))
                };
                out.push(ResolvedLlmProvider {
                    provider_ref,
                    name: format!("{key} ({})", openai.model),
                    source: "config",
                    adapter: "openai".to_owned(),
                    model: openai.model.clone(),
                    base_url: openai.base_url.to_string(),
                    api_key,
                    timeout_ms: openai.timeout_ms,
                    max_tokens: openai.max_tokens.unwrap_or(orch_cfg.max_output_tokens),
                    context_limit_tokens: openai
                        .context_limit_tokens
                        .map(|v| v as usize)
                        .unwrap_or(orch_cfg.context_limit_tokens),
                    reasoning_effort: openai.reasoning_effort.clone(),
                    temperature_percent: openai.temperature_percent,
                    tags: crate::providers::llm::effective_llm_tags(openai.tags.as_deref()),
                    muse_options: None,
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
                    muse_options: None,
                });
            }
            LlmInstanceConfig::Muse(muse) => {
                let tags = crate::providers::llm::effective_llm_tags(muse.tags.as_deref())
                    .into_iter()
                    .filter(|t| matches!(t.as_str(), "image" | "video"))
                    .collect();
                out.push(ResolvedLlmProvider {
                    provider_ref,
                    name: format!("{key} ({})", muse.model),
                    source: "config",
                    adapter: "muse".to_owned(),
                    model: muse.model.clone(),
                    base_url: crate::providers::llm::muse::SITE_URL.to_owned(),
                    api_key: (!muse.api_key.expose().trim().is_empty())
                        .then(|| SecretString::new(muse.api_key.expose().to_owned())),
                    timeout_ms: muse.timeout_ms,
                    max_tokens: muse.max_tokens.unwrap_or(orch_cfg.max_output_tokens),
                    context_limit_tokens: muse
                        .context_limit_tokens
                        .map(|v| v as usize)
                        .unwrap_or(orch_cfg.context_limit_tokens),
                    reasoning_effort: muse.reasoning_effort.clone(),
                    temperature_percent: muse.temperature_percent,
                    tags,
                    muse_options: Some(MuseCatalogOptions {
                        account_concurrency: muse.account_concurrency,
                        cdp_port: muse.cdp_port,
                        headless: muse.headless,
                    }),
                });
            }
            LlmInstanceConfig::LlmGateway(gw) => {
                let base_url = gw.base_url.as_ref().map(ToString::to_string).unwrap_or_else(
                    || {
                        crate::services::llm_gateway::SharedGatewayHub::global()
                            .default_base_url_for_binding(&gw.gateway_binding)
                            .to_owned()
                    },
                );
                out.push(ResolvedLlmProvider {
                    provider_ref,
                    name: format!("{key} ({})", gw.model),
                    source: "config",
                    adapter: "llm_gateway".to_owned(),
                    model: gw.model.clone(),
                    base_url,
                    api_key: Some(SecretString::new(format!(
                        "gateway_binding:{}",
                        gw.gateway_binding.as_config_string()
                    ))),
                    timeout_ms: gw.timeout_ms,
                    max_tokens: gw.max_tokens.unwrap_or(orch_cfg.max_output_tokens),
                    context_limit_tokens: gw
                        .context_limit_tokens
                        .map(|v| v as usize)
                        .unwrap_or(orch_cfg.context_limit_tokens),
                    reasoning_effort: gw.reasoning_effort.clone(),
                    temperature_percent: gw.temperature_percent,
                    tags: crate::providers::llm::effective_llm_tags(gw.tags.as_deref()),
                    muse_options: None,
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
            let model = parsed
                .get("model")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_owned)
                .or_else(|| {
                    (adapter == "muse")
                        .then(|| crate::providers::llm::muse::MUSE_MODELS[0].to_owned())
                });
            let Some(model) = model else {
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

            let (base_url, api_key, muse_options) = match adapter.as_str() {
                "openai" => {
                    let parsed_binding = parsed
                        .get("gateway_binding")
                        .and_then(Value::as_str)
                        .and_then(|s| crate::services::llm_gateway::GatewayBinding::parse(s).ok());
                    let url = parsed
                        .get("base_url")
                        .and_then(Value::as_str)
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                        .map(str::to_owned)
                        .or_else(|| {
                            parsed_binding.as_ref().map(|b| {
                                crate::services::llm_gateway::SharedGatewayHub::global()
                                    .default_base_url_for_binding(b)
                                    .to_owned()
                            })
                        });
                    let Some(url) = url else {
                        continue;
                    };
                    let resolved_key = if let Some(binding) = parsed_binding {
                        Some(SecretString::new(format!(
                            "gateway_binding:{}",
                            binding.as_config_string()
                        )))
                    } else {
                        resolve_db_provider_secret(
                            state,
                            &key,
                            &adapter,
                            credential_json.as_deref(),
                        )
                    };
                    (url, resolved_key, None)
                }
                "llm_gateway" => {
                    let Some(binding) = parsed
                        .get("gateway_binding")
                        .and_then(Value::as_str)
                        .and_then(|s| crate::services::llm_gateway::GatewayBinding::parse(s).ok())
                    else {
                        continue;
                    };
                    let url = parsed
                        .get("base_url")
                        .and_then(Value::as_str)
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                        .map(str::to_owned)
                        .unwrap_or_else(|| {
                            crate::services::llm_gateway::SharedGatewayHub::global()
                                .default_base_url_for_binding(&binding)
                                .to_owned()
                        });
                    let binding_token = SecretString::new(format!(
                        "gateway_binding:{}",
                        binding.as_config_string()
                    ));
                    (url, Some(binding_token), None)
                }
                "opencode_free" => (
                    crate::providers::llm::opencode_free::BASE_URL.to_owned(),
                    Some(SecretString::new(
                        crate::providers::llm::opencode_free::PUBLIC_KEY,
                    )),
                    None,
                ),
                "muse" => {
                    let url = crate::providers::llm::muse::SITE_URL.to_owned();
                    let resolved_key = resolve_db_provider_secret(
                        state,
                        &key,
                        &adapter,
                        credential_json.as_deref(),
                    );
                    let account_concurrency = parsed
                        .get("account_concurrency")
                        .and_then(Value::as_u64)
                        .and_then(|v| usize::try_from(v).ok())
                        .unwrap_or(2);
                    let cdp_port = parsed
                        .get("cdp_port")
                        .and_then(Value::as_u64)
                        .and_then(|v| u16::try_from(v).ok())
                        .unwrap_or(19222);
                    let headless = parsed
                        .get("headless")
                        .and_then(Value::as_bool)
                        .unwrap_or(true);
                    (
                        url,
                        resolved_key,
                        Some(MuseCatalogOptions {
                            account_concurrency,
                            cdp_port,
                            headless,
                        }),
                    )
                }
                _ => continue,
            };

            let mut tags = crate::providers::llm::llm_config_tags(&config_json);
            if adapter == "muse" {
                tags.retain(|t| matches!(t.as_str(), "image" | "video"));
            }

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
                muse_options,
            });
        }
    }

    // 3. Deployment config TTS instances (`config:tts:<key>`)
    let mut tts_config_keys: Vec<&String> = state.config.providers.tts.instances.keys().collect();
    tts_config_keys.sort();
    for key in tts_config_keys {
        let Some(instance) = state.config.providers.tts.instances.get(key) else {
            continue;
        };
        let adapter = instance.adapter().to_owned();
        let serialized = serde_json::to_value(instance).unwrap_or(Value::Null);
        let (model, base_url) = extract_tts_model_and_url(&adapter, &serialized);
        out.push(ResolvedLlmProvider {
            provider_ref: format!("config:tts:{key}"),
            name: format!("{key} ({model})"),
            source: "config",
            adapter,
            model,
            base_url,
            api_key: None,
            timeout_ms: state.config.tts.timeout_ms,
            max_tokens: orch_cfg.max_output_tokens,
            context_limit_tokens: orch_cfg.context_limit_tokens,
            reasoning_effort: None,
            temperature_percent: None,
            tags: vec!["tts".to_owned()],
            muse_options: None,
        });
    }

    // 4. Database TTS providers (`providers` table where `type='tts' AND enabled=1`)
    if let Some(database) = &state.database
        && let Ok(rows) = sqlx::query_as::<_, (String, String, String, String, Option<String>)>(
            "SELECT key, name, adapter, config_json, credential_json FROM providers WHERE type='tts' AND enabled=1 ORDER BY id",
        )
        .fetch_all(database.pool())
        .await
    {
        for (key, name, adapter, config_json, credential_json) in rows {
            if out.iter().any(|p| p.provider_ref == key) {
                continue;
            }
            let parsed = serde_json::from_str::<Value>(&config_json).unwrap_or(Value::Null);
            let (model, base_url) = extract_tts_model_and_url(&adapter, &parsed);
            let api_key = resolve_db_provider_secret(
                state,
                &key,
                &adapter,
                credential_json.as_deref(),
            );

            out.push(ResolvedLlmProvider {
                provider_ref: key,
                name,
                source: "database",
                adapter,
                model,
                base_url,
                api_key,
                timeout_ms: state.config.tts.timeout_ms,
                max_tokens: orch_cfg.max_output_tokens,
                context_limit_tokens: orch_cfg.context_limit_tokens,
                reasoning_effort: None,
                temperature_percent: None,
                tags: vec!["tts".to_owned()],
                muse_options: None,
            });
        }
    }

    // 5. Resident TTS runtimes (`state.runtimes.tts`) not already covered above
    let mut runtime_tts_keys: Vec<&String> = state.runtimes.tts.keys().collect();
    runtime_tts_keys.sort();
    for key in runtime_tts_keys {
        let Some(runtime) = state.runtimes.tts.get(key) else {
            continue;
        };
        let adapter = runtime.provider().adapter().to_owned();
        if adapter == "unavailable" {
            continue;
        }
        let config_ref = format!("config:tts:{key}");
        if out
            .iter()
            .any(|p| p.provider_ref == *key || p.provider_ref == config_ref)
        {
            continue;
        }
        out.push(ResolvedLlmProvider {
            provider_ref: key.clone(),
            name: format!("{key} ({adapter})"),
            source: "runtime",
            adapter: adapter.clone(),
            model: adapter,
            base_url: String::new(),
            api_key: None,
            timeout_ms: state.config.tts.timeout_ms,
            max_tokens: orch_cfg.max_output_tokens,
            context_limit_tokens: orch_cfg.context_limit_tokens,
            reasoning_effort: None,
            temperature_percent: None,
            tags: vec!["tts".to_owned()],
            muse_options: None,
        });
    }

    out
}

fn resolve_db_provider_secret(
    state: &AppState,
    key: &str,
    adapter: &str,
    credential_json: Option<&str>,
) -> Option<SecretString> {
    let secret_ref = crate::database::credentials::reference(
        &format!("provider:{key}"),
        credential_json,
        crate::database::secrets::provider_secret_env(key, adapter),
    );
    secret_ref
        .and_then(|r| crate::database::secrets::SecretRef::parse(r).ok())
        .and_then(|s| state.secret_resolver.resolve(&s).ok())
        .map(|v| v.expose().trim().to_owned())
        .filter(|s| !s.is_empty())
        .map(SecretString::new)
}

fn extract_tts_model_and_url(adapter: &str, parsed: &Value) -> (String, String) {
    let model = parsed
        .get("model")
        .or_else(|| parsed.get("voice"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or(adapter)
        .to_owned();
    let base_url = parsed
        .get("url")
        .or_else(|| parsed.get("base_url"))
        .and_then(Value::as_str)
        .map(str::trim)
        .unwrap_or_default()
        .to_owned();
    (model, base_url)
}
