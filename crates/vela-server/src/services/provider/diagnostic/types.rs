use std::sync::Arc;

use tokio::sync::{OwnedSemaphorePermit, Semaphore};
use tokio_util::sync::CancellationToken;

use crate::{
    providers::{DatabaseRuntimeStatus, ProviderType},
    services::provider_runtime::RuntimeError,
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
    #[error("config provider requires managed runtime")]
    ConfigManagerRequired,
    #[error("speaker runtime manager is required")]
    SpeakerManagerRequired,
    #[error("provider desired revision changed")]
    RevisionConflict,
    #[error(transparent)]
    Runtime(#[from] RuntimeError),
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
    pub media: Option<crate::providers::llm::LlmDiagnosticMedia>,
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

pub struct VadDiagnosticResult {
    pub provider_key: String,
    pub probability: f32,
    pub start_sample: u64,
    pub end_sample: u64,
    pub runtime: ProviderDiagnosticRuntimeMetadata,
}

/// One raw enrollment embedding, still unvalidated against the pinned space by the caller.
pub struct SpeakerEmbeddingResult {
    pub embedding: Vec<f32>,
    pub embedding_space_id: String,
    pub dimension: usize,
}

#[derive(serde::Serialize)]
pub struct ProviderPrepareResult {
    pub provider_key: String,
    pub desired_revision: i64,
    pub runtime: crate::services::provider_runtime::RuntimeInspection,
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
    pub tested_provider_id: Option<i64>,
    pub tested_revision: Option<i64>,
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
