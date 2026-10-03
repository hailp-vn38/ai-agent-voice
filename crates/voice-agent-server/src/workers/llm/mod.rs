use std::{
    collections::HashMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use futures_util::StreamExt;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::{
    providers::{
        LlmEvent, LlmProvider,
        llm::{LlmRequest, ToolCall},
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
}

impl LlmRuntime {
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
        cancellations.is_empty() && self.admission.active_work() == 0
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
        tokio::spawn(async move {
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
                .unwrap_or(LlmRuntimeEvent::Failed {
                    identity: identity.clone(),
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
        });
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
        let mut stream = self
            .provider
            .stream(self.request.clone())
            .await
            .map_err(|_| ProviderDiagnosticOperationError::Failed)?;
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
                    Some(Err(_)) => return Err(ProviderDiagnosticOperationError::Failed),
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
        false
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
    let Ok(mut stream) = provider.stream(request).await else {
        return LlmRuntimeEvent::Failed { identity };
    };
    loop {
        tokio::select! {
            _ = cancellation.cancelled() => return LlmRuntimeEvent::Cancelled { identity },
            item = stream.next() => match item {
                Some(Ok(LlmEvent::TextDelta(text))) => {
                    if route.send(LlmRuntimeEvent::TextDelta { identity: identity.clone(), text }).await.is_err() {
                        return LlmRuntimeEvent::Cancelled { identity };
                    }
                }
                Some(Ok(LlmEvent::ToolCall(call))) => {
                    if route.send(LlmRuntimeEvent::ToolCall { identity: identity.clone(), call }).await.is_err() {
                        return LlmRuntimeEvent::Cancelled { identity };
                    }
                }
                Some(Ok(LlmEvent::Finished)) | None => return LlmRuntimeEvent::Finished { identity },
                Some(Err(_)) => return LlmRuntimeEvent::Failed { identity },
            },
        }
    }
}

#[cfg(test)]
mod tests;
