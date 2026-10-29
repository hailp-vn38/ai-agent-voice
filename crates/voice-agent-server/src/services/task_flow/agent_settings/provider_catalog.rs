//! LLM and TTS provider catalog discovery from `[orchestrator]`, `[providers.llm.instances]`,
//! `[providers.tts.instances]`, resident TTS runtimes, and the SQLite `providers` table.

use std::time::Duration;

use orchestrator_agent_rs::{LlmBackend, LlmProviderConfig, ProviderAuth};
use serde_json::{Value, json};

use crate::app::AppState;
use crate::config::{LlmInstanceConfig, OrchestratorSectionConfig, SecretString};

use super::DEFAULT_ORCHESTRATOR_PROVIDER_REF;
use super::gateway_executor::GatewayOrchestratorExecutor;
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
    pub gateway_binding: Option<crate::services::llm_gateway::GatewayBinding>,
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

    pub fn is_asr_adapter(&self) -> bool {
        matches!(
            self.adapter.as_str(),
            "zipformer_sherpa" | "gipformer_sherpa_offline" | "qualification_asr"
        ) || self.provider_ref.starts_with("config:asr:")
    }

    pub fn requires_api_key(&self) -> bool {
        if self.adapter == "llm_gateway"
            || self.gateway_binding().is_some()
            || self.is_asr_adapter()
        {
            return false;
        }
        !self.is_tts_adapter() || self.adapter == "chillaudio_ws"
    }

    pub fn gateway_binding(&self) -> Option<crate::services::llm_gateway::GatewayBinding> {
        self.gateway_binding.clone()
    }

    pub fn is_muse_provider(&self) -> bool {
        self.muse_options.is_some()
            || self.provider_ref.eq_ignore_ascii_case("pool:muse")
            || self.adapter.eq_ignore_ascii_case("muse")
            || self.model.to_ascii_lowercase().starts_with("muse")
            || matches!(
                self.gateway_binding,
                Some(crate::services::llm_gateway::GatewayBinding::Pool(
                    crate::services::llm_gateway::GatewayVendor::Muse
                ))
            )
    }

    pub fn video_constraints(&self) -> orchestrator_flow_rs::VideoProviderConstraints {
        let label = format!("{} (model: {})", self.name, self.model);
        if self.is_muse_provider() {
            orchestrator_flow_rs::VideoProviderConstraints {
                provider_label: label,
                max_clip_duration_secs: 10,
                max_clips_per_call: 6,
                max_total_duration_secs: 60,
                provider_guidance: "Generates MP4 video with built-in voiceover/narration audio and burned-in subtitles directly from `prompt` (do NOT call `tts_synthesize` for video requests, and do NOT split `prompt` into 10-second sub-prompts). \
                    Pass a single unified video `prompt` and the target `duration_secs` (up to 60s). \
                    When `duration_secs` exceeds the 10-second single-clip generation limit, the provider concatenates the generated segments and returns the merged result video; \
                    only call `video_concat` if `video_create` returns unmerged clips (`requires_concat: true`) or when the requested duration exceeds a single `video_create` call."
                    .to_owned(),
            }
        } else if self.adapter.eq_ignore_ascii_case("openai") {
            orchestrator_flow_rs::VideoProviderConstraints {
                provider_label: label,
                max_clip_duration_secs: 10,
                max_clips_per_call: 1,
                max_total_duration_secs: 10,
                provider_guidance: "Generates 1 video clip (max 10 seconds) per `video_create` call (with built-in audio and subtitles; do NOT call `tts_synthesize`). \
                    Only when the requested video duration exceeds the 10-second single-call limit, call `video_create` with `role: \"intermediate\"` for each segment and merge them using `video_concat`."
                    .to_owned(),
            }
        } else {
            orchestrator_flow_rs::VideoProviderConstraints {
                provider_label: label,
                max_clip_duration_secs: 60,
                max_clips_per_call: 1,
                max_total_duration_secs: 60,
                provider_guidance: "Generates a single MP4 clip up to 60 seconds per call."
                    .to_owned(),
            }
        }
    }

    pub fn supports_tag(&self, tag: &str) -> bool {
        let normalized = tag.trim().to_ascii_lowercase();
        if normalized == "asr" {
            if self.is_asr_adapter() {
                return true;
            }
            if self.is_tts_adapter() {
                return false;
            }
            return self.tags.iter().any(|t| {
                let candidate = t.trim().to_ascii_lowercase();
                candidate == "asr" || candidate == "audio"
            });
        }
        if normalized == "tts" {
            if self.is_tts_adapter() {
                return true;
            }
            if self.is_asr_adapter() {
                return false;
            }
            return self.tags.iter().any(|t| {
                let candidate = t.trim().to_ascii_lowercase();
                candidate == "tts" || candidate == "audio"
            });
        }
        self.tags.iter().any(|t| {
            let candidate = t.trim().to_ascii_lowercase();
            candidate == normalized
                || (normalized == "chat" && candidate == "reasoning")
                || (normalized == "audio"
                    && (candidate == "audio" || candidate == "tts" || candidate == "asr"))
        }) || (normalized == "audio" && (self.is_tts_adapter() || self.is_asr_adapter()))
    }

    pub(super) fn clamped_max_tokens(&self) -> u32 {
        self.max_tokens.clamp(1, 65_536)
    }

    pub(super) fn clamped_context_limit_tokens(&self) -> u32 {
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
                    account_concurrency: self.muse_options.as_ref().map(|o| o.account_concurrency),
                    cdp_port: self.muse_options.as_ref().map(|o| o.cdp_port),
                    headless: self.muse_options.as_ref().map(|o| o.headless),
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
            _ => Err(crate::providers::LlmError::Failed),
        }
    }

    pub fn to_json_summary(&self) -> Value {
        let mut out = json!({
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
        });
        if let Some(binding) = self.gateway_binding() {
            out["gateway_binding"] = json!(binding.as_config_string());
        }
        out
    }

    pub fn to_orchestrator_provider(&self) -> LlmProviderConfig {
        self.to_orchestrator_provider_with_hub(
            crate::services::llm_gateway::SharedGatewayHub::global(),
        )
    }

    pub fn to_orchestrator_provider_with_hub(
        &self,
        hub: std::sync::Arc<crate::services::llm_gateway::LlmGatewayHub>,
    ) -> LlmProviderConfig {
        let auth = if self.gateway_binding().is_some() {
            match GatewayOrchestratorExecutor::with_hub(self, hub) {
                Ok(exec) => ProviderAuth::CustomExecutor(std::sync::Arc::new(exec)),
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
                let api_key = if openai.gateway_binding.is_some() {
                    None
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
                    gateway_binding: openai.gateway_binding.clone(),
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
                    gateway_binding: None,
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
            LlmInstanceConfig::LlmGateway(gw) => {
                let base_url = gw
                    .base_url
                    .as_ref()
                    .map(ToString::to_string)
                    .unwrap_or_else(|| {
                        state
                            .llm_gateway_hub
                            .default_base_url_for_binding(&gw.gateway_binding)
                            .to_owned()
                    });
                let tags = if gw.tags.is_none()
                    && matches!(
                        &gw.gateway_binding,
                        crate::services::llm_gateway::GatewayBinding::Pool(
                            crate::services::llm_gateway::GatewayVendor::Muse
                        )
                    ) {
                    vec![
                        "chat".to_owned(),
                        "vision".to_owned(),
                        "image".to_owned(),
                        "video".to_owned(),
                    ]
                } else {
                    crate::providers::llm::effective_llm_tags(gw.tags.as_deref())
                };
                let muse_options = (gw.account_concurrency.is_some()
                    || gw.cdp_port.is_some()
                    || gw.headless.is_some())
                .then(|| MuseCatalogOptions {
                    account_concurrency: gw.effective_account_concurrency(),
                    cdp_port: gw.effective_cdp_port(),
                    headless: gw.effective_headless(),
                });
                out.push(ResolvedLlmProvider {
                    provider_ref,
                    name: format!("{key} ({})", gw.model),
                    source: "config",
                    adapter: "llm_gateway".to_owned(),
                    model: gw.model.clone(),
                    base_url,
                    api_key: None,
                    gateway_binding: Some(gw.gateway_binding.clone()),
                    timeout_ms: gw.timeout_ms,
                    max_tokens: gw.max_tokens.unwrap_or(orch_cfg.max_output_tokens),
                    context_limit_tokens: gw
                        .context_limit_tokens
                        .map(|v| v as usize)
                        .unwrap_or(orch_cfg.context_limit_tokens),
                    reasoning_effort: gw.reasoning_effort.clone(),
                    temperature_percent: gw.temperature_percent,
                    tags,
                    muse_options,
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
                .map(str::to_owned);
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

            let (base_url, api_key, gateway_binding, muse_options) = match adapter.as_str() {
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
                                state
                                    .llm_gateway_hub
                                    .default_base_url_for_binding(b)
                                    .to_owned()
                            })
                        });
                    let Some(url) = url else {
                        continue;
                    };
                    let resolved_key = if parsed_binding.is_some() {
                        None
                    } else {
                        resolve_db_provider_secret(
                            state,
                            &key,
                            &adapter,
                            credential_json.as_deref(),
                        )
                    };
                    (url, resolved_key, parsed_binding, None)
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
                            state
                                .llm_gateway_hub
                                .default_base_url_for_binding(&binding)
                                .to_owned()
                        });
                    let account_concurrency = parsed
                        .get("account_concurrency")
                        .and_then(Value::as_u64)
                        .and_then(|v| usize::try_from(v).ok());
                    let cdp_port = parsed
                        .get("cdp_port")
                        .and_then(Value::as_u64)
                        .and_then(|v| u16::try_from(v).ok());
                    let headless = parsed.get("headless").and_then(Value::as_bool);
                    let muse_opts = (account_concurrency.is_some()
                        || cdp_port.is_some()
                        || headless.is_some())
                    .then(|| MuseCatalogOptions {
                        account_concurrency: account_concurrency.unwrap_or(2),
                        cdp_port: cdp_port.unwrap_or(19222),
                        headless: headless.unwrap_or(true),
                    });
                    (url, None, Some(binding), muse_opts)
                }
                "opencode_free" => (
                    crate::providers::llm::opencode_free::BASE_URL.to_owned(),
                    Some(SecretString::new(
                        crate::providers::llm::opencode_free::PUBLIC_KEY,
                    )),
                    None,
                    None,
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
                gateway_binding,
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

    // 3–5. Deployment config, database, and resident TTS providers
    append_speech_provider_group(
        state,
        orch_cfg,
        &mut out,
        "tts",
        state.config.tts.timeout_ms,
        state
            .config
            .providers
            .tts
            .instances
            .iter()
            .map(|(k, v)| {
                (
                    k.clone(),
                    v.adapter().to_owned(),
                    serde_json::to_value(v).unwrap_or(Value::Null),
                )
            })
            .collect(),
        state
            .runtimes
            .tts
            .iter()
            .map(|(k, r)| (k.clone(), r.provider().adapter().to_owned()))
            .collect(),
    )
    .await;

    // 6–8. Deployment config, database, and resident ASR providers
    append_speech_provider_group(
        state,
        orch_cfg,
        &mut out,
        "asr",
        state.config.workers.asr.final_timeout_ms,
        state
            .config
            .providers
            .asr
            .instances
            .iter()
            .map(|(k, v)| {
                (
                    k.clone(),
                    v.adapter().to_owned(),
                    serde_json::to_value(v).unwrap_or(Value::Null),
                )
            })
            .collect(),
        state
            .runtimes
            .asr
            .iter()
            .map(|(k, r)| (k.clone(), r.adapter().to_owned()))
            .collect(),
    )
    .await;

    out
}

async fn append_speech_provider_group(
    state: &AppState,
    orch_cfg: &crate::config::OrchestratorSectionConfig,
    out: &mut Vec<ResolvedLlmProvider>,
    kind: &str,
    timeout_ms: u64,
    mut config_entries: Vec<(String, String, Value)>,
    mut runtime_entries: Vec<(String, String)>,
) {
    config_entries.sort_by(|a, b| a.0.cmp(&b.0));
    for (key, adapter, serialized) in config_entries {
        let (model, base_url) = extract_speech_model_and_url(kind, &adapter, &serialized);
        out.push(ResolvedLlmProvider {
            provider_ref: format!("config:{kind}:{key}"),
            name: format!("{key} ({model})"),
            source: "config",
            adapter,
            model,
            base_url,
            api_key: None,
            gateway_binding: None,
            timeout_ms,
            max_tokens: orch_cfg.max_output_tokens,
            context_limit_tokens: orch_cfg.context_limit_tokens,
            reasoning_effort: None,
            temperature_percent: None,
            tags: vec![kind.to_owned()],
            muse_options: None,
        });
    }

    if let Some(database) = &state.database
        && let Ok(rows) = sqlx::query_as::<_, (String, String, String, String, Option<String>)>(
            "SELECT key, name, adapter, config_json, credential_json FROM providers WHERE type=?1 AND enabled=1 ORDER BY id",
        )
        .bind(kind)
        .fetch_all(database.pool())
        .await
    {
        for (key, name, adapter, config_json, credential_json) in rows {
            if out.iter().any(|p| p.provider_ref == key) {
                continue;
            }
            let parsed = serde_json::from_str::<Value>(&config_json).unwrap_or(Value::Null);
            let (model, base_url) = extract_speech_model_and_url(kind, &adapter, &parsed);
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
                gateway_binding: None,
                timeout_ms,
                max_tokens: orch_cfg.max_output_tokens,
                context_limit_tokens: orch_cfg.context_limit_tokens,
                reasoning_effort: None,
                temperature_percent: None,
                tags: vec![kind.to_owned()],
                muse_options: None,
            });
        }
    }

    runtime_entries.sort_by(|a, b| a.0.cmp(&b.0));
    for (key, adapter) in runtime_entries {
        if adapter == "unavailable" {
            continue;
        }
        let config_ref = format!("config:{kind}:{key}");
        if out
            .iter()
            .any(|p| p.provider_ref == key || p.provider_ref == config_ref)
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
            gateway_binding: None,
            timeout_ms,
            max_tokens: orch_cfg.max_output_tokens,
            context_limit_tokens: orch_cfg.context_limit_tokens,
            reasoning_effort: None,
            temperature_percent: None,
            tags: vec![kind.to_owned()],
            muse_options: None,
        });
    }
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

fn extract_speech_model_and_url(kind: &str, adapter: &str, parsed: &Value) -> (String, String) {
    let raw_model = if kind == "asr" {
        parsed
            .get("model")
            .or_else(|| parsed.get("model_dir"))
            .or_else(|| parsed.get("encoder"))
    } else {
        parsed.get("model").or_else(|| parsed.get("voice"))
    };
    let model = raw_model
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| {
            if kind == "asr" {
                std::path::Path::new(s)
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or(s)
            } else {
                s
            }
        })
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
