//! Admission and lifecycle ownership for bounded provider diagnostics.
//!
//! HTTP handlers deliberately do not own these permits: a request timeout only asks a worker to
//! stop. The lease remains alive until a terminal acknowledgement or an explicit quarantine.

use std::{
    sync::{Arc, Mutex},
    time::Duration,
};

use crate::{
    database::{Database, DesiredProvider},
    providers::{
        DatabaseRuntimeSnapshot, DatabaseRuntimeStatus, RuntimeCatalog,
    },
    services::provider_runtime::{ProviderRuntimeManager, ResourceLease, RuntimeError},
    workers::ProviderCapacityPermit,
};
#[cfg(test)]
use crate::providers::ProviderType;

mod execution;
mod runners;
mod speaker;
mod types;

pub use types::*;

/// The application-owned diagnostic admission boundary. It never constructs or reloads a runtime.
pub struct ProviderDiagnosticService {
    pub(self) limiter: ProviderDiagnosticLimiter,
    pub(self) pilot: Option<crate::session::pilot::PilotAdmission>,
    pub(self) execution_timeout: Duration,
    pub(self) registry: Arc<RuntimeCatalog>,
    pub(self) runtime_snapshot: Option<Arc<DatabaseRuntimeSnapshot>>,
    pub(self) database: Option<Arc<Database>>,
    /// Quarantined permits intentionally stay owned until process teardown. This makes degraded
    /// capacity observable by admission rather than accidentally reusing a worker that has not
    /// acknowledged cancellation.
    pub(self) quarantined_capacity: Arc<Mutex<Vec<QuarantinedDiagnosticCapacity>>>,
    pub(self) manager: Option<Arc<ProviderRuntimeManager>>,
    pub(self) config: Option<Arc<crate::config::AppConfig>>,
    pub(self) managed_target: Option<DesiredProvider>,
    pub(self) managed_admission: Mutex<Option<ProviderDiagnosticPermit>>,
    pub(self) _managed_lease: Option<ResourceLease>,
}

pub(self) struct QuarantinedDiagnosticCapacity {
    pub(self) _pipeline: Option<crate::session::pilot::PilotPermit>,
    pub(self) _capacity: ProviderCapacityPermit,
    pub(self) _resource: Option<ResourceLease>,
}

pub struct ProviderDiagnosticLease {
    pub(self) pipeline: Option<crate::session::pilot::PilotPermit>,
    pub(self) _diagnostic: ProviderDiagnosticPermit,
    pub(self) capacity: Option<ProviderCapacityPermit>,
    pub(self) quarantine: Arc<Mutex<Vec<QuarantinedDiagnosticCapacity>>>,
    pub(self) resource_lease: Option<ResourceLease>,
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
            config: None,
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

    pub fn with_config(mut self, config: Arc<crate::config::AppConfig>) -> Self {
        self.config = Some(config);
        self
    }

