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
    database::{Database, DesiredProvider},
    providers::{
        DatabaseRuntimeSnapshot, DatabaseRuntimeStatus, DiagnosticRuntimeError,
        DiagnosticRuntimeKind, ProviderType, RuntimeCatalog, TtsDiagnosticRequest,
        TtsDiagnosticValidationError, llm::LlmRequest,
    },
    services::provider_runtime::{ProviderRuntimeManager, ResourceLease, RuntimeError},
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

/// The application-owned diagnostic admission boundary. It never constructs or reloads a runtime.
pub struct ProviderDiagnosticService {
    limiter: ProviderDiagnosticLimiter,
    pilot: Option<crate::session::pilot::PilotAdmission>,
    execution_timeout: Duration,
    registry: Arc<RuntimeCatalog>,
    runtime_snapshot: Option<Arc<DatabaseRuntimeSnapshot>>,
    database: Option<Arc<Database>>,
    /// Quarantined permits intentionally stay owned until process teardown. This makes degraded
    /// capacity observable by admission rather than accidentally reusing a worker that has not
    /// acknowledged cancellation.
    quarantined_capacity: Arc<Mutex<Vec<QuarantinedDiagnosticCapacity>>>,
    manager: Option<Arc<ProviderRuntimeManager>>,
    managed_target: Option<DesiredProvider>,
    managed_admission: Mutex<Option<ProviderDiagnosticPermit>>,
    _managed_lease: Option<ResourceLease>,
}

struct QuarantinedDiagnosticCapacity {
    _pipeline: Option<crate::session::pilot::PilotPermit>,
    _capacity: ProviderCapacityPermit,
    _resource: Option<ResourceLease>,
}

