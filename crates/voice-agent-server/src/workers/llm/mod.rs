use std::{
    collections::HashMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

use futures_util::StreamExt;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use tracing::Instrument;

use crate::{
    providers::{
        LlmCapabilityTag, LlmError, LlmEvent, LlmOperation, LlmProvider,
        llm::{ChatMessage, LlmRequest, ToolCall},
    },
    services::provider_diagnostic::{
        ProviderDiagnosticOperation, ProviderDiagnosticOperationError,
    },
    workers::{
        ProviderAdmissionError, ProviderCapacityPermit, ProviderRuntimeAdmission,
        ProviderWorkloadClass, WorkerIdentity,
    },
};

/// Why an LLM operation could not be accepted without exposing provider details.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum LlmStartError {
    #[error("LLM operations require a Tokio runtime")]
    NoTokioRuntime,
    #[error("LLM operation capacity is exhausted")]
    Capacity,
}

/// Coarse, redaction-safe reason a single-shot LLM operation could not complete.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum LlmInvokeError {
    #[error("LLM operation capacity is exhausted")]
    Overloaded,
    #[error("LLM operation timed out")]
    Timeout,
    #[error("LLM provider does not support capability tag `{0}`")]
    UnsupportedCapability(&'static str),
    #[error("LLM provider rejected the request")]
    Rejected,
    #[error("LLM provider failed")]
    Failed,
}

/// Coarse, redaction-safe reason an LLM operation could not produce a response.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LlmFailureReason {
    OpenStream,
    Stream,
    TimedOut,
}

/// Identity-tagged LLM outcomes consumed only by the owning Voice Session.
#[derive(Clone, Debug, PartialEq)]
pub enum LlmRuntimeEvent {
    TextDelta {
        identity: WorkerIdentity,
        text: String,
    },
    ToolCall {
        identity: WorkerIdentity,
        call: ToolCall,
    },
    Finished {
        identity: WorkerIdentity,
    },
    Failed {
        identity: WorkerIdentity,
        reason: LlmFailureReason,
    },
    Cancelled {
        identity: WorkerIdentity,
    },
}

/// Application-owned bounded runtime for remote LLM operations.
#[derive(Clone)]
pub struct LlmRuntime {
    closed: Arc<AtomicBool>,
    provider: Arc<dyn LlmProvider>,
    admission: ProviderRuntimeAdmission,
    timeout: Duration,
    routes: Arc<Mutex<HashMap<String, mpsc::Sender<LlmRuntimeEvent>>>>,
    cancellations: Arc<Mutex<HashMap<WorkerIdentity, CancellationToken>>>,
}

/// One bounded, tool-free LLM request that is independent of a Voice Session.
pub struct LlmDiagnosticOperation {
    provider: Arc<dyn LlmProvider>,
    request: LlmRequest,
    max_text_bytes: usize,
    cancellation: Option<CancellationToken>,
    terminal_acknowledged: bool,
    media: Arc<Mutex<Option<crate::providers::llm::LlmDiagnosticMedia>>>,
}

impl LlmDiagnosticOperation {
    pub fn media_slot(&self) -> Arc<Mutex<Option<crate::providers::llm::LlmDiagnosticMedia>>> {
        Arc::clone(&self.media)
    }
}

impl LlmRuntime {
    pub fn provider(&self) -> Arc<dyn LlmProvider> {
        Arc::clone(&self.provider)
    }

    pub fn supports_tools(&self) -> bool {
        self.provider.supports_tools()
    }
    /// Physical work remains occupied until terminal acknowledgement, including quarantine.
    pub(crate) fn pilot_work_pending(&self) -> bool {
        self.admission.view_usage() != 0
    }

    pub(crate) async fn wait_pilot_work_idle(&self) {
        self.admission.wait_until_idle().await;
    }

    pub fn new(provider: Arc<dyn LlmProvider>, capacity: usize, timeout: Duration) -> Self {
        Self::new_with_voice_reservation(provider, capacity, 1, timeout)
    }

    pub fn new_with_voice_reservation(
        provider: Arc<dyn LlmProvider>,
        capacity: usize,
        voice_reserved_capacity: usize,
        timeout: Duration,
    ) -> Self {
        assert!(capacity > 0);
        assert!(voice_reserved_capacity > 0 && voice_reserved_capacity <= capacity);
        assert!(!timeout.is_zero());
        Self::new_with_admission(
            provider,
            ProviderRuntimeAdmission::new(capacity, voice_reserved_capacity),
            timeout,
        )
    }

