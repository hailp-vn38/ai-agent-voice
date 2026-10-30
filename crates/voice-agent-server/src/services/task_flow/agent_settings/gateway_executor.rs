//! Dynamic [`orchestrator_agent_rs::CustomModelExecutor`] bridge for OAuth-backed
//! [`ResolvedLlmProvider`] instances (`llm_gateway` or `openai` with `gateway_binding`).

use std::sync::{Arc, Mutex};

use futures_util::StreamExt;
use orchestrator_agent_rs::{
    CustomModelExecutor, GatewayError, ModelRequest, ModelResponse, ModelRole, ModelToolCall,
    ReasoningEffort, StreamEvent, StreamSink, strip_thinking_blocks,
};
use tokio_util::sync::CancellationToken;

use crate::{
    config::LlmGatewayConfig,
    providers::llm::{
        ChatMessage, LlmError, LlmEvent, LlmGatewayProvider, LlmRequest, ToolCall, ToolDefinition,
        gateway::provider::WireError,
    },
    services::llm_gateway::{LlmGatewayHub, SharedGatewayHub},
};

use super::provider_catalog::ResolvedLlmProvider;

/// Bridges an OAuth-backed [`ResolvedLlmProvider`] (`llm_gateway` or `openai` with `gateway_binding`)
/// into [`CustomModelExecutor`] so every Root Orchestrator or Sub-Agent turn acquires a fresh 0ms
/// lease from [`LlmGatewayHub`]'s in-RAM snapshot, attaches vendor-specific `extra_headers`,
/// speaks either OpenAI or Anthropic SSE, and reports 429/401 outcomes back to the pool.
#[derive(Debug)]
pub struct GatewayOrchestratorExecutor {
    fingerprint: String,
    base_config: LlmGatewayConfig,
    provider: LlmGatewayProvider,
    last_error_detail: Mutex<Option<String>>,
}

impl GatewayOrchestratorExecutor {
    fn set_error_detail(&self, detail: impl Into<String>) {
        let text = detail.into();
        let trimmed = text.trim();
        if !trimmed.is_empty() {
            *self
                .last_error_detail
                .lock()
                .unwrap_or_else(|p| p.into_inner()) = Some(trimmed.to_owned());
        }
    }

    pub fn from_resolved(resolved: &ResolvedLlmProvider) -> Result<Self, LlmError> {
        Self::with_hub(resolved, SharedGatewayHub::global())
    }

    pub fn with_hub(
        resolved: &ResolvedLlmProvider,
        hub: Arc<LlmGatewayHub>,
    ) -> Result<Self, LlmError> {
        let Some(gateway_binding) = resolved.gateway_binding() else {
            return Err(LlmError::Failed);
        };
        let default_for_binding = hub.default_base_url_for_binding(&gateway_binding);
        let default_openai = crate::config::defaults::default_openai_base_url();
        let base_url = (!resolved.base_url.trim().is_empty())
            .then(|| url::Url::parse(&resolved.base_url))
            .transpose()
            .map_err(|_| LlmError::Failed)?
            .filter(|u| {
                let trimmed = u.as_str().trim_end_matches('/');
                trimmed != default_for_binding.trim_end_matches('/')
                    && !(resolved.adapter == "openai" && *u == default_openai)
            });
        let base_config = LlmGatewayConfig {
            gateway_binding: gateway_binding.clone(),
            model: resolved.model.clone(),
            base_url,
            timeout_ms: resolved.timeout_ms.max(1_000),
            max_tokens: Some(resolved.clamped_max_tokens()),
            context_limit_tokens: Some(resolved.clamped_context_limit_tokens()),
            reasoning_effort: resolved.reasoning_effort.clone(),
            temperature_percent: resolved.temperature_percent,
            tags: Some(resolved.tags.clone()),
            account_concurrency: resolved
                .muse_options
                .as_ref()
                .map(|o| o.account_concurrency),
            cdp_port: resolved.muse_options.as_ref().map(|o| o.cdp_port),
            headless: resolved.muse_options.as_ref().map(|o| o.headless),
        };
        let provider = LlmGatewayProvider::with_hub(&base_config, hub)?;
        let fingerprint = format!(
            "llm_gateway:{}:{}",
            gateway_binding.as_config_string(),
            resolved.base_url.trim()
        );
        Ok(Self {
            fingerprint,
            base_config,
            provider,
            last_error_detail: Mutex::new(None),
        })
    }
}

