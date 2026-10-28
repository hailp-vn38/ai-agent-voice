use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

const MAX_POOL_FAILOVER_ATTEMPTS: usize = 4;

use serde_json::{Value, json};
use tokio::sync::mpsc;

use crate::{
    config::{LlmInstanceConfig, OpenAiConfig},
    providers::{
        LlmFactory, ProviderLoadError,
        llm::{
            LlmCapabilityTag, LlmError, LlmEvent, LlmEventStream, LlmOperation, LlmOutput,
            LlmProvider, LlmRequest, MAX_REFERENCE_IMAGES, ReferenceImage,
            capability_tags_support, collect_chat_stream, effective_capability_tags,
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
        cdp::BrowserEngine,
        pool::{SharedMusePool, UpstreamFailureKind},
        workflow::{
            MediaKind, WorkflowFailure, format_image_prompt, format_video_prompt,
            run_chat_turn_once, run_media_turn_once,
        },
    },
    wire_anthropic, wire_openai,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WireError {
    NoLease,
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

fn default_tags_for_config(config: &LlmGatewayConfig) -> Vec<LlmCapabilityTag> {
    if let Some(ref tags) = config.tags
        && !tags.is_empty()
    {
        return effective_capability_tags(Some(tags));
    }
    if matches!(&config.gateway_binding, GatewayBinding::Pool(GatewayVendor::Muse))
        || config.model.trim().starts_with("muse-")
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

fn is_retryable_muse_failure(failure: &WorkflowFailure) -> bool {
    matches!(
        failure,
        WorkflowFailure::Upstream(
            UpstreamFailureKind::QuotaExceeded
                | UpstreamFailureKind::AuthInvalid
                | UpstreamFailureKind::Transient,
            _
        )
    )
}

fn report_muse_failure_to_hub(
    hub: &LlmGatewayHub,
    account_key: &str,
    failure: WorkflowFailure,
) -> LlmError {
    match &failure {
        WorkflowFailure::Upstream(UpstreamFailureKind::AuthInvalid, _) => {
            hub.report_outcome(account_key, GatewayCallOutcome::Unauthorized);
        }
        WorkflowFailure::Upstream(UpstreamFailureKind::QuotaExceeded, _) => {
            hub.report_outcome(
                account_key,
                GatewayCallOutcome::RateLimited {
                    retry_after_secs: Some(3600),
                },
            );
        }
        WorkflowFailure::Upstream(UpstreamFailureKind::Transient, _) => {
            hub.report_outcome(
                account_key,
                GatewayCallOutcome::RateLimited {
                    retry_after_secs: Some(30),
                },
            );
        }
        WorkflowFailure::Rejected(_)
        | WorkflowFailure::Timeout(_)
        | WorkflowFailure::Cdp(_) => {}
    }
    failure.into_llm_error()
}

pub struct LlmGatewayProvider {
    adapter_name: &'static str,
    config: LlmGatewayConfig,
    hub: Arc<LlmGatewayHub>,
    http: reqwest::Client,
    tags: Vec<LlmCapabilityTag>,
    muse_engine: Option<Arc<dyn BrowserEngine>>,
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

    pub fn with_hub(
        config: &LlmGatewayConfig,
        hub: Arc<LlmGatewayHub>,
    ) -> Result<Self, LlmError> {
        Self::with_hub_and_adapter(config, hub, ADAPTER_NAME, None)
    }

    #[cfg(test)]
    pub(crate) fn with_hub_and_muse_engine(
        config: &LlmGatewayConfig,
        hub: Arc<LlmGatewayHub>,
        muse_engine: Arc<dyn BrowserEngine>,
    ) -> Result<Self, LlmError> {
        Self::with_hub_and_adapter(config, hub, ADAPTER_NAME, Some(muse_engine))
    }

    fn with_hub_and_adapter(
        config: &LlmGatewayConfig,
        hub: Arc<LlmGatewayHub>,
        adapter_name: &'static str,
        muse_engine: Option<Arc<dyn BrowserEngine>>,
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
            tags: default_tags_for_config(config),
            muse_engine,
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
            account_concurrency: None,
            cdp_port: None,
            headless: None,
        };
        Self::with_hub_and_adapter(&gw_cfg, SharedGatewayHub::global(), "openai", None)
    }

    fn resolve_endpoint_for<'a>(cfg: &'a LlmGatewayConfig, lease: &'a GatewayLease) -> &'a str {
        cfg.base_url
            .as_ref()
            .map(|u| u.as_str().trim_end_matches('/'))
            .unwrap_or(&lease.base_url)
    }

    fn muse_pool_for_lease(
        &self,
        cfg: &LlmGatewayConfig,
        lease: &GatewayLease,
    ) -> Arc<SharedMusePool> {
        SharedMusePool::for_gateway_lease(
            cfg.effective_cdp_port(),
            cfg.effective_headless(),
            cfg.effective_account_concurrency(),
            &lease.account_key,
            lease.access_token.expose(),
            self.muse_engine.clone(),
        )
    }

    fn finalize_wire_result<T>(
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
            Err(WireError::NoLease | WireError::Other) => {}
        }
        result
    }

    pub(crate) async fn open_chat_stream_with_config(
        &self,
        effective_cfg: &LlmGatewayConfig,
        request: &LlmRequest,
    ) -> Result<LlmEventStream, WireError> {
        // 1. Synchronous 0ms lease acquisition from in-RAM GatewayRuntimeSnapshot (ADR-0083 / ADR-0130)
        let lease = self
            .hub
            .acquire_lease(&effective_cfg.gateway_binding)
            .map_err(|_| WireError::NoLease)?;
        let is_pool = matches!(effective_cfg.gateway_binding, GatewayBinding::Pool(_));
        let max_attempts = if is_pool {
            MAX_POOL_FAILOVER_ATTEMPTS
        } else {
            1
        };

        if lease.vendor == GatewayVendor::Muse {
            let timeout = Duration::from_millis(effective_cfg.timeout_ms.max(1_000));
            let req_clone = request.clone();
            let hub = Arc::clone(&self.hub);
            let cfg_clone = effective_cfg.clone();
            let muse_engine = self.muse_engine.clone();
            let (tx, mut rx) = mpsc::unbounded_channel::<Result<LlmEvent, LlmError>>();

            tokio::spawn(async move {
                let mut current_lease = lease;
                for attempt in 0..max_attempts {
                    let pool = SharedMusePool::for_gateway_lease(
                        cfg_clone.effective_cdp_port(),
                        cfg_clone.effective_headless(),
                        cfg_clone.effective_account_concurrency(),
                        &current_lease.account_key,
                        current_lease.access_token.expose(),
                        muse_engine.clone(),
                    );
                    let emitted_delta = Arc::new(AtomicBool::new(false));
                    let emitted_flag = Arc::clone(&emitted_delta);
                    let tx_delta = tx.clone();
                    let res = run_chat_turn_once(&pool, &req_clone, &[], timeout, move |delta| {
                        emitted_flag.store(true, Ordering::Relaxed);
                        let _ = tx_delta.send(Ok(LlmEvent::TextDelta(delta)));
                    })
                    .await;

                    match res {
                        Ok(_) => {
                            hub.report_outcome(
                                &current_lease.account_key,
                                GatewayCallOutcome::Success,
                            );
                            let _ = tx.send(Ok(LlmEvent::Finished));
                            return;
                        }
                        Err(failure) => {
                            let can_failover = is_pool
                                && !emitted_delta.load(Ordering::Relaxed)
                                && is_retryable_muse_failure(&failure)
                                && attempt + 1 < max_attempts;
                            let err = report_muse_failure_to_hub(
                                &hub,
                                &current_lease.account_key,
                                failure,
                            );
                            if can_failover
                                && let Ok(next_lease) =
                                    hub.acquire_lease(&cfg_clone.gateway_binding)
                            {
                                current_lease = next_lease;
                                continue;
                            }
                            let _ = tx.send(Err(err));
                            return;
                        }
                    }
                }
            });

            let stream = futures_util::stream::poll_fn(move |cx| rx.poll_recv(cx));
            return Ok(Box::pin(stream));
        }

        let mut current_lease = lease;
        for attempt in 0..max_attempts {
            let endpoint = Self::resolve_endpoint_for(effective_cfg, &current_lease);
            let response = match current_lease.vendor {
                GatewayVendor::Anthropic => {
                    wire_anthropic::open_sse_stream(
                        &self.http,
                        endpoint,
                        &current_lease,
                        effective_cfg,
                        request,
                    )
                    .await
                }
                GatewayVendor::OpenaiCodex
                | GatewayVendor::Gemini
                | GatewayVendor::GithubCopilot => {
                    wire_openai::open_sse_stream(
                        &self.http,
                        endpoint,
                        &current_lease,
                        effective_cfg,
                        request,
                    )
                    .await
                }
                GatewayVendor::Muse => unreachable!("handled above"),
            };

            let outcome = self.finalize_wire_result(&current_lease.account_key, response);
            match outcome {
                Ok(stream) => return Ok(stream),
                Err(err @ (WireError::RateLimited { .. } | WireError::Unauthorized))
                    if is_pool && attempt + 1 < max_attempts =>
                {
                    if let Ok(next_lease) = self.hub.acquire_lease(&effective_cfg.gateway_binding) {
                        current_lease = next_lease;
                        continue;
                    }
                    return Err(err);
                }
                Err(err) => return Err(err),
            }
        }
        Err(WireError::Other)
    }

    async fn open_chat_stream(
        &self,
        request: LlmRequest,
        force_reasoning: bool,
    ) -> Result<LlmEventStream, LlmError> {
        let mut effective_cfg = self.config.clone();
        if force_reasoning && effective_cfg.reasoning_effort.is_none() {
            effective_cfg.reasoning_effort = Some("medium".to_owned());
        }
        self.open_chat_stream_with_config(&effective_cfg, &request)
            .await
            .map_err(|err| match err {
                WireError::NoLease => LlmError::Rejected,
                _ => LlmError::Failed,
            })
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
            .map_err(|_| LlmError::Rejected)?;
        let is_pool = matches!(self.config.gateway_binding, GatewayBinding::Pool(_));
        let max_attempts = if is_pool {
            MAX_POOL_FAILOVER_ATTEMPTS
        } else {
            1
        };

        if lease.vendor == GatewayVendor::Muse {
            let timeout = Duration::from_millis(self.config.timeout_ms.max(1_000));
            let ref_img = ReferenceImage {
                bytes: image_bytes.to_vec(),
                mime_type: mime_type.to_owned(),
            };
            let req = LlmRequest::text_turn(prompt.to_owned());
            let mut current_lease = lease;
            for attempt in 0..max_attempts {
                let pool = self.muse_pool_for_lease(&self.config, &current_lease);
                match run_chat_turn_once(
                    &pool,
                    &req,
                    std::slice::from_ref(&ref_img),
                    timeout,
                    |_| {},
                )
                .await
                {
                    Ok(text) => {
                        self.hub.report_outcome(
                            &current_lease.account_key,
                            GatewayCallOutcome::Success,
                        );
                        return Ok(LlmOutput::Text(text));
                    }
                    Err(failure) => {
                        let can_failover = is_pool
                            && is_retryable_muse_failure(&failure)
                            && attempt + 1 < max_attempts;
                        let err = report_muse_failure_to_hub(
                            &self.hub,
                            &current_lease.account_key,
                            failure,
                        );
                        if can_failover
                            && let Ok(next_lease) =
                                self.hub.acquire_lease(&self.config.gateway_binding)
                        {
                            current_lease = next_lease;
                            continue;
                        }
                        return Err(err);
                    }
                }
            }
            return Err(LlmError::Failed);
        }

        let mut current_lease = lease;
        for attempt in 0..max_attempts {
            let endpoint = Self::resolve_endpoint_for(&self.config, &current_lease);
            let result = match current_lease.vendor {
                GatewayVendor::Anthropic => {
                    wire_anthropic::execute_vision(
                        &self.http,
                        endpoint,
                        &current_lease,
                        &self.config,
                        prompt,
                        image_bytes,
                        mime_type,
                    )
                    .await
                }
                GatewayVendor::OpenaiCodex
                | GatewayVendor::Gemini
                | GatewayVendor::GithubCopilot => {
                    wire_openai::execute_vision(
                        &self.http,
                        endpoint,
                        &current_lease,
                        &self.config,
                        prompt,
                        image_bytes,
                        mime_type,
                    )
                    .await
                }
                GatewayVendor::Muse => unreachable!("handled above"),
            };

            match self.finalize_wire_result(&current_lease.account_key, result) {
                Ok(out) => return Ok(out),
                Err(WireError::RateLimited { .. } | WireError::Unauthorized)
                    if is_pool && attempt + 1 < max_attempts =>
                {
                    if let Ok(next_lease) = self.hub.acquire_lease(&self.config.gateway_binding) {
                        current_lease = next_lease;
                        continue;
                    }
                    return Err(LlmError::Failed);
                }
                Err(_) => return Err(LlmError::Failed),
            }
        }
        Err(LlmError::Failed)
    }

    async fn run_muse_media(
        &self,
        kind: MediaKind,
        formatted_prompt: &str,
        images: &[ReferenceImage],
        transparent_background: bool,
        required_tag: &'static str,
    ) -> Result<LlmOutput, LlmError> {
        let lease = self
            .hub
            .acquire_lease(&self.config.gateway_binding)
            .map_err(|_| LlmError::Rejected)?;
        if lease.vendor != GatewayVendor::Muse {
            return Err(LlmError::UnsupportedCapability(required_tag));
        }
        let is_pool = matches!(self.config.gateway_binding, GatewayBinding::Pool(_));
        let max_attempts = if is_pool {
            MAX_POOL_FAILOVER_ATTEMPTS
        } else {
            1
        };
        let timeout = Duration::from_millis(self.config.timeout_ms.max(1_000));
        let mut current_lease = lease;
        for attempt in 0..max_attempts {
            let pool = self.muse_pool_for_lease(&self.config, &current_lease);
            match run_media_turn_once(
                &pool,
                kind,
                formatted_prompt,
                images,
                transparent_background,
                timeout,
            )
            .await
            {
                Ok(output) => {
                    self.hub
                        .report_outcome(&current_lease.account_key, GatewayCallOutcome::Success);
                    return Ok(output);
                }
                Err(failure) => {
                    let can_failover = is_pool
                        && is_retryable_muse_failure(&failure)
                        && attempt + 1 < max_attempts;
                    let err = report_muse_failure_to_hub(
                        &self.hub,
                        &current_lease.account_key,
                        failure,
                    );
                    if can_failover
                        && let Ok(next_lease) =
                            self.hub.acquire_lease(&self.config.gateway_binding)
                    {
                        current_lease = next_lease;
                        continue;
                    }
                    return Err(err);
                }
            }
        }
        Err(LlmError::Failed)
    }
}

#[async_trait::async_trait]
impl LlmProvider for LlmGatewayProvider {
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
                self.run_vision(&prompt, &image_bytes, &mime_type).await
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
                self.run_muse_media(
                    MediaKind::Image,
                    &formatted,
                    &reference_images,
                    transparent_background,
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
            } => {
                self.ensure_tag(LlmCapabilityTag::Video)?;
                let formatted = format_video_prompt(&prompt, width, height, duration_secs);
                let images = first_frame.into_iter().collect::<Vec<_>>();
                self.run_muse_media(
                    MediaKind::Video,
                    &formatted,
                    &images,
                    false,
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
        }
        LlmGatewayProvider::from_config(options)
            .map(|p| Arc::new(p) as Arc<dyn LlmProvider>)
            .map_err(|_| {
                ProviderLoadError::Provider("llm_gateway provider initialization failed".into())
            })
    }
}