    pub(crate) fn new_with_admission(
        provider: Arc<dyn LlmProvider>,
        admission: ProviderRuntimeAdmission,
        timeout: Duration,
    ) -> Self {
        Self {
            closed: Arc::new(AtomicBool::new(false)),
            provider,
            admission,
            timeout,
            routes: Arc::new(Mutex::new(HashMap::new())),
            cancellations: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub fn register_session(
        &self,
        session: &str,
        capacity: usize,
    ) -> mpsc::Receiver<LlmRuntimeEvent> {
        let (sender, receiver) = mpsc::channel(capacity);
        self.routes
            .lock()
            .expect("LLM routes lock")
            .insert(session.into(), sender);
        receiver
    }

    pub(crate) fn logical_view(&self, quota: ProviderRuntimeAdmission) -> Self {
        let mut view = self.clone();
        view.admission = quota.composed(&self.admission);
        view
    }
    pub fn shutdown_acknowledged(&self) -> bool {
        let cancellations = self.cancellations.lock().expect("LLM cancellations lock");
        self.closed.store(true, Ordering::Release);
        cancellations.is_empty() && self.admission.view_usage() == 0
    }

    pub fn unregister_session(&self, session: &str) {
        self.routes.lock().expect("LLM routes lock").remove(session);
    }

    /// Accepts an operation only while global capacity is available. A permit is held by the task
    /// until it reports a terminal outcome or cancellation has dropped the provider stream.
    pub fn start(
        &self,
        identity: WorkerIdentity,
        request: LlmRequest,
        cancellation: CancellationToken,
    ) -> Result<(), LlmStartError> {
        if tokio::runtime::Handle::try_current().is_err() {
            return Err(LlmStartError::NoTokioRuntime);
        }
        let permit = self
            .admission
            .try_admit(ProviderWorkloadClass::Voice)
            .map_err(|_| LlmStartError::Capacity)?;
        {
            let mut cancellations = self.cancellations.lock().expect("LLM cancellations lock");
            if self.closed.load(Ordering::Acquire) {
                return Err(LlmStartError::Capacity);
            }
            cancellations.insert(identity.clone(), cancellation.clone());
        }
        let provider = Arc::clone(&self.provider);
        let routes = Arc::clone(&self.routes);
        let cancellations = Arc::clone(&self.cancellations);
        let timeout = self.timeout;
        let turn_span = tracing::info_span!(
            "voice_turn",
            session_id = %identity.session(),
            operation_id = identity.stream(),
            generation = identity.generation(),
        );
        tokio::spawn(
            async move {
                let started_at = Instant::now();
                let route = routes
                    .lock()
                    .expect("LLM routes lock")
                    .get(identity.session())
                    .cloned();
                let terminal = if let Some(route) = &route {
                    tokio::time::timeout(
                        timeout,
                        run_operation(
                            provider,
                            identity.clone(),
                            request,
                            cancellation,
                            route.clone(),
                        ),
                    )
                    .await
                    .unwrap_or_else(|_| {
                        tracing::warn!(
                            subsystem = "voice.llm",
                            event = "llm_stream_failed",
                            outcome = "failure",
                            reason = ?LlmFailureReason::TimedOut,
                            duration_ms = started_at.elapsed().as_millis() as u64,
                            "LLM stream timed out"
                        );
                        LlmRuntimeEvent::Failed {
                            identity: identity.clone(),
                            reason: LlmFailureReason::TimedOut,
                        }
                    })
                } else {
                    LlmRuntimeEvent::Cancelled {
                        identity: identity.clone(),
                    }
                };
                cancellations
                    .lock()
                    .expect("LLM cancellations lock")
                    .remove(&identity);
                drop(permit);
                if let Some(route) = route {
                    // The operation is terminal before the actor receives this event, so a
                    // tool-result continuation can acquire capacity immediately.
                    let _ = route.send(terminal).await;
                }
            }
            .instrument(turn_span),
        );
        Ok(())
    }

    pub fn cancel(&self, identity: &WorkerIdentity) {
        if let Some(token) = self
            .cancellations
            .lock()
            .expect("LLM cancellations lock")
            .get(identity)
        {
            token.cancel();
        }
    }

    /// Executes a bounded single-shot operation under Voice workload capacity and the runtime's
    /// configured timeout.
    pub async fn invoke(
        &self,
        op: LlmOperation,
    ) -> Result<crate::providers::LlmOutput, LlmInvokeError> {
        self.provider
            .ensure_tag(op.required_tag())
            .map_err(|_| LlmInvokeError::UnsupportedCapability(op.required_tag().as_str()))?;
        let permit = {
            let _state = self.cancellations.lock().expect("LLM cancellations lock");
            if self.closed.load(Ordering::Acquire) {
                return Err(LlmInvokeError::Overloaded);
            }
            self.admission
                .try_admit(ProviderWorkloadClass::Voice)
                .map_err(|_| LlmInvokeError::Overloaded)?
        };
        let result = tokio::time::timeout(self.timeout, self.provider.invoke(op)).await;
        drop(permit);
        match result {
            Ok(Ok(output)) => Ok(output),
            Ok(Err(LlmError::UnsupportedCapability(tag))) => {
                Err(LlmInvokeError::UnsupportedCapability(tag))
            }
            Ok(Err(LlmError::Rejected)) => Err(LlmInvokeError::Rejected),
            Ok(Err(LlmError::Failed)) => Err(LlmInvokeError::Failed),
            Err(_) => Err(LlmInvokeError::Timeout),
        }
    }

    /// Acquires capacity for a bounded diagnostic. The returned permit must live until the
    /// provider operation has acknowledged terminal completion or has been quarantined.
    pub fn admit_diagnostic(&self) -> Result<ProviderCapacityPermit, ProviderAdmissionError> {
        let _state = self.cancellations.lock().expect("LLM cancellations lock");
        if self.closed.load(Ordering::Acquire) {
            return Err(ProviderAdmissionError::Capacity);
        }
        self.admission.try_admit(ProviderWorkloadClass::Diagnostic)
    }

    /// Builds an operation for the already-materialized provider. Capacity is admitted by
    /// `ProviderDiagnosticService`, so constructing this value neither starts a request nor
    /// changes the runtime.
    pub fn diagnostic(&self, request: LlmRequest, max_text_bytes: usize) -> LlmDiagnosticOperation {
        debug_assert!(request.tools.is_empty());
        assert!(max_text_bytes > 0);
        LlmDiagnosticOperation {
            provider: Arc::clone(&self.provider),
            request,
            max_text_bytes,
            cancellation: None,
            terminal_acknowledged: false,
            media: Arc::new(Mutex::new(None)),
        }
    }
}

#[async_trait::async_trait]
impl ProviderDiagnosticOperation for LlmDiagnosticOperation {
    type Output = String;

    async fn execute(
        &mut self,
        cancellation: CancellationToken,
    ) -> Result<Self::Output, ProviderDiagnosticOperationError> {
        self.cancellation = Some(cancellation.clone());
        if !self.provider.supports_tag(LlmCapabilityTag::Chat) {
            let tag = self
                .provider
                .capability_tags()
                .first()
                .copied()
                .unwrap_or(LlmCapabilityTag::Chat);
            let prompt = self
                .request
                .messages
                .iter()
                .find_map(|msg| match msg {
                    ChatMessage::User { content } => Some(content.clone()),
                    _ => None,
                })
                .unwrap_or_else(|| "diagnostic".to_owned());
            let operation = LlmOperation::diagnostic_for(tag, prompt);
            let output = tokio::select! {
                _ = cancellation.cancelled() => return Err(ProviderDiagnosticOperationError::Failed),
                res = self.provider.invoke(operation) => res.map_err(|error| {
                    self.terminal_acknowledged = matches!(
                        error,
                        LlmError::Rejected | LlmError::UnsupportedCapability(_)
                    );
                    ProviderDiagnosticOperationError::Failed
                })?,
            };
            self.terminal_acknowledged = true;
            let summary = output.diagnostic_summary();
            if summary.is_empty() || summary.len() > self.max_text_bytes {
                return Err(ProviderDiagnosticOperationError::InvalidResponse);
            }
            if let Ok(mut guard) = self.media.lock() {
                *guard = output.diagnostic_media();
            }
            return Ok(summary);
        }

        let mut stream = self
            .provider
            .stream(self.request.clone())
            .await
            .map_err(|error| {
                self.terminal_acknowledged = matches!(
                    error,
                    LlmError::Rejected | LlmError::UnsupportedCapability(_)
                );
                ProviderDiagnosticOperationError::Failed
            })?;
        let mut text = String::new();
        loop {
            tokio::select! {
                _ = cancellation.cancelled() => return Err(ProviderDiagnosticOperationError::Failed),
                item = stream.next() => match item {
                    Some(Ok(LlmEvent::TextDelta(delta))) => {
                        let Some(next_length) = text.len().checked_add(delta.len()) else {
                            return Err(ProviderDiagnosticOperationError::InvalidResponse);
                        };
                        if next_length > self.max_text_bytes {
                            return Err(ProviderDiagnosticOperationError::InvalidResponse);
                        }
                        text.push_str(&delta);
                    }
                    Some(Ok(LlmEvent::ToolCall(_))) => {
                        return Err(ProviderDiagnosticOperationError::InvalidResponse);
                    }
                    Some(Ok(LlmEvent::Finished)) | None => return Ok(text),
                    Some(Err(error)) => {
                        self.terminal_acknowledged = matches!(
                            error,
                            LlmError::Rejected | LlmError::UnsupportedCapability(_)
                        );
                        return Err(ProviderDiagnosticOperationError::Failed);
                    }
                },
            }
        }
    }

    fn cancel_exact(&mut self) {
        if let Some(cancellation) = &self.cancellation {
            cancellation.cancel();
        }
    }

    async fn await_terminal_acknowledgement(&mut self) -> bool {
        // `LlmProvider` has no provider-specific cancellation acknowledgement seam. Dropping a
        // Rust stream is not evidence that its remote operation has terminated, so timeout must
        // retain (quarantine) this exact capacity rather than release it optimistically.
        self.terminal_acknowledged
    }

    fn quarantine_exact(&mut self) {
        // There is no reusable native worker behind this direct provider stream. The service
        // retains the exact runtime capacity permit in its quarantine set.
    }
}

async fn run_operation(
    provider: Arc<dyn LlmProvider>,
    identity: WorkerIdentity,
    request: LlmRequest,
    cancellation: CancellationToken,
    route: mpsc::Sender<LlmRuntimeEvent>,
) -> LlmRuntimeEvent {
    let started_at = Instant::now();
    let Ok(mut stream) = provider.stream(request).await else {
        tracing::warn!(
            subsystem = "voice.llm",
            event = "llm_stream_failed",
            outcome = "failure",
            reason = ?LlmFailureReason::OpenStream,
            duration_ms = started_at.elapsed().as_millis() as u64,
            "LLM provider failed to open stream"
        );
        return LlmRuntimeEvent::Failed {
            identity,
            reason: LlmFailureReason::OpenStream,
        };
    };
    let mut ttft_ms: Option<u64> = None;
    let mut tool_calls: u64 = 0;
    loop {
        tokio::select! {
            _ = cancellation.cancelled() => return LlmRuntimeEvent::Cancelled { identity },
            item = stream.next() => match item {
                Some(Ok(LlmEvent::TextDelta(text))) => {
                    if ttft_ms.is_none() {
                        ttft_ms = Some(started_at.elapsed().as_millis() as u64);
                    }
                    if route.send(LlmRuntimeEvent::TextDelta { identity: identity.clone(), text }).await.is_err() {
                        return LlmRuntimeEvent::Cancelled { identity };
                    }
                }
                Some(Ok(LlmEvent::ToolCall(call))) => {
                    if ttft_ms.is_none() {
                        ttft_ms = Some(started_at.elapsed().as_millis() as u64);
                    }
                    tool_calls = tool_calls.saturating_add(1);
                    if route.send(LlmRuntimeEvent::ToolCall { identity: identity.clone(), call }).await.is_err() {
                        return LlmRuntimeEvent::Cancelled { identity };
                    }
                }
                Some(Ok(LlmEvent::Finished)) | None => {
                    let duration_ms = started_at.elapsed().as_millis() as u64;
                    tracing::info!(
                        subsystem = "voice.llm",
                        event = "llm_stream_completed",
                        outcome = "success",
                        duration_ms,
                        ttft_ms = ttft_ms.unwrap_or(duration_ms),
                        tool_calls,
                        "LLM stream completed"
                    );
                    return LlmRuntimeEvent::Finished { identity };
                }
                Some(Err(_)) => {
                    tracing::warn!(
                        subsystem = "voice.llm",
                        event = "llm_stream_failed",
                        outcome = "failure",
                        reason = ?LlmFailureReason::Stream,
                        duration_ms = started_at.elapsed().as_millis() as u64,
                        "LLM stream failed mid-turn"
                    );
                    return LlmRuntimeEvent::Failed {
                        identity,
                        reason: LlmFailureReason::Stream,
                    };
                }
            },
        }
    }
}

#[cfg(test)]
mod tests;
