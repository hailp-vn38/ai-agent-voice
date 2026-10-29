use std::{sync::Arc, time::Duration};

use futures_util::FutureExt;
use tokio_util::sync::CancellationToken;

use super::*;
use crate::providers::{
    DatabaseRuntimeStatus, DiagnosticRuntimeError, DiagnosticRuntimeKind,
};

impl ProviderDiagnosticService {
    /// Owns a diagnostic from execution through terminal acknowledgement. A timeout never drops
    /// runtime capacity early: it first cancels the exact operation, then waits the caller's
    /// worker-specific grace window, and only then quarantines that exact operation/slot.
    pub async fn execute<O>(
        &self,
        target: ProviderDiagnosticTarget,
        mut operation: O,
        cleanup_grace: Duration,
    ) -> Result<(O::Output, ProviderDiagnosticRuntimeMetadata), ProviderDiagnosticError>
    where
        O: ProviderDiagnosticOperation + 'static,
        O::Output: 'static,
    {
        let lease = self.begin(target)?;
        let cancellation = CancellationToken::new();
        let _cancel_on_drop = cancellation.clone().drop_guard();
        let execution_timeout = self.execution_timeout;
        // The bounded owner survives its HTTP waiter until terminal ACK or quarantine.
        let attempt = tokio::spawn(async move {
            if cancellation.is_cancelled() {
                lease.acknowledge_terminal();
                return Err(ProviderDiagnosticError::Timeout);
            }
            let outcome = tokio::select! {
                _ = cancellation.cancelled() => None,
                result = std::panic::AssertUnwindSafe(tokio::time::timeout(execution_timeout, operation.execute(cancellation.clone()))).catch_unwind() => Some(result.unwrap_or(Ok(Err(ProviderDiagnosticOperationError::Failed)))),
            };
            match outcome {
                Some(Ok(Ok(output))) => {
                    let metadata = lease.metadata.clone();
                    lease.acknowledge_terminal();
                    Ok((output, metadata))
                }
                Some(Ok(Err(error))) => {
                    // An operation-level error can be raised while its exact native worker is still
                    // producing output (for example after a diagnostic output cap). Treat it like a
                    // timeout: cancellation must be acknowledged before capacity is reusable.
                    operation.cancel_exact();
                    let acknowledged = tokio::time::timeout(
                        cleanup_grace,
                        operation.await_terminal_acknowledgement(),
                    )
                    .await
                    .unwrap_or(false);
                    if acknowledged {
                        lease.acknowledge_terminal();
                    } else {
                        operation.quarantine_exact();
                        lease.quarantine();
                    }
                    Err(map_operation_error(error))
                }
                None | Some(Err(_)) => {
                    operation.cancel_exact();
                    cancellation.cancel();
                    let acknowledged = tokio::time::timeout(
                        cleanup_grace,
                        operation.await_terminal_acknowledgement(),
                    )
                    .await
                    .unwrap_or(false);
                    if acknowledged {
                        lease.acknowledge_terminal();
                    } else {
                        operation.quarantine_exact();
                        lease.quarantine();
                    }
                    Err(ProviderDiagnosticError::Timeout)
                }
            }
        });
        attempt
            .await
            .unwrap_or(Err(ProviderDiagnosticError::Failed))
    }

    /// Starts only if the desired row says a runtime was loaded at process bootstrap. The caller
    /// receives stale-runtime metadata so a later HTTP response cannot imply desired DB edits
    /// were hot-loaded.
    pub fn begin(
        &self,
        target: ProviderDiagnosticTarget,
    ) -> Result<ProviderDiagnosticLease, ProviderDiagnosticError> {
        let kind = DiagnosticRuntimeKind::try_from(target.provider_type)
            .map_err(|_| ProviderDiagnosticError::TypeMismatch)?;
        let metadata = self.runtime_metadata(&target);
        if !matches!(metadata.tested_runtime, DatabaseRuntimeStatus::Loaded) {
            return Err(ProviderDiagnosticError::RuntimeNotLoaded);
        }
        let diagnostic = self
            .managed_admission
            .lock()
            .expect("diagnostic admission poisoned")
            .take()
            .map(Ok)
            .unwrap_or_else(|| self.limiter.try_acquire())?;
        let pipeline = self
            .pilot
            .as_ref()
            .map(|pilot| pilot.try_voice().ok_or(ProviderDiagnosticError::Busy))
            .transpose()?;
        let capacity = self
            .registry
            .admit_diagnostic(kind, &target.key)
            .map_err(map_runtime_error)?;
        Ok(ProviderDiagnosticLease {
            pipeline,
            _diagnostic: diagnostic,
            capacity: Some(capacity),
            resource_lease: self._managed_lease.clone(),
            quarantine: Arc::clone(&self.quarantined_capacity),
            metadata,
        })
    }

