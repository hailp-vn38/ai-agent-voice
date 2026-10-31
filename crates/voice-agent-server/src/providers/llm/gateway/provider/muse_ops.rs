use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use tokio::sync::mpsc;

use crate::{
    providers::llm::{
        DurableMediaProvider, LlmCapabilityTag, LlmError, LlmEvent, LlmEventStream, LlmOperation,
        LlmOutput, LlmProvider, LlmRequest, OperationJournal, OperationState, ReferenceImage,
    },
    services::llm_gateway::{
        GatewayBinding, GatewayCallOutcome, GatewayError, GatewayLease, GatewayVendor,
        LlmGatewayHub,
    },
};

use super::super::{
    config::LlmGatewayConfig,
    muse::{
        pool::{SharedMusePool, UpstreamFailureKind, VideoRequestSignature, job_entry},
        workflow::{
            ChatSignal, MediaBatch, MediaKind, WorkflowFailure, count_remote_user_turns,
            format_video_batches, resume_media_turn_once, run_chat_turn_once, run_media_turn_once,
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
        started_running: &AtomicBool,
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
                        let already_running = started_running.load(Ordering::Relaxed);
                        if !refreshed_in_attempt
                            && !already_running
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
                        let failure_summary = format!(
                            "Muse workflow failure on account '{}': {:?}",
                            current_lease.account_key, failure
                        );
                        let can_failover = is_pool
                            && !already_running
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
                        record_wire_detail(&self.last_error_detail, failure_summary);
                        return Err(err);
                    }
                }
            }
        }
        Err(LlmError::Failed)
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) async fn run_muse_media(
        &self,
        kind: MediaKind,
        batches: &[MediaBatch],
        images: &[ReferenceImage],
        transparent_background: bool,
        continuation_key: Option<String>,
        request_sig: Option<VideoRequestSignature>,
        existing_op: Option<&orchestrator_agent_rs::ToolOperation>,
        journal: Option<&dyn OperationJournal>,
        required_tag: &'static str,
    ) -> Result<LlmOutput, LlmError> {
        let job = continuation_key.as_deref().map(job_entry);
        // Keep a job on the account that already holds its Muse session. If that account is
        // temporarily unavailable, surface a transient error rather than falling back to the pool
        // and orphaning the remote session.
        let pinned_account = existing_op
            .and_then(|op| op.account_key.clone())
            .or_else(|| job.as_ref().and_then(|entry| entry.account_hint()));
        let (lease, is_pool) = match pinned_account {
            Some(account_key) => {
                let lease = self
                    .hub
                    .acquire_lease(&GatewayBinding::Account(account_key))
                    .map_err(|err| match err {
                        GatewayError::AccountReauthRequired => LlmError::Unauthorized,
                        _ => LlmError::Transient,
                    })?;
                (lease, false)
            }
            None => {
                let lease = self
                    .hub
                    .acquire_lease(&self.config.gateway_binding)
                    .map_err(|_| LlmError::Rejected)?;
                let is_pool = matches!(self.config.gateway_binding, GatewayBinding::Pool(_));
                (lease, is_pool)
            }
        };
        if lease.vendor != GatewayVendor::Muse {
            return Err(LlmError::UnsupportedCapability(required_tag));
        }
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
        let deadline = tokio::time::Instant::now() + timeout;
        let started_running = Arc::new(AtomicBool::new(false));
        let started_flag = Arc::clone(&started_running);
        self.execute_muse_with_auth_refresh_and_failover(
            lease,
            is_pool,
            max_attempts,
            &started_running,
            |pool| {
                let job = job.clone();
                let request_sig = request_sig.clone();
                let started_flag = Arc::clone(&started_flag);
                async move {
                    run_media_turn_once(
                        &pool,
                        kind,
                        batches,
                        images,
                        transparent_background,
                        job,
                        request_sig,
                        deadline,
                        &started_flag,
                        existing_op,
                        journal,
                    )
                    .await
                }
            },
        )
        .await
    }
}

