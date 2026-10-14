//! Exact-version Background LLM admission, sharing the provider's Voice reservation.
use super::{JobError, ProviderPin, model::LlmRunContext};
use crate::{
    config::AppConfig,
    database::Database,
    providers::{config_catalog::ProviderOrigin, llm::LlmRequest},
    services::provider_runtime::ProviderRuntimeManager,
};
use serde_json::json;
use std::{sync::Arc, time::Duration};
use tokio_util::sync::CancellationToken;

#[async_trait::async_trait]
pub(crate) trait JobModels: Send + Sync {
    async fn ready(&self, _: &ProviderPin, _: i64) -> Result<(), JobError> {
        Ok(())
    }
    async fn complete(
        &self,
        job_id: &str,
        pin: &ProviderPin,
        request: LlmRequest,
        deadline_at: i64,
        cancellation: CancellationToken,
    ) -> Result<String, JobError>;

    /// Native output schema is used by Responses providers. Chat-only test adapters
    /// and providers retain the existing Rust validation semantics.
    async fn complete_structured(
        &self,
        job_id: &str,
        pin: &ProviderPin,
        request: LlmRequest,
        _schema: serde_json::Value,
        deadline_at: i64,
        cancellation: CancellationToken,
    ) -> Result<String, JobError> {
        self.complete(job_id, pin, request, deadline_at, cancellation)
            .await
    }
    async fn turn_traced(
        &self,
        run: &LlmRunContext,
        pin: &ProviderPin,
        request: LlmRequest,
        mode: crate::providers::llm::BackgroundMode,
        deadline_at: i64,
        cancellation: CancellationToken,
    ) -> Result<crate::providers::llm::BackgroundTurnResult, JobError> {
        // Test doubles for existing Root repair tests may implement only the
        // structured completion seam. Production ManagedModels overrides this.
        match mode {
            crate::providers::llm::BackgroundMode::Structured { schema, .. } => {
                let text = self
                    .complete_structured_traced(
                        run,
                        pin,
                        request,
                        schema,
                        deadline_at,
                        cancellation,
                    )
                    .await?;
                Ok(crate::providers::llm::BackgroundTurnResult {
                    turn: crate::providers::llm::BackgroundTurn::Text(text),
                    usage: None,
                })
            }
            crate::providers::llm::BackgroundMode::Tools
            | crate::providers::llm::BackgroundMode::ToolsAuto
            | crate::providers::llm::BackgroundMode::ToolsParallel => {
                Err(JobError::CapabilityUnavailable)
            }
        }
    }
    async fn complete_traced(
        &self,
        run: &LlmRunContext,
        pin: &ProviderPin,
        request: LlmRequest,
        deadline_at: i64,
        cancellation: CancellationToken,
    ) -> Result<String, JobError> {
        self.complete(&run.job_id, pin, request, deadline_at, cancellation)
            .await
    }
    async fn complete_structured_traced(
        &self,
        run: &LlmRunContext,
        pin: &ProviderPin,
        request: LlmRequest,
        schema: serde_json::Value,
        deadline_at: i64,
        cancellation: CancellationToken,
    ) -> Result<String, JobError> {
        self.complete_structured(&run.job_id, pin, request, schema, deadline_at, cancellation)
            .await
    }
}

pub(crate) struct ManagedModels {
    pub database: Arc<Database>,
    pub config: Arc<AppConfig>,
    pub manager: Arc<ProviderRuntimeManager>,
    pub quarantine: std::sync::Mutex<
        Vec<(
            crate::services::provider_runtime::ResourceLease,
            crate::workers::ProviderCapacityPermit,
        )>,
    >,
}

struct CompletionRequest {
    request: LlmRequest,
    schema: Option<serde_json::Value>,
    mode: Option<crate::providers::llm::BackgroundMode>,
}
fn legacy_text(value: crate::providers::llm::BackgroundTurnResult) -> Result<String, JobError> {
    match value.turn {
        crate::providers::llm::BackgroundTurn::Text(text) => Ok(text),
        _ => Err(JobError::ModelFailed),
    }
}