    fn runtime_metadata(
        &self,
        target: &ProviderDiagnosticTarget,
    ) -> ProviderDiagnosticRuntimeMetadata {
        if self._managed_lease.as_ref().is_some_and(|lease| {
            matches!(
                lease.version().identity,
                crate::services::provider_runtime::ProviderIdentity::Deployment { .. }
            )
        }) {
            return ProviderDiagnosticRuntimeMetadata {
                tested_provider_id: None,
                tested_revision: Some(1),
                tested_runtime: DatabaseRuntimeStatus::Loaded,
                runtime_matches_desired: true,
                requires_restart: false,
            };
        }
        if self._managed_lease.as_ref().is_some_and(|lease| {
            matches!(
                lease.version().identity,
                crate::services::provider_runtime::ProviderIdentity::Draft(_)
            )
        }) {
            return ProviderDiagnosticRuntimeMetadata {
                tested_provider_id: None,
                tested_revision: None,
                tested_runtime: DatabaseRuntimeStatus::Loaded,
                runtime_matches_desired: false,
                requires_restart: false,
            };
        }
        let state = self
            .runtime_snapshot
            .as_ref()
            .map(|snapshot| snapshot.runtime_state(&target.key, target.desired_revision));
        let loaded = state
            .as_ref()
            .is_some_and(|state| matches!(state.status, DatabaseRuntimeStatus::Loaded));
        let matches_desired = loaded
            && state
                .as_ref()
                .is_some_and(|state| state.desired_revision == target.desired_revision);
        ProviderDiagnosticRuntimeMetadata {
            tested_provider_id: state
                .as_ref()
                .filter(|state| state.provider_id > 0)
                .map(|state| state.provider_id),
            tested_revision: state
                .as_ref()
                .filter(|_| loaded)
                .map(|state| state.desired_revision),
            tested_runtime: if loaded {
                DatabaseRuntimeStatus::Loaded
            } else {
                DatabaseRuntimeStatus::NotLoaded
            },
            runtime_matches_desired: matches_desired,
            requires_restart: if self.managed_target.is_some() {
                false
            } else {
                !matches_desired
            },
        }
    }
}

impl ProviderDiagnosticLease {
    /// Terminal acknowledgement releases both permits exactly once when this guard drops.
    pub fn acknowledge_terminal(self) {}

    /// A bounded cleanup window elapsed without acknowledgement. Runtime capacity is retained in
    /// the quarantine set; only the global diagnostic permit is released when this guard drops.
    pub fn quarantine(mut self) {
        if let Some(capacity) = self.capacity.take() {
            self.quarantine
                .lock()
                .expect("provider diagnostic quarantine poisoned")
                .push(QuarantinedDiagnosticCapacity {
                    _capacity: capacity,
                    _pipeline: self.pipeline.take(),
                    _resource: self.resource_lease.take(),
                });
        }
    }
}

fn map_runtime_error(error: DiagnosticRuntimeError) -> ProviderDiagnosticError {
    match error {
        DiagnosticRuntimeError::NotLoaded => ProviderDiagnosticError::RuntimeNotLoaded,
        DiagnosticRuntimeError::Capacity => ProviderDiagnosticError::Busy,
    }
}

fn map_operation_error(error: ProviderDiagnosticOperationError) -> ProviderDiagnosticError {
    match error {
        ProviderDiagnosticOperationError::Unavailable => ProviderDiagnosticError::Unavailable,
        ProviderDiagnosticOperationError::InvalidResponse => {
            ProviderDiagnosticError::InvalidResponse
        }
        ProviderDiagnosticOperationError::Failed => ProviderDiagnosticError::Failed,
    }
}