#[async_trait::async_trait]
impl CustomModelExecutor for GatewayOrchestratorExecutor {
    fn routing_fingerprint(&self) -> String {
        self.fingerprint.clone()
    }

    fn take_last_error_detail(&self) -> Option<String> {
        self.last_error_detail
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .take()
            .or_else(|| self.provider.take_last_error_detail())
    }

    async fn complete(
        &self,
        request: &ModelRequest,
        cancel: &CancellationToken,
        sink: StreamSink<'_>,
    ) -> Result<ModelResponse, GatewayError> {
        *self
            .last_error_detail
            .lock()
            .unwrap_or_else(|p| p.into_inner()) = None;
        self.provider.clear_last_error_detail();

        let mut effective_cfg = self.base_config.clone();
        if !request.model_id.trim().is_empty() {
            effective_cfg.model = request.model_id.trim().to_owned();
        }
        if let Some(max_tok) = request.params.max_output_tokens {
            effective_cfg.max_tokens = Some(max_tok.max(1));
        }
        if let Some(temp) = request.params.temperature {
            effective_cfg.temperature_percent = Some((temp.clamp(0.0, 2.0) * 100.0).round() as u16);
        }
        if let Some(effort) = request.params.reasoning_effort {
            let mapped = match effort {
                ReasoningEffort::Low => "low",
                ReasoningEffort::Medium => "medium",
                ReasoningEffort::High => "high",
            };
            effective_cfg.reasoning_effort = Some(mapped.to_owned());
        }

        let binding_label = effective_cfg.gateway_binding.as_config_string();
        let model_label = effective_cfg.model.clone();

        let mut messages = Vec::with_capacity(request.messages.len() + 1);
        if !request.system.trim().is_empty() {
            messages.push(ChatMessage::System {
                content: request.system.clone(),
            });
        }
        for m in &request.messages {
            match m.role {
                ModelRole::User => {
                    messages.push(ChatMessage::User {
                        content: m.text.clone(),
                    });
                }
                ModelRole::Assistant if m.tool_calls.is_empty() => {
                    messages.push(ChatMessage::AssistantText {
                        content: m.text.clone(),
                    });
                }
                ModelRole::Assistant => {
                    if !m.text.trim().is_empty() {
                        messages.push(ChatMessage::AssistantText {
                            content: m.text.clone(),
                        });
                    }
                    let calls = m
                        .tool_calls
                        .iter()
                        .map(|tc| ToolCall {
                            id: tc.id.clone(),
                            name: tc.name.clone(),
                            arguments: tc.arguments.clone(),
                        })
                        .collect();
                    messages.push(ChatMessage::AssistantToolCalls { calls });
                }
                ModelRole::Tool => {
                    messages.push(ChatMessage::ToolResult {
                        tool_call_id: m.tool_call_id.clone().unwrap_or_default(),
                        content: m.text.clone(),
                    });
                }
            }
        }

        let tools = request
            .tools
            .iter()
            .map(|t| ToolDefinition {
                name: t.name.clone(),
                description: t.description.clone(),
                parameters: t.input_schema.clone(),
            })
            .collect();

        let llm_request = LlmRequest { messages, tools };

        let mut stream = tokio::select! {
            () = cancel.cancelled() => return Err(GatewayError::Cancelled),
            res = self.provider.open_chat_stream_with_config(&effective_cfg, &llm_request) => {
                res.map_err(|err| {
                    let wire_detail = self.provider.take_last_error_detail();
                    let fallback = match err {
                        WireError::Unauthorized => format!(
                            "OAuth gateway HTTP 401/403 unauthorized (binding='{binding_label}', model='{model_label}')"
                        ),
                        WireError::NoLease => format!(
                            "no active OAuth account lease available (binding='{binding_label}', model='{model_label}')"
                        ),
                        WireError::RateLimited { retry_after_secs } => format!(
                            "upstream HTTP 429 rate limited (binding='{binding_label}', model='{model_label}', retry_after={retry_after_secs:?}s)"
                        ),
                        WireError::HttpRejected => format!(
                            "upstream HTTP request rejected (binding='{binding_label}', model='{model_label}')"
                        ),
                        WireError::Other => format!(
                            "upstream HTTP transport/gateway failure (binding='{binding_label}', model='{model_label}')"
                        ),
                    };
                    self.set_error_detail(wire_detail.unwrap_or(fallback));
                    match err {
                        WireError::Unauthorized => GatewayError::Auth,
                        WireError::NoLease
                        | WireError::RateLimited { .. }
                        | WireError::HttpRejected
                        | WireError::Other => {
                            GatewayError::Provider
                        }
                    }
                })?
            }
        };

        let mut raw_text = String::new();
        let mut tool_calls = Vec::new();
        let mut finished = false;

        loop {
            let next_item = tokio::select! {
                () = cancel.cancelled() => return Err(GatewayError::Cancelled),
                item = stream.next() => item,
            };
            let Some(item) = next_item else {
                break;
            };
            let event = match item {
                Ok(ev) => ev,
                Err(_) => {
                    let detail = self.provider.take_last_error_detail().unwrap_or_else(|| {
                        format!(
                            "SSE stream failed mid-response after {} chars and {} tool calls (binding='{binding_label}', model='{model_label}')",
                            raw_text.len(),
                            tool_calls.len()
                        )
                    });
                    self.set_error_detail(detail);
                    return Err(GatewayError::Protocol);
                }
            };
            match event {
                LlmEvent::TextDelta(delta) => {
                    if !delta.is_empty() {
                        sink(StreamEvent::TextDelta(delta.clone()));
                        raw_text.push_str(&delta);
                    }
                }
                LlmEvent::ToolCall(tc) => {
                    sink(StreamEvent::ToolCallReady {
                        id: tc.id.clone(),
                        name: tc.name.clone(),
                    });
                    tool_calls.push(ModelToolCall {
                        id: tc.id,
                        name: tc.name,
                        arguments: tc.arguments,
                    });
                }
                LlmEvent::Finished => {
                    finished = true;
                    break;
                }
            }
        }

        if !finished {
            let detail = self.provider.take_last_error_detail().unwrap_or_else(|| {
                format!(
                    "SSE stream closed before completion signal after {} chars and {} tool calls (binding='{binding_label}', model='{model_label}')",
                    raw_text.len(),
                    tool_calls.len()
                )
            });
            self.set_error_detail(detail);
            return Err(GatewayError::Protocol);
        }
        let clean_text = strip_thinking_blocks(&raw_text);
        if clean_text.trim().is_empty() && tool_calls.is_empty() {
            let detail = self.provider.take_last_error_detail().unwrap_or_else(|| {
                if !raw_text.trim().is_empty() {
                    format!(
                        "model returned only <think> reasoning ({} chars) with no final text or tool calls (binding='{binding_label}', model='{model_label}')",
                        raw_text.len()
                    )
                } else {
                    format!(
                        "model finished with empty text and no tool calls (binding='{binding_label}', model='{model_label}')"
                    )
                }
            });
            self.set_error_detail(detail);
            return Err(GatewayError::Protocol);
        }
        let finish_reason = if tool_calls.is_empty() {
            "stop".to_owned()
        } else {
            "tool_calls".to_owned()
        };
        Ok(ModelResponse {
            text: clean_text,
            finish_reason,
            tool_calls,
            usage: None,
        })
    }
}
