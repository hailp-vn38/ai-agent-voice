use std::{sync::Arc, time::Duration};

use serde_json::{Value, json};

use crate::{
    config::{LlmInstanceConfig, OpenAiConfig},
    providers::{
        LlmFactory, ProviderLoadError,
        llm::{
            LlmCapabilityTag, LlmError, LlmEventStream, LlmOperation, LlmOutput, LlmProvider,
            LlmRequest, capability_tags_support, collect_chat_stream, effective_capability_tags,
        },
    },
    services::llm_gateway::{
        GatewayCallOutcome, GatewayLease, GatewayVendor, LlmGatewayHub, SharedGatewayHub,
    },
};

use super::{
    config::{ADAPTER_NAME, LlmGatewayConfig},
    wire_anthropic, wire_openai,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WireError {
    RateLimited { retry_after_secs: Option<u64> },
    Unauthorized,
    Other,
}

pub(crate) fn apply_lease_headers(
    mut builder: reqwest::RequestBuilder,
    lease: &GatewayLease,
) -> reqwest::RequestBuilder {
    builder = builder.bearer_auth(lease.access_token.expose());
    for (k, v) in &lease.extra_headers {
        builder = builder.header(k, v);
    }
    builder
}

pub(crate) fn check_wire_status(resp: reqwest::Response) -> Result<reqwest::Response, WireError> {
    let status = resp.status();
    if status.as_u16() == 429 {
        let retry_after = resp
            .headers()
            .get("retry-after")
            .and_then(|h| h.to_str().ok())
            .and_then(|s| s.trim().parse::<u64>().ok());
        return Err(WireError::RateLimited {
            retry_after_secs: retry_after,
        });
    }
    if matches!(status.as_u16(), 401 | 403) {
        return Err(WireError::Unauthorized);
    }
    if !status.is_success() {
        return Err(WireError::Other);
    }
    Ok(resp)
}

pub(crate) fn parse_tool_arguments(raw: &str) -> Result<Value, LlmError> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Ok(json!({}));
    }
    serde_json::from_str(trimmed).map_err(|_| LlmError::Failed)
}

pub struct LlmGatewayProvider {
    adapter_name: &'static str,
    config: LlmGatewayConfig,
    hub: Arc<LlmGatewayHub>,
    http: reqwest::Client,
    tags: Vec<LlmCapabilityTag>,
}

impl LlmGatewayProvider {
    pub fn from_config(config: &LlmGatewayConfig) -> Result<Self, LlmError> {
        Self::with_hub(config, SharedGatewayHub::global())
    }

    pub fn with_hub(
        config: &LlmGatewayConfig,
        hub: Arc<LlmGatewayHub>,
    ) -> Result<Self, LlmError> {
        Self::with_hub_and_adapter(config, hub, ADAPTER_NAME)
    }

    fn with_hub_and_adapter(
        config: &LlmGatewayConfig,
        hub: Arc<LlmGatewayHub>,
        adapter_name: &'static str,
    ) -> Result<Self, LlmError> {
        if !config.valid() {
            return Err(LlmError::Failed);
        }
        let http = reqwest::Client::builder()
            .timeout(Duration::from_millis(config.timeout_ms.max(1_000)))
            .build()
            .map_err(|_| LlmError::Failed)?;
        Ok(Self {
            adapter_name,
            config: config.clone(),
            hub,
            http,
            tags: effective_capability_tags(config.tags.as_deref()),
        })
    }

    /// Constructs a `LlmGatewayProvider` from an `OpenAiConfig` that specifies `gateway_binding`,
    /// preserving the `"openai"` adapter identity.
    pub fn from_openai_config(config: &OpenAiConfig) -> Result<Self, LlmError> {
        let Some(ref binding) = config.gateway_binding else {
            return Err(LlmError::Failed);
        };
        let default_openai = crate::config::defaults::default_openai_base_url();
        let base_url = (config.base_url != default_openai).then(|| config.base_url.clone());
        let gw_cfg = LlmGatewayConfig {
            gateway_binding: binding.clone(),
            model: config.model.clone(),
            base_url,
            timeout_ms: config.timeout_ms,
            max_tokens: config.max_tokens,
            context_limit_tokens: config.context_limit_tokens,
            reasoning_effort: config.reasoning_effort.clone(),
            temperature_percent: config.temperature_percent,
            tags: config.tags.clone(),
        };
        Self::with_hub_and_adapter(&gw_cfg, SharedGatewayHub::global(), "openai")
    }

    fn resolve_endpoint<'a>(&'a self, lease: &'a GatewayLease) -> &'a str {
        self.config
            .base_url
            .as_ref()
            .map(|u| u.as_str().trim_end_matches('/'))
            .unwrap_or(&lease.base_url)
    }

    fn finalize_wire_result<T>(&self, account_key: &str, result: Result<T, WireError>) -> Result<T, LlmError> {
        match result {
            Ok(out) => {
                self.hub
                    .report_outcome(account_key, GatewayCallOutcome::Success);
                Ok(out)
            }
            Err(WireError::RateLimited { retry_after_secs }) => {
                self.hub.report_outcome(
                    account_key,
                    GatewayCallOutcome::RateLimited { retry_after_secs },
                );
                Err(LlmError::Failed)
            }
            Err(WireError::Unauthorized) => {
                self.hub
                    .report_outcome(account_key, GatewayCallOutcome::Unauthorized);
                Err(LlmError::Failed)
            }
            Err(WireError::Other) => Err(LlmError::Failed),
        }
    }