#[async_trait::async_trait]
impl DurableMediaProvider for LlmGatewayProvider {
    async fn submit(
        &self,
        op: &LlmOperation,
        existing: Option<&orchestrator_agent_rs::ToolOperation>,
        io: &dyn OperationJournal,
    ) -> Result<LlmOutput, LlmError> {
        match op {
            LlmOperation::VideoGeneration {
                prompt,
                width,
                height,
                duration_secs,
                first_frame,
                continuation_key,
            } => {
                self.ensure_tag(LlmCapabilityTag::Video)?;
                let Some(batches) =
                    format_video_batches(prompt, *width, *height, *duration_secs)
                else {
                    return Err(LlmError::Rejected);
                };
                let images = first_frame.clone().into_iter().collect::<Vec<_>>();
                let request_sig = Some(VideoRequestSignature {
                    prompt: prompt.clone(),
                    duration_secs: *duration_secs,
                });
                self.run_muse_media(
                    MediaKind::Video,
                    &batches,
                    &images,
                    false,
                    continuation_key.clone(),
                    request_sig,
                    existing,
                    Some(io),
                    LlmCapabilityTag::Video.as_str(),
                )
                .await
            }
            _ => self.execute_operation(op.clone()).await,
        }
    }

    async fn reconcile_submitting(&self, op: &orchestrator_agent_rs::ToolOperation) -> Option<u32> {
        let account_key = op.account_key.as_deref()?;
        let remote_sid = op.remote_session_id.as_deref()?;
        let lease = match self
            .hub
            .acquire_lease(&GatewayBinding::Account(account_key.to_string()))
        {
            Ok(lease) => lease,
            Err(GatewayError::AccountCoolingDown) => {
                self.hub.acquire_account_for_polling(account_key).ok()?
            }
            Err(_) => return None,
        };
        if lease.vendor != GatewayVendor::Muse {
            return None;
        }
        let pool = self.muse_pool_for_lease(&self.config, &lease);
        let mut vm_lease = pool
            .acquire_existing_session(remote_sid.to_string())
            .await
            .ok()?;
        vm_lease.retain_session();
        let count = count_remote_user_turns(&mut vm_lease).await;
        vm_lease.finish_clean(None);
        count
    }

    async fn resume(
        &self,
        state: &OperationState,
        io: &dyn OperationJournal,
    ) -> Result<LlmOutput, LlmError> {
        self.ensure_tag(LlmCapabilityTag::Video)?;
        let Some(batches) = format_video_batches(
            &state.prompt,
            state.width,
            state.height,
            state.duration_secs,
        ) else {
            return Err(LlmError::Rejected);
        };
        let account_key = state
            .operation
            .account_key
            .clone()
            .or_else(|| {
                state
                    .continuation_key
                    .as_deref()
                    .and_then(|k| job_entry(k).account_hint())
            })
            .ok_or(LlmError::Transient)?;
        let lease = match self
            .hub
            .acquire_lease(&GatewayBinding::Account(account_key.clone()))
        {
            Ok(lease) => lease,
            Err(GatewayError::AccountCoolingDown) => self
                .hub
                .acquire_account_for_polling(&account_key)
                .map_err(|_| LlmError::Transient)?,
            Err(GatewayError::AccountReauthRequired) => return Err(LlmError::Unauthorized),
            Err(_) => return Err(LlmError::Transient),
        };
        if lease.vendor != GatewayVendor::Muse {
            return Err(LlmError::UnsupportedCapability(
                LlmCapabilityTag::Video.as_str(),
            ));
        }
        let extra_batches = batches.len().saturating_sub(1) as u64;
        let min_timeout_ms = 900_000 + 800_000 * extra_batches;
        let timeout = Duration::from_millis(self.config.timeout_ms.max(min_timeout_ms));
        let deadline = tokio::time::Instant::now() + timeout;
        let pool = self.muse_pool_for_lease(&self.config, &lease);
        let job = state.continuation_key.as_deref().map(job_entry);
        let request_sig = Some(VideoRequestSignature {
            prompt: state.prompt.clone(),
            duration_secs: state.duration_secs,
        });
        match resume_media_turn_once(
            &pool,
            &batches,
            job,
            request_sig,
            deadline,
            &state.operation,
            Some(io),
        )
        .await
        {
            Ok(output) => {
                self.hub
                    .report_outcome(&lease.account_key, GatewayCallOutcome::Success);
                Ok(output)
            }
            Err(failure) => {
                let failure_summary = format!(
                    "Muse workflow resume failure on account '{}': {:?}",
                    lease.account_key, failure
                );
                let err = report_muse_failure_to_hub(&self.hub, &lease.account_key, failure);
                record_wire_detail(&self.last_error_detail, failure_summary);
                Err(err)
            }
        }
    }
}
