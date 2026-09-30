//! Admission and lifecycle ownership for bounded provider diagnostics.
//!
//! HTTP handlers deliberately do not own these permits: a request timeout only asks a worker to
//! stop. The lease remains alive until a terminal acknowledgement or an explicit quarantine.

use std::{
    sync::{Arc, Mutex},
    time::Duration,
};

use tokio::sync::{OwnedSemaphorePermit, Semaphore};
use tokio_util::sync::CancellationToken;

use crate::{
    audio::PcmF32Mono,
    database::Database,
    providers::{
        DatabaseRuntimeSnapshot, DatabaseRuntimeStatus, DiagnosticRuntimeError,
        DiagnosticRuntimeKind, ProviderType, RuntimeCatalog, TtsDiagnosticRequest,
        TtsDiagnosticValidationError, llm::LlmRequest,
    },
    workers::ProviderCapacityPermit,
};

#[derive(Clone)]
pub struct ProviderDiagnosticLimiter {
    permits: Arc<Semaphore>,
}
pub struct ProviderDiagnosticPermit {
    _permit: OwnedSemaphorePermit,
}
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ProviderDiagnosticError {
    #[error("provider diagnostic capacity is exhausted")]
    Busy,
    #[error("provider type does not support diagnostics")]
    TypeMismatch,
    #[error("provider runtime is not loaded")]
    RuntimeNotLoaded,
    #[error("provider diagnostic timed out")]
    Timeout,
    #[error("provider is unavailable")]
    Unavailable,
    #[error("provider returned an invalid diagnostic response")]
    InvalidResponse,
    #[error("provider diagnostic failed")]
    Failed,
}

/// Coarse, privacy-safe result for a public Admin provider diagnostic request.
#[derive(Debug, thiserror::Error)]
pub enum ProviderDiagnosticRequestError {
    #[error("provider was not found")]
    NotFound,
    #[error("provider is disabled")]
    Disabled,
    #[error("database is unavailable")]
    DatabaseUnavailable,
    #[error("provider type does not support this diagnostic")]
    TypeMismatch,
    #[error("provider diagnostic input is invalid")]
    InvalidInput,
    #[error(transparent)]
    Diagnostic(#[from] ProviderDiagnosticError),
}

pub struct LlmDiagnosticResult {
    pub provider_key: String,
    pub text: String,
    pub runtime: ProviderDiagnosticRuntimeMetadata,
}

pub struct TtsDiagnosticResult {
    pub provider_key: String,
    pub wav: Vec<u8>,
    pub runtime: ProviderDiagnosticRuntimeMetadata,
}

pub struct AsrDiagnosticResult {
    pub provider_key: String,
    pub text: String,
    pub language: String,
    pub runtime: ProviderDiagnosticRuntimeMetadata,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProviderDiagnosticOperationError {
    Unavailable,
    InvalidResponse,
    Failed,
}

/// Provider-specific diagnostic operation bound to one exact runtime operation or native slot.
/// `quarantine_exact` must make that operation unavailable for reuse.
#[async_trait::async_trait]
pub trait ProviderDiagnosticOperation: Send {
    type Output: Send;
    async fn execute(
        &mut self,
        cancellation: CancellationToken,
    ) -> Result<Self::Output, ProviderDiagnosticOperationError>;
    fn cancel_exact(&mut self);
    async fn await_terminal_acknowledgement(&mut self) -> bool;
    fn quarantine_exact(&mut self);
}

/// Desired-instance metadata read by the Admin boundary. `key`, never the numeric database id,
/// identifies the loaded runtime and the diagnostic operation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProviderDiagnosticTarget {
    pub key: String,
    pub provider_type: ProviderType,
    pub desired_revision: i64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProviderDiagnosticRuntimeMetadata {
    pub tested_runtime: DatabaseRuntimeStatus,
    pub runtime_matches_desired: bool,
    pub requires_restart: bool,
}

impl ProviderDiagnosticLimiter {
    pub fn new(max_concurrency: usize) -> Self {
        assert!((1..=8).contains(&max_concurrency));
        Self {
            permits: Arc::new(Semaphore::new(max_concurrency)),
        }
    }
    pub fn try_acquire(&self) -> Result<ProviderDiagnosticPermit, ProviderDiagnosticError> {
        self.permits
            .clone()
            .try_acquire_owned()
            .map(|permit| ProviderDiagnosticPermit { _permit: permit })
            .map_err(|_| ProviderDiagnosticError::Busy)
    }
}

/// The application-owned diagnostic admission boundary. It never constructs or reloads a runtime.
pub struct ProviderDiagnosticService {
    limiter: ProviderDiagnosticLimiter,
    execution_timeout: Duration,
    registry: Arc<RuntimeCatalog>,
    runtime_snapshot: Option<Arc<DatabaseRuntimeSnapshot>>,
    database: Option<Arc<Database>>,
    /// Quarantined permits intentionally stay owned until process teardown. This makes degraded
    /// capacity observable by admission rather than accidentally reusing a worker that has not
    /// acknowledged cancellation.
    quarantined_capacity: Arc<Mutex<Vec<ProviderCapacityPermit>>>,
}

pub struct ProviderDiagnosticLease {
    _diagnostic: ProviderDiagnosticPermit,
    capacity: Option<ProviderCapacityPermit>,
    quarantine: Arc<Mutex<Vec<ProviderCapacityPermit>>>,
    pub metadata: ProviderDiagnosticRuntimeMetadata,
}

impl ProviderDiagnosticService {
    pub fn new(
        registry: Arc<RuntimeCatalog>,
        runtime_snapshot: Option<Arc<DatabaseRuntimeSnapshot>>,
        database: Option<Arc<Database>>,
        limiter: ProviderDiagnosticLimiter,
        execution_timeout: Duration,
    ) -> Self {
        assert!(!execution_timeout.is_zero());
        Self {
            limiter,
            execution_timeout,
            registry,
            runtime_snapshot,
            database,
            quarantined_capacity: Arc::new(Mutex::new(Vec::new())),
        }
    }

