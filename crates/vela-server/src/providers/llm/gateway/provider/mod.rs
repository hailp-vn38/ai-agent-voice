mod chat;
mod muse_ops;
mod muse_task;
mod vision;
mod wire;

use std::{sync::Arc, time::Duration};

pub(super) const MAX_POOL_FAILOVER_ATTEMPTS: usize = 4;

use crate::{
    config::{LlmInstanceConfig, OpenAiConfig},
    providers::{
        LlmFactory, ProviderLoadError,
        llm::{
            DurableMediaProvider, LlmCapabilityTag, LlmError, LlmEventStream, LlmOperation,
            LlmOutput, LlmProvider, LlmRequest, MAX_REFERENCE_IMAGES, capability_tags_support,
            collect_chat_stream, effective_capability_tags,
        },
    },
    services::llm_gateway::{
        GatewayBinding, GatewayCallOutcome, GatewayLease, GatewayVendor, LlmGatewayHub,
        SharedGatewayHub,
    },
};

use super::{
    config::{ADAPTER_NAME, LlmGatewayConfig},
    muse::{
        client::MuseVmTransport,
        pool::VideoRequestSignature,
        workflow::{MediaBatch, MediaKind, format_image_prompt, format_video_batches},
    },
};

pub(crate) use wire::{
    ActivityThrottle, ErrorDetailSlot, NoLeaseKind, WireError, apply_lease_headers,
    check_wire_status, drain_next_sse_line, extract_sse_error_detail, new_error_detail_slot,
    parse_tool_arguments, record_wire_detail, summarize_error_body,
};

fn default_tags_for_config(config: &LlmGatewayConfig) -> Vec<LlmCapabilityTag> {
    if let Some(ref tags) = config.tags
        && !tags.is_empty()
    {
        return effective_capability_tags(Some(tags));
    }
    if matches!(
        &config.gateway_binding,
        GatewayBinding::Pool(GatewayVendor::Muse)
    ) || config.model.trim().starts_with("muse-")
    {
        return vec![
            LlmCapabilityTag::Chat,
            LlmCapabilityTag::Vision,
            LlmCapabilityTag::Image,
            LlmCapabilityTag::Video,
        ];
    }
    effective_capability_tags(config.tags.as_deref())
}

pub struct LlmGatewayProvider {
    pub(super) adapter_name: &'static str,
    pub(super) config: LlmGatewayConfig,
    pub(super) hub: Arc<LlmGatewayHub>,
    pub(super) http: reqwest::Client,
    pub(super) tags: Vec<LlmCapabilityTag>,
    pub(super) muse_transport: Option<Arc<dyn MuseVmTransport>>,
    pub(crate) last_error_detail: ErrorDetailSlot,
}

impl std::fmt::Debug for LlmGatewayProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LlmGatewayProvider")
            .field("adapter_name", &self.adapter_name)
            .field("config", &self.config)
            .finish()
    }
}

impl LlmGatewayProvider {
    pub fn from_config(config: &LlmGatewayConfig) -> Result<Self, LlmError> {
        Self::with_hub(config, SharedGatewayHub::global())
    }

    pub fn with_hub(config: &LlmGatewayConfig, hub: Arc<LlmGatewayHub>) -> Result<Self, LlmError> {
        Self::with_hub_and_adapter(config, hub, ADAPTER_NAME, None)
    }

    #[cfg(test)]
    pub(crate) fn with_hub_and_muse_transport(
        config: &LlmGatewayConfig,
        hub: Arc<LlmGatewayHub>,
        muse_transport: Arc<dyn MuseVmTransport>,
    ) -> Result<Self, LlmError> {
        Self::with_hub_and_adapter(config, hub, ADAPTER_NAME, Some(muse_transport))
    }

