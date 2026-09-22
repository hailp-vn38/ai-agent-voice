use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::Duration,
};

use futures_util::StreamExt;
use tokio::sync::{Semaphore, mpsc};
use tokio_util::sync::CancellationToken;

use crate::{
    providers::{LlmEvent, LlmProvider},
    workers::WorkerIdentity,
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
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LlmRuntimeEvent {
    TextDelta {
        identity: WorkerIdentity,
        text: String,
    },
    UnexpectedToolCall {
        identity: WorkerIdentity,
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
pub struct LlmRuntime {
    provider: Arc<dyn LlmProvider>,
    permits: Arc<Semaphore>,
    timeout: Duration,
    routes: Arc<Mutex<HashMap<String, mpsc::Sender<LlmRuntimeEvent>>>>,
    cancellations: Arc<Mutex<HashMap<WorkerIdentity, CancellationToken>>>,
}

impl LlmRuntime {
    pub fn new(provider: Arc<dyn LlmProvider>, capacity: usize, timeout: Duration) -> Self {
        assert!(capacity > 0);
        assert!(!timeout.is_zero());
        Self {
            provider,
            permits: Arc::new(Semaphore::new(capacity)),
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

    pub fn unregister_session(&self, session: &str) {
        self.routes.lock().expect("LLM routes lock").remove(session);
    }

    /// Accepts an operation only while global capacity is available. A permit is held by the task
    /// until it reports a terminal outcome or cancellation has dropped the provider stream.
    pub fn start(&self, identity: WorkerIdentity, prompt: String) -> Result<(), LlmStartError> {
        if tokio::runtime::Handle::try_current().is_err() {
            return Err(LlmStartError::NoTokioRuntime);
        }
        let permit = self
            .permits
            .clone()
            .try_acquire_owned()
            .map_err(|_| LlmStartError::Capacity)?;
        let cancellation = CancellationToken::new();
        self.cancellations
            .lock()
            .expect("LLM cancellations lock")
            .insert(identity.clone(), cancellation.clone());
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
            if let Some(route) = route {
                let terminal = tokio::time::timeout(
                    timeout,
                    run_operation(
                        provider,
                        identity.clone(),
                        prompt,
                        cancellation,
                        route.clone(),
                    ),
                )
                .await
                .unwrap_or(LlmRuntimeEvent::Failed {
                    identity: identity.clone(),
                });
                // Awaiting preserves a terminal event when the bounded route is temporarily full.
                let _ = route.send(terminal).await;
            }
            cancellations
                .lock()
                .expect("LLM cancellations lock")
                .remove(&identity);
            drop(permit);
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
}

async fn run_operation(
    provider: Arc<dyn LlmProvider>,
    identity: WorkerIdentity,
    prompt: String,
    cancellation: CancellationToken,
    route: mpsc::Sender<LlmRuntimeEvent>,
) -> LlmRuntimeEvent {
    let Ok(mut stream) = provider.stream(prompt).await else {
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
                Some(Ok(LlmEvent::UnexpectedToolCall)) => return LlmRuntimeEvent::UnexpectedToolCall { identity },
                Some(Ok(LlmEvent::Finished)) | None => return LlmRuntimeEvent::Finished { identity },
                Some(Err(_)) => return LlmRuntimeEvent::Failed { identity },
            },
        }
    }
}
