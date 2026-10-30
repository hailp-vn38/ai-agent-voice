use std::time::Duration;

use orchestrator_agent_rs::{LlmBackend, LlmProviderConfig, ProviderAuth};
use serde_json::{Value, json};

use crate::config::SecretString;

use super::super::gateway_executor::GatewayOrchestratorExecutor;
use super::MuseCatalogOptions;

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
            "zipformer_sherpa"
                | "gipformer_sherpa_offline"
                | "sensevoice_sherpa_offline"
                | "qualification_asr"
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
                    When `duration_secs` exceeds the 10-second single-clip generation limit, `video_create` automatically merges the generated segments into a single continuous primary MP4 artifact."
                    .to_owned(),
            }
        } else if self.adapter.eq_ignore_ascii_case("openai") {
            orchestrator_flow_rs::VideoProviderConstraints {
                provider_label: label,
                max_clip_duration_secs: 10,
                max_clips_per_call: 1,
                max_total_duration_secs: 10,
                provider_guidance: "Generates 1 video clip (max 10 seconds) per `video_create` call (with built-in audio and subtitles; do NOT call `tts_synthesize`)."
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

    pub fn tts_constraints(
        &self,
        runtime_default_voice: Option<&str>,
    ) -> orchestrator_flow_rs::TtsProviderConstraints {
        self.tts_constraints_with_runtime(None, runtime_default_voice)
    }

    pub fn tts_constraints_with_runtime(
        &self,
        runtime_model_id: Option<&str>,
        runtime_default_voice: Option<&str>,
    ) -> orchestrator_flow_rs::TtsProviderConstraints {
        let spec = runtime_model_id.unwrap_or(&self.model);
        let cat = crate::providers::tts::voice_catalog::catalog_for_adapter(
            &self.adapter,
            Some(spec),
            runtime_default_voice,
        );
        orchestrator_flow_rs::TtsProviderConstraints {
            provider_label: format!("{} (model: {})", self.name, self.model),
            default_voice: cat.default_voice,
            female_voices: cat.female_voices,
            male_voices: cat.male_voices,
            provider_guidance: cat.guidance,
        }
    }

    pub fn resolve_tts_voice(
        &self,
        runtime_default_voice: Option<&str>,
        requested_voice: Option<&str>,
    ) -> String {
        self.resolve_tts_voice_with_runtime(None, runtime_default_voice, requested_voice)
    }

    pub fn resolve_tts_voice_with_runtime(
        &self,
        runtime_model_id: Option<&str>,
        runtime_default_voice: Option<&str>,
        requested_voice: Option<&str>,
    ) -> String {
        let spec = runtime_model_id.unwrap_or(&self.model);
        crate::providers::tts::voice_catalog::resolve_tts_voice(
            &self.adapter,
            Some(spec),
            runtime_default_voice,
            requested_voice,
        )
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

    pub(crate) fn clamped_max_tokens(&self) -> u32 {
        self.max_tokens.clamp(1, 65_536)
    }

    pub(crate) fn clamped_context_limit_tokens(&self) -> u32 {
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