    fn with_hub_and_adapter(
        config: &LlmGatewayConfig,
        hub: Arc<LlmGatewayHub>,
        adapter_name: &'static str,
        muse_transport: Option<Arc<dyn MuseVmTransport>>,
    ) -> Result<Self, LlmError> {
        if !config.valid() {
            return Err(LlmError::Failed);
        }
        let limits = config.resolved_limits();
        let read_timeout = limits.first_byte.max(limits.idle) + Duration::from_secs(30);
        let http = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(15))
            .read_timeout(read_timeout)
            .build()
            .map_err(|_| LlmError::Failed)?;
        Ok(Self {
            adapter_name,
            config: config.clone(),
            hub,
            http,
            tags: default_tags_for_config(config),
            muse_transport,
            last_error_detail: new_error_detail_slot(),
        })
    }

    pub(crate) fn clear_last_error_detail(&self) {
        *self
            .last_error_detail
            .lock()
            .unwrap_or_else(|p| p.into_inner()) = None;
    }

    pub(crate) fn take_last_error_detail(&self) -> Option<String> {
        self.last_error_detail
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .take()
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
            timeout_profile: config.timeout_profile,
            first_byte_ms: config.first_byte_ms,
            idle_ms: config.idle_ms,
            max_total_ms: config.max_total_ms,
            max_tokens: config.max_tokens,
            context_limit_tokens: config.context_limit_tokens,
            reasoning_effort: config.reasoning_effort.clone(),
            temperature_percent: config.temperature_percent,
            tags: config.tags.clone(),
            account_concurrency: None,
            cdp_port: None,
            headless: None,
        };
        Self::with_hub_and_adapter(&gw_cfg, SharedGatewayHub::global(), "openai", None)
    }

    pub(super) fn resolve_endpoint_for<'a>(
        cfg: &'a LlmGatewayConfig,
        lease: &'a GatewayLease,
    ) -> &'a str {
        if let Some(ref custom) = cfg.base_url {
            let trimmed = custom.as_str().trim_end_matches('/');
            let is_legacy_codex_openai_url = lease.vendor == GatewayVendor::OpenaiCodex
                && matches!(
                    trimmed,
                    "https://api.openai.com/v1" | "https://api.openai.com"
                );
            let is_legacy_gemini_url = lease.vendor == GatewayVendor::Gemini
                && matches!(
                    trimmed,
                    "https://generativelanguage.googleapis.com/v1beta/openai"
                        | "https://generativelanguage.googleapis.com/v1beta"
                        | "https://api.openai.com/v1"
                        | "https://api.openai.com"
                );
            let is_other_vendor_default = GatewayVendor::ALL.iter().any(|v| {
                *v != lease.vendor && v.default_base_url().trim_end_matches('/') == trimmed
            });
            if !is_legacy_codex_openai_url && !is_legacy_gemini_url && !is_other_vendor_default {
                return trimmed;
            }
        }
        lease.base_url.trim_end_matches('/')
    }

    pub(super) fn finalize_wire_result<T>(
        &self,
        account_key: &str,
        result: Result<T, WireError>,
    ) -> Result<T, WireError> {
        match &result {
            Ok(_) => self
                .hub
                .report_outcome(account_key, GatewayCallOutcome::Success),
            Err(WireError::RateLimited { retry_after_secs }) => {
                self.hub.report_outcome(
                    account_key,
                    GatewayCallOutcome::RateLimited {
                        retry_after_secs: *retry_after_secs,
                    },
                );
            }
            Err(WireError::Unauthorized) => {
                self.hub
                    .report_outcome(account_key, GatewayCallOutcome::Unauthorized);
            }
            Err(
                WireError::NoLease(_)
                | WireError::HttpRejected
                | WireError::HttpUnavailable
                | WireError::Other,
            ) => {}
        }
        result
    }
}

