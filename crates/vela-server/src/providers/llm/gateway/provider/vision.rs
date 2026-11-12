use std::time::Duration;

use crate::{
    providers::llm::{LlmError, LlmOutput, LlmRequest, ReferenceImage},
    services::llm_gateway::{GatewayBinding, GatewayLease, GatewayVendor},
};

use super::super::{
    muse::workflow::run_chat_turn_once, wire_anthropic, wire_codex, wire_gemini, wire_openai,
};
use super::{LlmGatewayProvider, MAX_POOL_FAILOVER_ATTEMPTS, WireError};

impl LlmGatewayProvider {
    async fn dispatch_vision_wire(
        &self,
        endpoint: &str,
        lease: &GatewayLease,
        prompt: &str,
        image_bytes: &[u8],
        mime_type: &str,
    ) -> Result<LlmOutput, WireError> {
        match lease.vendor {
            GatewayVendor::Anthropic => {
                wire_anthropic::execute_vision(
                    &self.http,
                    endpoint,
                    lease,
                    &self.config,
                    prompt,
                    image_bytes,
                    mime_type,
                )
                .await
            }
            GatewayVendor::OpenaiCodex => {
                wire_codex::execute_vision(
                    &self.http,
                    endpoint,
                    lease,
                    &self.config,
                    prompt,
                    image_bytes,
                    mime_type,
                )
                .await
            }
            GatewayVendor::Gemini => {
                wire_gemini::execute_vision(
                    &self.http,
                    endpoint,
                    lease,
                    &self.config,
                    prompt,
                    image_bytes,
                    mime_type,
                    &self.last_error_detail,
                )
                .await
            }
            GatewayVendor::GithubCopilot => {
                wire_openai::execute_vision(
                    &self.http,
                    endpoint,
                    lease,
                    &self.config,
                    prompt,
                    image_bytes,
                    mime_type,
                )
                .await
            }
            GatewayVendor::Muse => unreachable!("handled above"),
        }
    }

    pub(super) async fn run_vision(
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
            let started_running = std::sync::atomic::AtomicBool::new(false);
            let text = self
                .execute_muse_with_auth_refresh_and_failover(
                    lease,
                    is_pool,
                    max_attempts,
                    &started_running,
                    |pool| {
                        let req_ref = &req;
                        let img_slice = std::slice::from_ref(&ref_img);
                        async move {
                            run_chat_turn_once(&pool, req_ref, img_slice, timeout, |_| {}).await
                        }
                    },
                )
                .await?;
            return Ok(LlmOutput::Text(text));
        }

        let mut current_lease = lease;
        for attempt in 0..max_attempts {
            let endpoint = Self::resolve_endpoint_for(&self.config, &current_lease);
            let mut result = self
                .dispatch_vision_wire(endpoint, &current_lease, prompt, image_bytes, mime_type)
                .await;

            if matches!(result, Err(WireError::Unauthorized))
                && let Some(refreshed_lease) = self
                    .hub
                    .try_refresh_on_unauthorized(
                        &current_lease.account_key,
                        current_lease.access_token.expose(),
                    )
                    .await
            {
                current_lease = refreshed_lease;
                let retry_endpoint = Self::resolve_endpoint_for(&self.config, &current_lease);
                result = self
                    .dispatch_vision_wire(
                        retry_endpoint,
                        &current_lease,
                        prompt,
                        image_bytes,
                        mime_type,
                    )
                    .await;
            }

            match self.finalize_wire_result(&current_lease.account_key, result) {
                Ok(out) => return Ok(out),
                Err(
                    WireError::RateLimited { .. }
                    | WireError::Unauthorized
                    | WireError::HttpRejected
                    | WireError::HttpUnavailable,
                ) if is_pool && attempt + 1 < max_attempts => {
                    if let Ok(next_lease) = self.hub.acquire_lease(&self.config.gateway_binding)
                        && next_lease.account_key != current_lease.account_key
                    {
                        current_lease = next_lease;
                        continue;
                    }
                    return Err(LlmError::Rejected);
                }
                Err(err) => return Err(err.into_llm_error()),
            }
        }
        Err(LlmError::Failed)
    }
}