    /// Shared execution bound used by every provider-specific diagnostic runner.
    pub fn execution_timeout(&self) -> Duration {
        self.execution_timeout
    }

    /// Executes a tool-free LLM diagnostic against the exact runtime loaded at process startup.
    /// This is the service boundary for desired-row lookup, type validation, runtime lookup,
    /// admission and timeout ownership; HTTP only supplies already-validated text and maps errors.
    pub async fn execute_llm(
        &self,
        key: &str,
        input: String,
    ) -> Result<LlmDiagnosticResult, ProviderDiagnosticRequestError> {
        let database = self
            .database
            .as_ref()
            .ok_or(ProviderDiagnosticRequestError::DatabaseUnavailable)?;
        let row: (String, String, i64, i64) =
            sqlx::query_as("SELECT key,type,enabled,revision FROM providers WHERE key=?")
                .bind(key)
                .fetch_one(database.pool())
                .await
                .map_err(|error| match error {
                    sqlx::Error::RowNotFound => ProviderDiagnosticRequestError::NotFound,
                    _ => ProviderDiagnosticRequestError::DatabaseUnavailable,
                })?;
        let (provider_key, provider_type, enabled, revision) = row;
        if enabled == 0 {
            return Err(ProviderDiagnosticRequestError::Disabled);
        }
        if provider_type != "llm" {
            return Err(ProviderDiagnosticRequestError::TypeMismatch);
        }
        let operation = self
            .registry
            .llm_diagnostic(&provider_key, LlmRequest::text_turn(input), 32 * 1024)
            .map_err(|_| ProviderDiagnosticError::RuntimeNotLoaded)?;
        let (text, runtime) = self
            .execute(
                ProviderDiagnosticTarget {
                    key: provider_key.clone(),
                    provider_type: ProviderType::Llm,
                    desired_revision: revision,
                },
                operation,
                Duration::from_millis(100),
            )
            .await?;
        Ok(LlmDiagnosticResult {
            provider_key,
            text,
            runtime,
        })
    }

    pub async fn execute_tts(
        &self,
        key: &str,
        request: TtsDiagnosticRequest,
    ) -> Result<TtsDiagnosticResult, ProviderDiagnosticRequestError> {
        let database = self
            .database
            .as_ref()
            .ok_or(ProviderDiagnosticRequestError::DatabaseUnavailable)?;
        let row: (String, String, i64, i64) =
            sqlx::query_as("SELECT key,type,enabled,revision FROM providers WHERE key=?")
                .bind(key)
                .fetch_one(database.pool())
                .await
                .map_err(|error| match error {
                    sqlx::Error::RowNotFound => ProviderDiagnosticRequestError::NotFound,
                    _ => ProviderDiagnosticRequestError::DatabaseUnavailable,
                })?;
        let (provider_key, provider_type, enabled, revision) = row;
        if enabled == 0 {
            return Err(ProviderDiagnosticRequestError::Disabled);
        }
        if provider_type != "tts" {
            return Err(ProviderDiagnosticRequestError::TypeMismatch);
        }
        self.registry
            .validate_tts_diagnostic(&provider_key, &request)
            .map_err(|error| match error {
                TtsDiagnosticValidationError::NotLoaded => {
                    ProviderDiagnosticRequestError::Diagnostic(
                        ProviderDiagnosticError::RuntimeNotLoaded,
                    )
                }
                TtsDiagnosticValidationError::InvalidInput => {
                    ProviderDiagnosticRequestError::InvalidInput
                }
            })?;
        let operation = self
            .registry
            .tts_diagnostic(&provider_key, request)
            .map_err(|_| ProviderDiagnosticError::RuntimeNotLoaded)?;
        let (output, runtime) = self
            .execute(
                ProviderDiagnosticTarget {
                    key: provider_key.clone(),
                    provider_type: ProviderType::Tts,
                    desired_revision: revision,
                },
                operation,
                Duration::from_millis(100),
            )
            .await?;
        Ok(TtsDiagnosticResult {
            provider_key,
            wav: output.wav,
            runtime,
        })
    }

