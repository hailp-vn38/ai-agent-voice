//! One bounded VAD inference through the application-owned VAD worker runtime.

use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};

use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::{
    audio::PcmF32Mono,
    providers::VadProbability,
    services::provider_diagnostic::{
        ProviderDiagnosticOperation, ProviderDiagnosticOperationError,
    },
};

use super::super::WorkerIdentity;
use super::{VadCaptureCycleId, VadCommand, VadWorkerEvent, VadWorkerLease, VadWorkerRuntime};

static NEXT_DIAGNOSTIC: AtomicU64 = AtomicU64::new(1);

pub struct VadDiagnosticOperation {
    runtime: Arc<VadWorkerRuntime>,
    session: Option<String>,
    receiver: Option<mpsc::Receiver<VadWorkerEvent>>,
    lease: Option<VadWorkerLease>,
    terminal: bool,
}

impl VadDiagnosticOperation {
    pub(super) fn new(runtime: Arc<VadWorkerRuntime>) -> Self {
        Self {
            runtime,
            session: None,
            receiver: None,
            lease: None,
            terminal: false,
        }
    }

    fn unregister(&mut self) {
        if let Some(session) = self.session.take() {
            self.runtime.unregister_session(&session);
        }
    }

    fn close_exact(&self) {
        if let Some(lease) = self.lease {
            let _ = self.runtime.send(lease, VadCommand::Close);
        }
    }

    async fn next_event(&mut self) -> Option<VadWorkerEvent> {
        self.receiver.as_mut()?.recv().await
    }

    fn is_terminal(event: &VadWorkerEvent) -> bool {
        matches!(
            event,
            VadWorkerEvent::Closed { .. } | VadWorkerEvent::Failed { .. }
        )
    }
}

#[async_trait::async_trait]
impl ProviderDiagnosticOperation for VadDiagnosticOperation {
    type Output = VadProbability;

    async fn execute(
        &mut self,
        cancellation: CancellationToken,
    ) -> Result<Self::Output, ProviderDiagnosticOperationError> {
        let session = format!(
            "admin-vad-diagnostic-{}",
            NEXT_DIAGNOSTIC.fetch_add(1, Ordering::Relaxed)
        );
        let identity = WorkerIdentity::new(session.clone(), 0, 0);
        self.receiver = Some(self.runtime.register_session(&session));
        self.session = Some(session);
        let lease = match self.runtime.open_diagnostic(identity) {
            Ok(lease) => lease,
            Err(_) => {
                self.unregister();
                return Err(ProviderDiagnosticOperationError::Unavailable);
            }
        };
        self.lease = Some(lease);
        if self
            .runtime
            .send(
                lease,
                VadCommand::Push {
                    cycle: VadCaptureCycleId::new(0),
                    // The worker ingress stays on the 960-sample transport frame; its rechunker
                    // emits exactly one 512-sample VAD inference frame for this diagnostic.
                    pcm: PcmF32Mono::new(vec![0.0; 960], 16_000),
                },
            )
            .is_err()
        {
            self.close_exact();
            return Err(ProviderDiagnosticOperationError::Unavailable);
        }
        let probability = loop {
            let event = tokio::select! {
                _ = cancellation.cancelled() => return Err(ProviderDiagnosticOperationError::Failed),
                event = self.next_event() => event,
            };
            match event {
                Some(VadWorkerEvent::Opened { .. }) => continue,
                Some(VadWorkerEvent::Probability { probability, .. })
                    if probability.start_sample == 0
                        && probability.end_sample == 512
                        && probability.probability.is_finite()
                        && (0.0..=1.0).contains(&probability.probability) =>
                {
                    break probability;
                }
                Some(event) => {
                    self.terminal = Self::is_terminal(&event);
                    if self.terminal {
                        self.unregister();
                    }
                    return Err(ProviderDiagnosticOperationError::Failed);
                }
                None => return Err(ProviderDiagnosticOperationError::Failed),
            }
        };
        self.close_exact();
        match self.next_event().await {
            Some(VadWorkerEvent::Closed { .. }) => {
                self.terminal = true;
                self.unregister();
                Ok(probability)
            }
            Some(event) => {
                self.terminal = Self::is_terminal(&event);
                if self.terminal {
                    self.unregister();
                }
                Err(ProviderDiagnosticOperationError::Failed)
            }
            None => Err(ProviderDiagnosticOperationError::Failed),
        }
    }

    fn cancel_exact(&mut self) {
        self.close_exact();
    }

    async fn await_terminal_acknowledgement(&mut self) -> bool {
        if self.terminal {
            return true;
        }
        loop {
            match self.next_event().await {
                Some(event) if Self::is_terminal(&event) => {
                    self.terminal = true;
                    self.unregister();
                    return true;
                }
                Some(VadWorkerEvent::Opened { .. } | VadWorkerEvent::Probability { .. }) => {
                    continue;
                }
                None => return false,
                Some(_) => return false,
            }
        }
    }

    fn quarantine_exact(&mut self) {
        if let Some(lease) = self.lease {
            self.runtime.quarantine_diagnostic(lease);
        }
        self.unregister();
    }
}