    async fn open_chat_stream(
        &self,
        request: LlmRequest,
        force_reasoning: bool,
    ) -> Result<LlmEventStream, LlmError> {
        // 1. Synchronous 0ms lease acquisition from in-RAM GatewayRuntimeSnapshot (ADR-0083 / ADR-0130)
        let lease = self
            .hub
            .acquire_lease(&self.config.gateway_binding)
            .map_err(|_| LlmError::Failed)?;
        let endpoint = self.resolve_endpoint(&lease);

        let mut effective_cfg = self.config.clone();
        if force_reasoning && effective_cfg.reasoning_effort.is_none() {
            effective_cfg.reasoning_effort = Some("medium".to_owned());
        }

        // 2. Single upstream attempt — zero automatic provider retries (ADR-0023)
        let response = match lease.vendor {
            GatewayVendor::Anthropic => {
                wire_anthropic::open_sse_stream(
                    &self.http,
                    endpoint,
                    &lease,
                    &effective_cfg,
                    &request,
                )
                .await
            }
            GatewayVendor::OpenaiCodex | GatewayVendor::Gemini | GatewayVendor::GithubCopilot => {
                wire_openai::open_sse_stream(
                    &self.http,
                    endpoint,
                    &lease,
                    &effective_cfg,
                    &request,
                )
                .await
            }
        };

        self.finalize_wire_result(&lease.account_key, response)
    }

    async fn run_vision(
        &self,
        prompt: &str,
        image_bytes: &[u8],
        mime_type: &str,
    ) -> Result<LlmOutput, LlmError> {
        let lease = self
            .hub
            .acquire_lease(&self.config.gateway_binding)
            .map_err(|_| LlmError::Failed)?;
        let endpoint = self.resolve_endpoint(&lease);

        let result = match lease.vendor {
            GatewayVendor::Anthropic => {
                wire_anthropic::execute_vision(
                    &self.http,
                    endpoint,
                    &lease,
                    &self.config,
                    prompt,
                    image_bytes,
                    mime_type,
                )
                .await
            }
            GatewayVendor::OpenaiCodex | GatewayVendor::Gemini | GatewayVendor::GithubCopilot => {
                wire_openai::execute_vision(
                    &self.http,
                    endpoint,
                    &lease,
                    &self.config,
                    prompt,
                    image_bytes,
                    mime_type,
                )
                .await
            }
        };

        self.finalize_wire_result(&lease.account_key, result)
    }
}

#[async_trait::async_trait]
impl LlmProvider for LlmGatewayProvider {
    fn supports_tools(&self) -> bool {
        true
    }

    fn adapter(&self) -> &'static str {
        self.adapter_name
    }

    fn capability_tags(&self) -> &[LlmCapabilityTag] {
        &self.tags
    }

    fn supports_tag(&self, tag: LlmCapabilityTag) -> bool {
        if matches!(
            tag,
            LlmCapabilityTag::Image
                | LlmCapabilityTag::Video
                | LlmCapabilityTag::Audio
                | LlmCapabilityTag::Embedding
        ) {
            return false;
        }
        capability_tags_support(&self.tags, tag)
    }

    async fn stream(&self, request: LlmRequest) -> Result<LlmEventStream, LlmError> {
        self.ensure_tag(LlmCapabilityTag::Chat)?;
        self.open_chat_stream(request, false).await
    }

    async fn execute_operation(&self, op: LlmOperation) -> Result<LlmOutput, LlmError> {
        match op {
            LlmOperation::Chat { request, reasoning } => {
                let required = if reasoning {
                    LlmCapabilityTag::Reasoning
                } else {
                    LlmCapabilityTag::Chat
                };
                self.ensure_tag(required)?;
                let stream = self.open_chat_stream(request, reasoning).await?;
                collect_chat_stream(stream).await
            }
            LlmOperation::Vision {
                prompt,
                image_bytes,
                mime_type,
            } => {
                self.ensure_tag(LlmCapabilityTag::Vision)?;
                self.run_vision(&prompt, &image_bytes, &mime_type).await
            }
            _ => Err(LlmError::UnsupportedCapability(op.required_tag().as_str())),
        }
    }
}

pub struct LlmGatewayFactory;

impl LlmFactory for LlmGatewayFactory {
    fn adapter(&self) -> &'static str {
        ADAPTER_NAME
    }

    fn build(&self, config: &LlmInstanceConfig) -> Result<Arc<dyn LlmProvider>, ProviderLoadError> {
        let LlmInstanceConfig::LlmGateway(options) = config else {
            return Err(ProviderLoadError::Configuration(
                "llm_gateway factory received another adapter config".into(),
            ));
        };
        if !options.valid() {
            return Err(ProviderLoadError::Configuration(
                "invalid llm_gateway configuration".into(),
            ));
        }
        LlmGatewayProvider::from_config(options)
            .map(|p| Arc::new(p) as Arc<dyn LlmProvider>)
            .map_err(|_| {
                ProviderLoadError::Provider("llm_gateway provider initialization failed".into())
            })
    }
}