#[async_trait::async_trait]
impl LlmProvider for LlmGatewayProvider {
    fn agent_task(&self) -> Option<&dyn crate::providers::llm::AgentTaskProvider> {
        let muse = matches!(
            &self.config.gateway_binding,
            GatewayBinding::Pool(GatewayVendor::Muse)
        ) || self.config.model.trim().starts_with("muse-")
            || match &self.config.gateway_binding {
                GatewayBinding::Account(key) => self
                    .hub
                    .acquire_account_for_polling(key)
                    .is_ok_and(|lease| lease.vendor == GatewayVendor::Muse),
                _ => false,
            };
        muse.then_some(self as &dyn crate::providers::llm::AgentTaskProvider)
    }

    fn durable_media(&self) -> Option<&dyn DurableMediaProvider> {
        let is_muse = matches!(
            &self.config.gateway_binding,
            GatewayBinding::Pool(GatewayVendor::Muse)
        ) || self.config.model.trim().starts_with("muse-")
            || self.supports_tag(LlmCapabilityTag::Video);
        is_muse.then_some(self as &dyn DurableMediaProvider)
    }

    fn supports_tools(&self) -> bool {
        !matches!(
            &self.config.gateway_binding,
            GatewayBinding::Pool(GatewayVendor::Muse)
        )
    }

    fn adapter(&self) -> &'static str {
        self.adapter_name
    }

    fn capability_tags(&self) -> &[LlmCapabilityTag] {
        &self.tags
    }

    fn supports_tag(&self, tag: LlmCapabilityTag) -> bool {
        if matches!(tag, LlmCapabilityTag::Audio | LlmCapabilityTag::Embedding) {
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
                let out = collect_chat_stream(stream).await?;
                if let LlmOutput::Text(ref t) = out
                    && t.trim().is_empty()
                    && matches!(
                        &self.config.gateway_binding,
                        GatewayBinding::Pool(GatewayVendor::Muse)
                    )
                {
                    return Err(LlmError::Rejected);
                }
                Ok(out)
            }
            LlmOperation::Vision {
                prompt,
                image_bytes,
                mime_type,
            } => {
                self.ensure_tag(LlmCapabilityTag::Vision)?;
                let max_total = self.config.resolved_limits().max_total;
                tokio::time::timeout(
                    max_total,
                    self.run_vision(&prompt, &image_bytes, &mime_type),
                )
                .await
                .map_err(|_| LlmError::Timeout)?
            }
            LlmOperation::ImageGeneration {
                prompt,
                width,
                height,
                reference_images,
                transparent_background,
            } => {
                self.ensure_tag(LlmCapabilityTag::Image)?;
                if reference_images.len() > MAX_REFERENCE_IMAGES {
                    return Err(LlmError::Rejected);
                }
                let formatted = format_image_prompt(&prompt, width, height, transparent_background);
                let batches = [MediaBatch {
                    prompt: formatted,
                    target_secs: None,
                }];
                self.run_muse_media(
                    MediaKind::Image,
                    &batches,
                    &reference_images,
                    transparent_background,
                    None,
                    None,
                    None,
                    None,
                    LlmCapabilityTag::Image.as_str(),
                )
                .await
            }
            LlmOperation::VideoGeneration {
                prompt,
                width,
                height,
                duration_secs,
                first_frame,
                continuation_key,
            } => {
                self.ensure_tag(LlmCapabilityTag::Video)?;
                // Longer than Muse can deliver in one call: reject instead of silently shortening.
                let Some(batches) = format_video_batches(&prompt, width, height, duration_secs)
                else {
                    return Err(LlmError::Rejected);
                };
                let images = first_frame.into_iter().collect::<Vec<_>>();
                let request_sig = Some(VideoRequestSignature {
                    prompt,
                    duration_secs,
                });
                self.run_muse_media(
                    MediaKind::Video,
                    &batches,
                    &images,
                    false,
                    continuation_key,
                    request_sig,
                    None,
                    None,
                    LlmCapabilityTag::Video.as_str(),
                )
                .await
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
        };
        LlmGatewayProvider::from_config(options)
            .map(|p| Arc::new(p) as Arc<dyn LlmProvider>)
            .map_err(|_| {
                ProviderLoadError::Provider("llm_gateway provider initialization failed".into())
            })
    }
}