pub(crate) fn remaining(deadline_at: i64) -> Result<Duration, JobError> {
    let deadline = std::time::UNIX_EPOCH
        .checked_add(Duration::from_secs(
            u64::try_from(deadline_at).map_err(|_| JobError::DeadlineExceeded)?,
        ))
        .ok_or(JobError::DeadlineExceeded)?;
    deadline
        .duration_since(std::time::SystemTime::now())
        .map_err(|_| JobError::DeadlineExceeded)
}

impl ManagedModels {
    async fn observe(
        &self,
        run: Option<&LlmRunContext>,
        event: &'static str,
        detail: serde_json::Value,
    ) {
        if let Some(run) = run
            && let Err(error) = self.database.record_llm_activity(run, event, detail).await
        {
            tracing::warn!(event="background_llm_activity_write_failed", job_id=%run.job_id, code=%error.code());
        }
    }

    async fn acquire(
        &self,
        previous: &ProviderPin,
        deadline_at: i64,
        cancellation: CancellationToken,
    ) -> Result<
        (
            crate::services::provider_runtime::ResourceLease,
            Arc<crate::workers::LlmRuntime>,
            String,
            Option<String>,
            &'static str,
        ),
        JobError,
    > {
        let resolved = self
            .database
            .background_resolve_provider(&self.config, previous)
            .await?;
        let deadline = tokio::time::Instant::now() + remaining(deadline_at)?;
        let provider_wait_started = std::time::Instant::now();
        let origin = resolved.origin;
        let key = resolved.key;
        let adapter = resolved.adapter;
        let model = resolved.model;
        let transport = resolved.transport;
        let snapshot = resolved.snapshot;
        let lease = tokio::select! {
            _ = cancellation.cancelled() => return Err(JobError::Cancelled),
            result = async {
                match origin {
                    ProviderOrigin::Config => self.manager.acquire_deployment_until(snapshot, deadline).await,
                    ProviderOrigin::Database => self.manager.acquire_until(snapshot, deadline).await,
                }
            } => result.map_err(|_| JobError::ProviderUnavailable)?,
        };
        tracing::info!(event="background_provider_acquired", provider=%previous.reference, provider_wait_ms=provider_wait_started.elapsed().as_millis() as u64);
        let catalog = lease.runtimes().ok_or(JobError::ProviderUnavailable)?;
        let runtime = catalog
            .llm
            .get(&key)
            .cloned()
            .ok_or(JobError::ProviderUnavailable)?;
        if !runtime.has_background_capacity() {
            return Err(JobError::CapacityUnavailable);
        }
        Ok((lease, runtime, adapter, model, transport))
    }

