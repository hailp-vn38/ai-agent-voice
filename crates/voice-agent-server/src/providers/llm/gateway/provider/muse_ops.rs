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
        pool::{SharedMusePool, UpstreamFailureKind, job_entry},
        workflow::{
            ChatSignal, MediaBatch, MediaKind, WorkflowFailure, run_chat_turn_once,
            run_media_turn_once,
        },
    },
};
use super::{LlmGatewayProvider, MAX_POOL_FAILOVER_ATTEMPTS, record_wire_detail};

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
        WorkflowFailure::Rejected(_) | WorkflowFailure::Timeout(_) => {}
    }
    failure.into_llm_error()
}

impl LlmGatewayProvider {
    pub(super) fn muse_pool_for_lease(
        &self,
        _cfg: &LlmGatewayConfig,
        lease: &GatewayLease,
    ) -> Arc<SharedMusePool> {
        SharedMusePool::for_gateway_lease(
            &self.http,
            &lease.account_key,
            lease.access_token.expose(),
            self.muse_transport.clone(),
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
        let timeout = if effective_cfg.timeout_profile.is_some()
            || effective_cfg.max_total_ms.is_some()
            || effective_cfg.timeout_ms >= 60_000
        {
            effective_cfg.resolved_limits().max_total + Duration::from_secs(30)
        } else {
            Duration::from_millis(effective_cfg.timeout_ms.max(1_000))
        };
        let req_clone = request.clone();
        let hub = Arc::clone(&self.hub);
        let http = self.http.clone();
        let cfg_clone = effective_cfg.clone();
        let muse_transport = self.muse_transport.clone();
        let err_slot = Arc::clone(&self.last_error_detail);
        let (tx, mut rx) = mpsc::unbounded_channel::<Result<LlmEvent, LlmError>>();

        let join_handle = tokio::spawn(async move {
            let mut current_lease = lease;
            for attempt in 0..max_attempts {
                let mut refreshed_in_attempt = false;
                loop {
                    let pool = SharedMusePool::for_gateway_lease(
                        &http,
                        &current_lease.account_key,
                        current_lease.access_token.expose(),
                        muse_transport.clone(),
                    );
                    let emitted_delta = Arc::new(AtomicBool::new(false));
                    let emitted_flag = Arc::clone(&emitted_delta);
                    let tx_delta = tx.clone();
                    let mut activity = super::ActivityThrottle::new();
                    let res = run_chat_turn_once(&pool, &req_clone, &[], timeout, move |signal| {
                        match signal {
                            ChatSignal::Delta(delta) => {
                                emitted_flag.store(true, Ordering::Relaxed);
                                let _ = tx_delta.send(Ok(LlmEvent::TextDelta(delta)));
                            }
                            ChatSignal::Activity => {
                                activity.maybe_emit(&tx_delta);
                            }
                        }
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
                            if !refreshed_in_attempt
                                && !emitted_delta.load(Ordering::Relaxed)
                                && matches!(
                                    failure,
                                    WorkflowFailure::Upstream(UpstreamFailureKind::AuthInvalid, _)
                                )
                                && let Some(refreshed_lease) = hub
                                    .try_refresh_on_unauthorized(
                                        &current_lease.account_key,
                                        current_lease.access_token.expose(),
                                    )
                                    .await
                            {
                                refreshed_in_attempt = true;
                                current_lease = refreshed_lease;
                                continue;
                            }
                            let failure_summary = format!(
                                "Muse workflow failure on account '{}': {:?}",
                                current_lease.account_key, failure
                            );
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
                                break;
                            }
                            record_wire_detail(&err_slot, failure_summary);
                            let _ = tx.send(Err(err));
                            return;
                        }
                    }
                }
            }
        });
        let abort_guard = tokio_util::task::AbortOnDropHandle::new(join_handle);

        let stream = futures_util::stream::poll_fn(move |cx| {
            let _keep_alive = &abort_guard;
            rx.poll_recv(cx)
        });
        Box::pin(stream)
    }

    pub(super) async fn execute_muse_with_auth_refresh_and_failover<T, F, Fut>(
        &self,
        initial_lease: GatewayLease,
        is_pool: bool,
        max_attempts: usize,
        mut op: F,
    ) -> Result<T, LlmError>
    where
        F: FnMut(Arc<SharedMusePool>) -> Fut,
        Fut: std::future::Future<Output = Result<T, WorkflowFailure>>,
    {
        let mut current_lease = initial_lease;
        for attempt in 0..max_attempts {
            let mut refreshed_in_attempt = false;
            loop {
                let pool = self.muse_pool_for_lease(&self.config, &current_lease);
                match op(pool).await {
                    Ok(output) => {
                        self.hub.report_outcome(
                            &current_lease.account_key,
                            GatewayCallOutcome::Success,
                        );
                        return Ok(output);
                    }
                    Err(failure) => {
                        if !refreshed_in_attempt
                            && matches!(
                                failure,
                                WorkflowFailure::Upstream(UpstreamFailureKind::AuthInvalid, _)
                            )
                            && let Some(refreshed_lease) = self
                                .hub
                                .try_refresh_on_unauthorized(
                                    &current_lease.account_key,
                                    current_lease.access_token.expose(),
                                )
                                .await
                        {
                            refreshed_in_attempt = true;
                            current_lease = refreshed_lease;
                            continue;
                        }
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
                            break;
                        }
                        return Err(err);
                    }
                }
            }
        }
        Err(LlmError::Failed)
    }

    pub(super) async fn run_muse_media(
        &self,
        kind: MediaKind,
        batches: &[MediaBatch],
        images: &[ReferenceImage],
        transparent_background: bool,
        continuation_key: Option<String>,
        required_tag: &'static str,
    ) -> Result<LlmOutput, LlmError> {
        let job = continuation_key.as_deref().map(job_entry);
        // Keep a job on the account that already holds its Muse session.
        let pinned = job
            .as_ref()
            .and_then(|entry| entry.account_hint())
            .and_then(|account_key| {
                self.hub
                    .acquire_lease(&GatewayBinding::Account(account_key))
                    .ok()
            })
            .filter(|lease| lease.vendor == GatewayVendor::Muse);
        let lease = match pinned {
            Some(lease) => lease,
            None => self
                .hub
                .acquire_lease(&self.config.gateway_binding)
                .map_err(|_| LlmError::Rejected)?,
        };
        if lease.vendor != GatewayVendor::Muse {
            return Err(LlmError::UnsupportedCapability(required_tag));
        }
        let is_pool = matches!(self.config.gateway_binding, GatewayBinding::Pool(_));
        let max_attempts = if is_pool {
            MAX_POOL_FAILOVER_ATTEMPTS
        } else {
            1
        };
        // 15 minutes for the first batch plus ~13 for each further batch of a long video.
        let extra_batches = batches.len().saturating_sub(1) as u64;
        let min_timeout_ms = match kind {
            MediaKind::Video => 900_000 + 800_000 * extra_batches,
            MediaKind::Image => 180_000,
        };
        let timeout = Duration::from_millis(self.config.timeout_ms.max(min_timeout_ms));
        self.execute_muse_with_auth_refresh_and_failover(lease, is_pool, max_attempts, |pool| {
            let job = job.clone();
            async move {
                run_media_turn_once(
                    &pool,
                    kind,
                    batches,
                    images,
                    transparent_background,
                    job,
                    timeout,
                )
                .await
            }
        })
        .await
    }
}