    pub(self) async fn request_snapshot(
        &self,
        key: &str,
        kind: &str,
    ) -> Result<DesiredProvider, ProviderDiagnosticRequestError> {
        if let Some(snapshot) = &self.managed_target {
            return Ok(snapshot.clone());
        }
        let reference = crate::providers::config_catalog::ProviderRef::parse(
            key,
            (!kind.is_empty()).then_some(kind),
        )
        .map_err(|_| ProviderDiagnosticRequestError::TypeMismatch)?;
        if reference.origin == crate::providers::config_catalog::ProviderOrigin::Config {
            if self.manager.is_none() {
                return Err(ProviderDiagnosticRequestError::ConfigManagerRequired);
            }
            return reference
                .snapshot(self.config.as_deref().ok_or(RuntimeError::Configuration)?)
                .map_err(|_| ProviderDiagnosticRequestError::NotFound);
        }
        let database = self
            .database
            .as_ref()
            .ok_or(ProviderDiagnosticRequestError::DatabaseUnavailable)?;
        let (id, provider_key, provider_type, adapter, config, revision, enabled, credential): (
            i64,
            String,
            String,
            String,
            Option<String>,
            i64,
            i64,
            Option<String>,
        ) = database
            .diagnostic_provider_row(key)
            .await
            .map_err(|error| match error {
                sqlx::Error::RowNotFound => ProviderDiagnosticRequestError::NotFound,
                _ => ProviderDiagnosticRequestError::DatabaseUnavailable,
            })?;
        if enabled == 0 {
            return Err(ProviderDiagnosticRequestError::Disabled);
        }
        if !kind.is_empty() && provider_type != kind {
            return Err(ProviderDiagnosticRequestError::TypeMismatch);
        }
        let config_json = config.ok_or(RuntimeError::Configuration)?;
        let secret_ref = crate::database::credentials::reference(
            &format!("provider:{provider_key}"),
            credential.as_deref(),
            crate::database::secrets::provider_secret_env_for_config(
                &provider_key,
                &adapter,
                &config_json,
            ),
        );
        Ok(DesiredProvider {
            id,
            key: provider_key,
            kind: provider_type,
            adapter,
            config_json,
            secret_ref,
            revision,
        })
    }

    pub(self) async fn managed_request(
        &self,
        key: &str,
        kind: &str,
    ) -> Result<Self, ProviderDiagnosticRequestError> {
        let snapshot = self.request_snapshot(key, kind).await?;
        let admission = self.limiter.try_acquire()?;
        let manager = self.manager.as_ref().expect("managed path checked");
        let test_deadline = tokio::time::Instant::now() + self.execution_timeout;
        let deadline = manager.admission_deadline().min(test_deadline);
        let reference = crate::providers::config_catalog::ProviderRef::parse(key, Some(kind))
            .map_err(|_| ProviderDiagnosticRequestError::TypeMismatch)?;
        let lease = match reference.origin {
            crate::providers::config_catalog::ProviderOrigin::Config => {
                manager
                    .acquire_deployment_until(snapshot.clone(), deadline)
                    .await?
            }
            crate::providers::config_catalog::ProviderOrigin::Database => {
                manager.acquire_until(snapshot.clone(), deadline).await?
            }
        };
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
            execution_timeout: test_deadline.saturating_duration_since(tokio::time::Instant::now()),
            registry: Arc::new(registry),
            runtime_snapshot: Some(Arc::new(runtime_snapshot)),
            database: self.database.clone(),
            quarantined_capacity: Arc::clone(&self.quarantined_capacity),
            manager: None,
            config: None,
            managed_target: Some(snapshot),
            managed_admission: Mutex::new(Some(admission)),
            _managed_lease: Some(lease),
        })
    }

    pub(crate) async fn draft_request(
        &self,
        snapshot: DesiredProvider,
        secrets: Arc<dyn crate::database::secrets::SecretResolver>,
    ) -> Result<Self, ProviderDiagnosticRequestError> {
        let admission = self.limiter.try_acquire()?;
        let manager = self
            .manager
            .as_ref()
            .ok_or(ProviderDiagnosticError::Unavailable)?;
        let test_deadline = tokio::time::Instant::now() + self.execution_timeout;
        let deadline = manager.admission_deadline().min(test_deadline);
        let lease = manager
            .acquire_draft_until(snapshot.clone(), secrets, deadline)
            .await?;
        let registry = lease.runtimes().ok_or(RuntimeError::Unavailable)?;
        Ok(Self {
            limiter: self.limiter.clone(),
            pilot: self.pilot.clone(),
            execution_timeout: test_deadline.saturating_duration_since(tokio::time::Instant::now()),
            registry: Arc::new(registry),
            runtime_snapshot: None,
            database: None,
            quarantined_capacity: self.quarantined_capacity.clone(),
            manager: None,
            config: None,
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
}

#[cfg(test)]
mod tests;