    async fn complete_inner(
        &self,
        job_id: &str,
        previous: &ProviderPin,
        completion: CompletionRequest,
        deadline_at: i64,
        cancellation: CancellationToken,
        run: Option<&LlmRunContext>,
    ) -> Result<crate::providers::llm::BackgroundTurnResult, JobError> {
        let CompletionRequest {
            request,
            schema,
            mode,
        } = completion;
        let bytes = request
            .messages
            .iter()
            .map(|message| match message {
                crate::providers::llm::ChatMessage::System { content }
                | crate::providers::llm::ChatMessage::User { content }
                | crate::providers::llm::ChatMessage::AssistantText { content }
                | crate::providers::llm::ChatMessage::ToolResult { content, .. } => content.len(),
                crate::providers::llm::ChatMessage::AssistantToolCalls { calls } => {
                    serde_json::to_string(
                        &calls
                            .iter()
                            .map(|call| (&call.id, &call.name, &call.arguments))
                            .collect::<Vec<_>>(),
                    )
                    .map_or(usize::MAX, |s| s.len())
                }
            })
            .fold(0usize, usize::saturating_add);
        if bytes > 65536
            || serde_json::to_string(
                &request
                    .tools
                    .iter()
                    .map(|tool| (&tool.name, &tool.description, &tool.parameters))
                    .collect::<Vec<_>>(),
            )
            .map_or(true, |s| s.len() > 16384)
            || (mode.is_none() && !request.tools.is_empty())
        {
            return Err(JobError::CapacityUnavailable);
        }
        let provider_wait_started = std::time::Instant::now();
        let (lease, runtime, adapter, model, transport) = self
            .acquire(previous, deadline_at, cancellation.clone())
            .await?;
        self.observe(
            run,
            "llm_provider_acquired",
            json!({
                "adapter": adapter, "model": model, "transport": transport,
                "provider_wait_ms": provider_wait_started.elapsed().as_millis() as u64,
            }),
        )
        .await;
        let deadline = tokio::time::Instant::now() + remaining(deadline_at)?;
        let capacity_wait_started = std::time::Instant::now();
        let permit = wait_background(&runtime, deadline, &cancellation).await?;
        let capacity_wait_ms = capacity_wait_started.elapsed().as_millis() as u64;
        tracing::info!(event="background_provider_permit", job_id, provider=%previous.reference, capacity_wait_ms);
        let publication = self.database.tool_security.publication.read().await;
        self.database
            .background_authorize_provider_call(&self.config, job_id, previous)
            .await?;
        self.observe(
            run,
            "llm_request_started",
            json!({
                "capacity_wait_ms":capacity_wait_ms,
                "message_count":request.messages.len(),
                "input_bytes":bytes
            }),
        )
        .await;
        // Release immediately before dispatch. No asynchronous DB work between
        // the last authorization check and the awaited provider invocation.
        drop(publication);
        let request_started = std::time::Instant::now();
        let (outcome, unknown) = tokio::select! {
            _=cancellation.cancelled()=>(Err(JobError::Cancelled),true),
            _=tokio::time::sleep_until(deadline)=>(Err(JobError::DeadlineExceeded),true),
            result=async {
                if let Some(mode) = mode {
                    runtime.background_turn(request,mode).await
                } else {
                    runtime.background_complete_with_usage(request,schema).await
                        .map(|v| crate::providers::llm::BackgroundTurnResult {
                            turn:crate::providers::llm::BackgroundTurn::Text(v.text),usage:v.usage
                        })
                }
            }=>match result {
                Ok(value)=>(Ok(value),false),
                Err(crate::providers::LlmError::Rejected)=>(Err(JobError::ModelFailed),false),
                Err(_)=>(Err(JobError::ModelFailed),true),
            },
        };
        if outcome.is_err() && unknown {
            self.quarantine
                .lock()
                .expect("Background LLM quarantine")
                .push((lease, permit));
        }
        match outcome {
            Ok(response) => {
                self.observe(run,"llm_response_received",json!({
                    "request_duration_ms":request_started.elapsed().as_millis() as u64,
                    "output_bytes":match &response.turn {
                        crate::providers::llm::BackgroundTurn::Text(s)=>s.len(),
                        crate::providers::llm::BackgroundTurn::ToolCalls(c)=>serde_json::to_string(
                            &c.iter().map(|call| (&call.id,&call.name,&call.arguments)).collect::<Vec<_>>()
                        ).map_or(0,|s|s.len()),
                    },
                    "input_tokens":response.usage.as_ref().and_then(|u| u.input_tokens),
                    "output_tokens":response.usage.as_ref().and_then(|u| u.output_tokens),
                    "reasoning_tokens":response.usage.as_ref().and_then(|u| u.reasoning_tokens),
                    "cached_input_tokens":response.usage.as_ref().and_then(|u| u.cached_input_tokens),
                })).await;
                Ok(response)
            }
            Err(error) => Err(error),
        }
    }
}

#[async_trait::async_trait]
impl JobModels for ManagedModels {
    async fn turn_traced(
        &self,
        run: &LlmRunContext,
        pin: &ProviderPin,
        request: LlmRequest,
        mode: crate::providers::llm::BackgroundMode,
        deadline_at: i64,
        cancellation: CancellationToken,
    ) -> Result<crate::providers::llm::BackgroundTurnResult, JobError> {
        self.complete_inner(
            &run.job_id,
            pin,
            CompletionRequest {
                request,
                schema: None,
                mode: Some(mode),
            },
            deadline_at,
            cancellation,
            Some(run),
        )
        .await
    }
    async fn ready(&self, pin: &ProviderPin, deadline_at: i64) -> Result<(), JobError> {
        self.acquire(pin, deadline_at, CancellationToken::new())
            .await
            .map(|_| ())
    }

