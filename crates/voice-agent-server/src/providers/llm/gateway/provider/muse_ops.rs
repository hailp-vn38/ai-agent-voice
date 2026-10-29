use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use tokio::sync::mpsc;

use crate::{
    providers::llm::{LlmError, LlmEvent, LlmEventStream, LlmOutput, LlmRequest, ReferenceImage},
    services::llm_gateway::{
        GatewayBinding, GatewayCallOutcome, GatewayLease, GatewayVendor, LlmGatewayHub,
    },
};

use super::super::{
    config::LlmGatewayConfig,
    muse::{
        pool::{SharedMusePool, UpstreamFailureKind},
        workflow::{MediaKind, WorkflowFailure, run_chat_turn_once, run_media_turn_once},
    },
};
use super::{LlmGatewayProvider, MAX_POOL_FAILOVER_ATTEMPTS};

pub(super) fn is_retryable_muse_failure(failure: &WorkflowFailure) -> bool {
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

pub(super) fn report_muse_failure_to_hub(
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
        WorkflowFailure::Rejected(_) | WorkflowFailure::Timeout(_) | WorkflowFailure::Cdp(_) => {}
    }
    failure.into_llm_error()
}

impl LlmGatewayProvider {
    pub(super) fn muse_pool_for_lease(
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

    pub(super) fn spawn_muse_chat_stream(
        &self,
        effective_cfg: &LlmGatewayConfig,
        request: &LlmRequest,
        lease: GatewayLease,
        is_pool: bool,
        max_attempts: usize,
    ) -> LlmEventStream {
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
                        hub.report_outcome(&current_lease.account_key, GatewayCallOutcome::Success);
                        let _ = tx.send(Ok(LlmEvent::Finished));
                        return;
                    }
                    Err(failure) => {
                        let can_failover = is_pool
                            && !emitted_delta.load(Ordering::Relaxed)
                            && is_retryable_muse_failure(&failure)
                            && attempt + 1 < max_attempts;
                        let err =
                            report_muse_failure_to_hub(&hub, &current_lease.account_key, failure);
                        if can_failover
                            && let Ok(next_lease) = hub.acquire_lease(&cfg_clone.gateway_binding)
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
        Box::pin(stream)
    }

    pub(super) async fn run_muse_media(
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
        let min_timeout_ms = match kind {
            MediaKind::Video => 600_000,
            MediaKind::Image => 1_000,
        };
        let timeout = Duration::from_millis(self.config.timeout_ms.max(min_timeout_ms));
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
                    let err =
                        report_muse_failure_to_hub(&self.hub, &current_lease.account_key, failure);
                    if can_failover
                        && let Ok(next_lease) = self.hub.acquire_lease(&self.config.gateway_binding)
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