    /// Executes one bounded ASR diagnostic against the runtime loaded at process startup.
    pub async fn execute_asr(
        &self,
        key: &str,
        pcm: PcmF32Mono,
    ) -> Result<AsrDiagnosticResult, ProviderDiagnosticRequestError> {
        let database = self
            .database
            .as_ref()
            .ok_or(ProviderDiagnosticRequestError::DatabaseUnavailable)?;
        let row: (String, String, String, i64, i64, String) = sqlx::query_as(
            "SELECT key,type,adapter,enabled,revision,config_json FROM providers WHERE key=?",
        )
        .bind(key)
        .fetch_one(database.pool())
        .await
        .map_err(|error| match error {
            sqlx::Error::RowNotFound => ProviderDiagnosticRequestError::NotFound,
            _ => ProviderDiagnosticRequestError::DatabaseUnavailable,
        })?;
        let (provider_key, provider_type, adapter, enabled, revision, config_json) = row;
        if enabled == 0 {
            return Err(ProviderDiagnosticRequestError::Disabled);
        }
        if provider_type != "asr" {
            return Err(ProviderDiagnosticRequestError::TypeMismatch);
        }
        let accepts_sample_rate = crate::providers::compiled_provider_adapter_registry()
            .get(&adapter)
            .filter(|descriptor| descriptor.provider_type == ProviderType::Asr)
            .and_then(|descriptor| descriptor.capabilities.input_sample_rates)
            .is_some_and(|rates| rates.contains(&pcm.sample_rate_hz()));
        if !accepts_sample_rate {
            return Err(ProviderDiagnosticRequestError::InvalidInput);
        }
        let language = serde_json::from_str::<serde_json::Value>(&config_json)
            .ok()
            .and_then(|config| config.get("language")?.as_str().map(str::to_owned))
            .unwrap_or_else(|| "und".to_owned());
        let operation = self
            .registry
            .asr_diagnostic(&provider_key, pcm)
            .map_err(|_| ProviderDiagnosticError::RuntimeNotLoaded)?;
        let (text, runtime) = self
            .execute(
                ProviderDiagnosticTarget {
                    key: provider_key.clone(),
                    provider_type: ProviderType::Asr,
                    desired_revision: revision,
                },
                operation,
                Duration::from_millis(100),
            )
            .await?;
        Ok(AsrDiagnosticResult {
            provider_key,
            text,
            language,
            runtime,
        })
    }

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
        O: ProviderDiagnosticOperation,
    {
        let lease = self.begin(target)?;
        let cancellation = CancellationToken::new();
        match tokio::time::timeout(
            self.execution_timeout,
            operation.execute(cancellation.clone()),
        )
        .await
        {
            Ok(Ok(output)) => {
                let metadata = lease.metadata.clone();
                lease.acknowledge_terminal();
                Ok((output, metadata))
            }
            Ok(Err(error)) => {
                // An operation-level error can be raised while its exact native worker is still
                // producing output (for example after a diagnostic output cap). Treat it like a
                // timeout: cancellation must be acknowledged before capacity is reusable.
                operation.cancel_exact();
                let acknowledged =
                    tokio::time::timeout(cleanup_grace, operation.await_terminal_acknowledgement())
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
            Err(_) => {
                operation.cancel_exact();
                cancellation.cancel();
                let acknowledged =
                    tokio::time::timeout(cleanup_grace, operation.await_terminal_acknowledgement())
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
        let diagnostic = self.limiter.try_acquire()?;
        let capacity = self
            .registry
            .admit_diagnostic(kind, &target.key)
            .map_err(map_runtime_error)?;
        Ok(ProviderDiagnosticLease {
            _diagnostic: diagnostic,
            capacity: Some(capacity),
            quarantine: Arc::clone(&self.quarantined_capacity),
            metadata,
        })
    }

    fn runtime_metadata(
        &self,
        target: &ProviderDiagnosticTarget,
    ) -> ProviderDiagnosticRuntimeMetadata {
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
            tested_runtime: if loaded {
                DatabaseRuntimeStatus::Loaded
            } else {
                DatabaseRuntimeStatus::NotLoaded
            },
            runtime_matches_desired: matches_desired,
            requires_restart: !matches_desired,
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
                .push(capacity);
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

#[cfg(test)]
#[path = "provider_diagnostic/tests.rs"]
mod tests;