    async fn complete(
        &self,
        job_id: &str,
        pin: &ProviderPin,
        request: LlmRequest,
        deadline_at: i64,
        cancellation: CancellationToken,
    ) -> Result<String, JobError> {
        self.complete_inner(
            job_id,
            pin,
            CompletionRequest {
                request,
                schema: None,
                mode: None,
            },
            deadline_at,
            cancellation,
            None,
        )
        .await
        .and_then(legacy_text)
    }
    async fn complete_structured(
        &self,
        job_id: &str,
        pin: &ProviderPin,
        request: LlmRequest,
        schema: serde_json::Value,
        deadline_at: i64,
        cancellation: CancellationToken,
    ) -> Result<String, JobError> {
        self.complete_inner(
            job_id,
            pin,
            CompletionRequest {
                request,
                schema: Some(schema),
                mode: None,
            },
            deadline_at,
            cancellation,
            None,
        )
        .await
        .and_then(legacy_text)
    }
    async fn complete_traced(
        &self,
        run: &LlmRunContext,
        pin: &ProviderPin,
        request: LlmRequest,
        deadline_at: i64,
        cancellation: CancellationToken,
    ) -> Result<String, JobError> {
        self.complete_inner(
            &run.job_id,
            pin,
            CompletionRequest {
                request,
                schema: None,
                mode: None,
            },
            deadline_at,
            cancellation,
            Some(run),
        )
        .await
        .and_then(legacy_text)
    }
    async fn complete_structured_traced(
        &self,
        run: &LlmRunContext,
        pin: &ProviderPin,
        request: LlmRequest,
        schema: serde_json::Value,
        deadline_at: i64,
        cancellation: CancellationToken,
    ) -> Result<String, JobError> {
        self.complete_inner(
            &run.job_id,
            pin,
            CompletionRequest {
                request,
                schema: Some(schema),
                mode: None,
            },
            deadline_at,
            cancellation,
            Some(run),
        )
        .await
        .and_then(legacy_text)
    }
}

/// Static zero capacity is an admission error; dynamic contention is bounded.
async fn wait_background(
    runtime: &crate::workers::LlmRuntime,
    deadline: tokio::time::Instant,
    cancellation: &CancellationToken,
) -> Result<crate::workers::ProviderCapacityPermit, JobError> {
    if !runtime.has_background_capacity() {
        return Err(JobError::CapacityUnavailable);
    }
    loop {
        if cancellation.is_cancelled() {
            return Err(JobError::Cancelled);
        }
        if tokio::time::Instant::now() >= deadline {
            return Err(JobError::DeadlineExceeded);
        }
        if let Ok(permit) = runtime.admit_background() {
            return Ok(permit);
        }
        tokio::select! {
            _ = cancellation.cancelled() => return Err(JobError::Cancelled),
            _ = tokio::time::sleep_until(deadline) => return Err(JobError::DeadlineExceeded),
            _ = tokio::time::sleep(Duration::from_millis(20)) => {},
        }
    }
}
#[cfg(test)]
mod capacity_tests {
    use super::*;
    #[tokio::test]
    async fn v3_background_static_zero_rejects_and_dynamic_wait_obeys_deadline_and_cancellation() {
        let provider = Arc::new(crate::providers::llm::UnavailableLlm);
        let zero = crate::workers::LlmRuntime::new(provider.clone(), 1, Duration::from_secs(1));
        let cancel = CancellationToken::new();
        let deadline = tokio::time::Instant::now() + Duration::from_secs(1);
        assert!(matches!(
            wait_background(&zero, deadline, &cancel).await,
            Err(JobError::CapacityUnavailable)
        ));
        let dynamic = crate::workers::LlmRuntime::new(provider, 2, Duration::from_secs(1));
        let held = dynamic.admit_background().unwrap();
        assert!(
            dynamic.has_background_capacity(),
            "contention is not static zero capacity"
        );
        assert!(matches!(
            wait_background(
                &dynamic,
                tokio::time::Instant::now() + Duration::from_millis(30),
                &cancel
            )
            .await,
            Err(JobError::DeadlineExceeded)
        ));
        cancel.cancel();
        assert!(matches!(
            wait_background(&dynamic, deadline, &cancel).await,
            Err(JobError::Cancelled)
        ));
        drop(held);
        assert!(
            wait_background(&dynamic, deadline, &CancellationToken::new())
                .await
                .is_ok()
        );
    }
}