pub struct ProviderDiagnosticLease {
    pipeline: Option<crate::session::pilot::PilotPermit>,
    _diagnostic: ProviderDiagnosticPermit,
    capacity: Option<ProviderCapacityPermit>,
    quarantine: Arc<Mutex<Vec<QuarantinedDiagnosticCapacity>>>,
    resource_lease: Option<ResourceLease>,
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
            pilot: None,
            execution_timeout,
            registry,
            runtime_snapshot,
            database,
            quarantined_capacity: Arc::new(Mutex::new(Vec::new())),
            manager: None,
            managed_target: None,
            managed_admission: Mutex::new(None),
            _managed_lease: None,
        }
    }

    pub fn with_pilot_admission(mut self, pilot: crate::session::pilot::PilotAdmission) -> Self {
        self.pilot = Some(pilot);
        self
    }

    pub fn with_runtime_manager(mut self, manager: Arc<ProviderRuntimeManager>) -> Self {
        self.manager = Some(manager);
        self
    }

    async fn request_snapshot(
        &self,
        key: &str,
        kind: &str,
    ) -> Result<DesiredProvider, ProviderDiagnosticRequestError> {
        if let Some(snapshot) = &self.managed_target {
            return Ok(snapshot.clone());
        }
        let database = self
            .database
            .as_ref()
            .ok_or(ProviderDiagnosticRequestError::DatabaseUnavailable)?;
        let (id, provider_key, provider_type, adapter, config, revision, enabled, credential):
            (i64, String, String, String, Option<String>, i64, i64, Option<String>) = sqlx::query_as(
            "SELECT id,key,type,adapter,CASE WHEN length(CAST(config_json AS BLOB))<=65536 THEN config_json ELSE NULL END,revision,enabled,credential_json FROM providers WHERE key=? AND length(CAST(key AS BLOB))<=128 AND length(CAST(type AS BLOB))<=16 AND length(CAST(adapter AS BLOB))<=64"
        ).bind(key).fetch_one(database.pool()).await.map_err(|error| match error {
            sqlx::Error::RowNotFound => ProviderDiagnosticRequestError::NotFound,
            _ => ProviderDiagnosticRequestError::DatabaseUnavailable,
        })?;
        if enabled == 0 {
            return Err(ProviderDiagnosticRequestError::Disabled);
        }
        if !kind.is_empty() && provider_type != kind {
            return Err(ProviderDiagnosticRequestError::TypeMismatch);
        }
        let secret_ref = crate::database::credentials::reference(
            &format!("provider:{provider_key}"),
            credential.as_deref(),
            crate::database::secrets::provider_secret_env(&provider_key, &adapter),
        );
        Ok(DesiredProvider {
            id,
            key: provider_key,
            kind: provider_type,
            adapter,
            config_json: config.ok_or(RuntimeError::Configuration)?,
            secret_ref,
            revision,
        })
    }

    async fn managed_request(
        &self,
        key: &str,
        kind: &str,
    ) -> Result<Self, ProviderDiagnosticRequestError> {
        let snapshot = self.request_snapshot(key, kind).await?;
        let admission = self.limiter.try_acquire()?;
        let manager = self.manager.as_ref().expect("managed path checked");
        let lease = manager.acquire(snapshot.clone()).await?;
        let registry = lease.runtimes().ok_or(RuntimeError::Unavailable)?;
        let runtime_snapshot = DatabaseRuntimeSnapshot::from_states([(
            snapshot.key.clone(),
            crate::providers::DatabaseRuntimeState {
                provider_id: snapshot.id,
                desired_revision: snapshot.revision,
                status: DatabaseRuntimeStatus::Loaded,
                failure: None,
            },
        )]);
        Ok(Self {
            limiter: self.limiter.clone(),
            pilot: self.pilot.clone(),
            execution_timeout: self.execution_timeout,
            registry: Arc::new(registry),
            runtime_snapshot: Some(Arc::new(runtime_snapshot)),
            database: self.database.clone(),
            quarantined_capacity: Arc::clone(&self.quarantined_capacity),
            manager: None,
            managed_target: Some(snapshot),
            managed_admission: Mutex::new(Some(admission)),
            _managed_lease: Some(lease),
        })
    }

    /// Idempotent explicit preparation. A short response wait never cancels the manager's
    /// accepted native attempt. The exact snapshot is retained by that bounded attempt.
    pub async fn prepare(
        &self,
        key: &str,
    ) -> Result<ProviderPrepareResult, ProviderDiagnosticRequestError> {
        let manager = self
            .manager
            .as_ref()
            .ok_or(ProviderDiagnosticError::RuntimeNotLoaded)?;
        let snapshot = self.request_snapshot(key, "").await?;
        let _admission = self.limiter.try_acquire()?;
        let deadline = manager
            .admission_deadline()
            .min(tokio::time::Instant::now() + Duration::from_millis(25));
        let result = manager.acquire_until(snapshot.clone(), deadline).await;
        let runtime = manager.inspect(snapshot.id, snapshot.revision);
        match result {
            Ok(lease) => drop(lease),
            Err(RuntimeError::Timeout)
                if matches!(
                    runtime.desired_state,
                    crate::services::provider_runtime::RuntimeState::Queued
                        | crate::services::provider_runtime::RuntimeState::Loading
                        | crate::services::provider_runtime::RuntimeState::Ready
                ) => {}
            Err(error) => return Err(error.into()),
        }
        Ok(ProviderPrepareResult {
            provider_key: snapshot.key,
            desired_revision: snapshot.revision,
            runtime,
        })
    }

    /// Shared execution bound used by every provider-specific diagnostic runner.
    pub fn execution_timeout(&self) -> Duration {
        self.execution_timeout
    }

    /// Speaker extraction is managed-only. The native request owns its exact lease and capacity
    /// until completion even if the bounded response wait expires.
    pub async fn execute_speaker(
        &self,
        key: &str,
        expected_revision: i64,
        pcm: PcmF32Mono,
    ) -> Result<serde_json::Value, ProviderDiagnosticRequestError> {
        let snapshot = self.request_snapshot(key, "speaker").await?;
        if snapshot.revision != expected_revision {
            return Err(ProviderDiagnosticRequestError::RevisionConflict);
        }
        let config: crate::config::CampPlusConfig = serde_json::from_str(&snapshot.config_json)
            .map_err(|_| ProviderDiagnosticRequestError::InvalidInput)?;
        if !config.valid() {
            return Err(ProviderDiagnosticRequestError::InvalidInput);
        }
        let manager = self
            .manager
            .as_ref()
            .ok_or(ProviderDiagnosticRequestError::SpeakerManagerRequired)?;
        let samples = pcm.samples();
        let rms = (samples.iter().map(|v| f64::from(*v).powi(2)).sum::<f64>()
            / samples.len().max(1) as f64)
            .sqrt();
        let clipping_fraction = samples.iter().filter(|v| v.abs() >= 0.999).count() as f64
            / samples.len().max(1) as f64;
        let duration_ms = samples.len() * 1000 / 16_000;
        if pcm.sample_rate_hz() != 16_000
            || !(1000..=12_000).contains(&duration_ms)
            || rms < 0.001
            || clipping_fraction > 0.05
        {
            return Err(ProviderDiagnosticRequestError::InvalidInput);
        }
        // ponytail: diagnostic uses the first bounded window; shared calibrated VAD/window
        // selection belongs to enrollment/Observe quality integration.
        let window_samples = usize::try_from(config.max_window_ms)
            .map_err(|_| ProviderDiagnosticRequestError::InvalidInput)?
            * 16;
        let pcm = PcmF32Mono::new(
            pcm.samples()[..pcm.samples().len().min(window_samples)].to_vec(),
            16_000,
        );
        let _diagnostic = self.limiter.try_acquire()?;
        let lease = manager.acquire(snapshot.clone()).await?;
        let runtime = lease
            .runtimes()
            .and_then(|catalog| catalog.speaker(key))
            .ok_or(ProviderDiagnosticError::Unavailable)?;
        let dimension = runtime.dimension();
        let embedding_space_id = runtime.embedding_space_id().to_owned();
        tokio::time::timeout(self.execution_timeout, runtime.extract(pcm, lease))
            .await
            .map_err(|_| ProviderDiagnosticError::Timeout)?
            .map_err(|err| match err {
                crate::providers::speaker::SpeakerError::InvalidInput => {
                    ProviderDiagnosticRequestError::InvalidInput
                }
                crate::providers::speaker::SpeakerError::Busy => {
                    ProviderDiagnosticError::Busy.into()
                }
                crate::providers::speaker::SpeakerError::InvalidEmbedding => {
                    ProviderDiagnosticError::InvalidResponse.into()
                }
                crate::providers::speaker::SpeakerError::Unavailable => {
                    ProviderDiagnosticError::Unavailable.into()
                }
            })?;
        let current = self.request_snapshot(key, "speaker").await?;
        if current.id != snapshot.id || current.revision != snapshot.revision {
            return Err(ProviderDiagnosticRequestError::RevisionConflict);
        }
        Ok(
            serde_json::json!({"provider_key": snapshot.key, "type":"speaker", "status":"success", "quality":{"duration_ms":duration_ms,"rms":rms,"clipping_fraction":clipping_fraction}, "provenance":{"embedding_space_id":embedding_space_id,"dimension":dimension,"model_revision":crate::providers::speaker::assets::MODEL_REVISION,"preprocessing":"pcm16-mono16k-v1"}, "runtime":{"tested_provider_id":snapshot.id,"tested_revision":snapshot.revision,"runtime_matches_desired":true,"requires_restart":false}}),
        )
    }

    /// One enrollment embedding. The same managed-only bounded-worker path as
    /// [`Self::execute_speaker`], but it returns the vector instead of a diagnostic summary. The
    /// caller has already run the quality gate and selected the window, so this boundary owns only
    /// runtime lookup, admission, timeout, and the post-run revision check.
    pub async fn extract_speaker_embedding(
        &self,
        key: &str,
        expected_revision: i64,
        pcm: PcmF32Mono,
    ) -> Result<SpeakerEmbeddingResult, ProviderDiagnosticRequestError> {
        let snapshot = self.request_snapshot(key, "speaker").await?;
        if snapshot.revision != expected_revision {
            return Err(ProviderDiagnosticRequestError::RevisionConflict);
        }
        let config: crate::config::CampPlusConfig = serde_json::from_str(&snapshot.config_json)
            .map_err(|_| ProviderDiagnosticRequestError::InvalidInput)?;
        if !config.valid() {
            return Err(ProviderDiagnosticRequestError::InvalidInput);
        }
        let manager = self
            .manager
            .as_ref()
            .ok_or(ProviderDiagnosticRequestError::SpeakerManagerRequired)?;
        let _diagnostic = self.limiter.try_acquire()?;
        let lease = manager.acquire(snapshot.clone()).await?;
        let runtime = lease
            .runtimes()
            .and_then(|catalog| catalog.speaker(key))
            .ok_or(ProviderDiagnosticError::Unavailable)?;
        let dimension = runtime.dimension();
        let embedding_space_id = runtime.embedding_space_id().to_owned();
        let embedding = tokio::time::timeout(self.execution_timeout, runtime.extract(pcm, lease))
            .await
            .map_err(|_| ProviderDiagnosticError::Timeout)?
            .map_err(|err| match err {
                crate::providers::speaker::SpeakerError::InvalidInput => {
                    ProviderDiagnosticRequestError::InvalidInput
                }
                crate::providers::speaker::SpeakerError::Busy => {
                    ProviderDiagnosticError::Busy.into()
                }
                crate::providers::speaker::SpeakerError::InvalidEmbedding => {
                    ProviderDiagnosticError::InvalidResponse.into()
                }
                crate::providers::speaker::SpeakerError::Unavailable => {
                    ProviderDiagnosticError::Unavailable.into()
                }
            })?;
        let current = self.request_snapshot(key, "speaker").await?;
        if current.id != snapshot.id || current.revision != snapshot.revision {
            return Err(ProviderDiagnosticRequestError::RevisionConflict);
        }
        Ok(SpeakerEmbeddingResult {
            embedding,
            embedding_space_id,
            dimension,
        })
    }

    /// Executes a tool-free LLM diagnostic against the exact runtime loaded at process startup.
    /// This is the service boundary for desired-row lookup, type validation, runtime lookup,
    /// admission and timeout ownership; HTTP only supplies already-validated text and maps errors.
    pub async fn execute_llm(
        &self,
        key: &str,
        input: String,
    ) -> Result<LlmDiagnosticResult, ProviderDiagnosticRequestError> {
        if self.manager.is_some() {
            let service = self.managed_request(key, "llm").await?;
            return Box::pin(service.execute_llm(key, input)).await;
        }
        let snapshot = self.request_snapshot(key, "llm").await?;
        let provider_key = snapshot.key.clone();
        let revision = snapshot.revision;
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
        if self.manager.is_some() {
            let service = self.managed_request(key, "tts").await?;
            return Box::pin(service.execute_tts(key, request)).await;
        }
        let snapshot = self.request_snapshot(key, "tts").await?;
        let provider_key = snapshot.key.clone();
        let revision = snapshot.revision;
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

    /// Executes exactly one canonical silence frame against the VAD runtime loaded at startup.
    pub async fn execute_vad(
        &self,
        key: &str,
    ) -> Result<VadDiagnosticResult, ProviderDiagnosticRequestError> {
        if self.manager.is_some() {
            let service = self.managed_request(key, "vad").await?;
            return Box::pin(service.execute_vad(key)).await;
        }
        let snapshot = self.request_snapshot(key, "vad").await?;
        let provider_key = snapshot.key.clone();
        let revision = snapshot.revision;
        let operation = self
            .registry
            .vad_diagnostic(&provider_key)
            .map_err(|_| ProviderDiagnosticError::RuntimeNotLoaded)?;
        let (probability, runtime) = self
            .execute(
                ProviderDiagnosticTarget {
                    key: provider_key.clone(),
                    provider_type: ProviderType::Vad,
                    desired_revision: revision,
                },
                operation,
                Duration::from_millis(100),
            )
            .await?;
        Ok(VadDiagnosticResult {
            provider_key,
            probability: probability.probability,
            start_sample: probability.start_sample,
            end_sample: probability.end_sample,
            runtime,
        })
    }

    /// Executes one bounded ASR diagnostic against the runtime loaded at process startup.
    pub async fn execute_asr(
        &self,
        key: &str,
        pcm: PcmF32Mono,
    ) -> Result<AsrDiagnosticResult, ProviderDiagnosticRequestError> {
        if self.manager.is_some() {
            let service = self.managed_request(key, "asr").await?;
            return Box::pin(service.execute_asr(key, pcm)).await;
        }
        let snapshot = self.request_snapshot(key, "asr").await?;
        let provider_key = snapshot.key.clone();
        let revision = snapshot.revision;
        let adapter = snapshot.adapter;
        let config_json = snapshot.config_json;
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

#[cfg(test)]
#[path = "provider_diagnostic/tests.rs"]
mod tests;
