use crate::{
    providers::llm::{LlmError, LlmEventStream, LlmRequest},
    services::llm_gateway::{GatewayBinding, GatewayVendor},
};

use super::super::{config::LlmGatewayConfig, wire_anthropic, wire_codex, wire_openai};
use super::{LlmGatewayProvider, MAX_POOL_FAILOVER_ATTEMPTS, WireError};

impl LlmGatewayProvider {
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
            return Ok(self.spawn_muse_chat_stream(
                effective_cfg,
                request,
                lease,
                is_pool,
                max_attempts,
            ));
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
                GatewayVendor::OpenaiCodex => {
                    wire_codex::open_sse_stream(
                        &self.http,
                        endpoint,
                        &current_lease,
                        effective_cfg,
                        request,
                    )
                    .await
                }
                GatewayVendor::Gemini | GatewayVendor::GithubCopilot => {
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

    pub(super) async fn open_chat_stream(
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
}
