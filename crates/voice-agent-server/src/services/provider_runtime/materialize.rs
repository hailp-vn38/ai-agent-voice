use super::RuntimeError;
use crate::workers::ProviderRuntimeAdmission;
use crate::{database::DesiredProvider, providers::RuntimeCatalog};
use std::{sync::Arc, time::Duration};

/// Wall-clock cost of each materialization stage, captured by the builder that retained the
/// resource. Values carry no provider identity, revision or resource hash, so they stay safe as
/// fixed-cardinality metrics and structured log fields.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MaterializationTimings {
    pub artifact_prepare: Duration,
    pub artifact_verify: Duration,
    pub provider_contract: Duration,
}

/// What preparation established before `build` runs.
///
/// Preparation ensures the provider's files exist; `build` then resolves their paths and constructs
/// the runtime. Carrying the outcome forward keeps `build` from re-entering the asset layer, so a
/// materialization never downloads twice.
#[derive(Clone, Copy, Debug)]
pub enum PreparedRuntime {
    /// A remote adapter has no local model files to prepare.
    Remote,
    /// The provider's model files were present before the runtime was built.
    Local {
        /// Wall time spent ensuring assets, so a materialization can report which phase dominated.
        ensure: Duration,
    },
}

/// Blocking/native factory seam. The manager bounds execution and reserves before build.
/// A normal error means partial allocations have already acknowledged cleanup; uncertainty
/// must be reported as Quarantined so accounting and loader capacity remain reserved.
pub trait RuntimeMaterializer: Send + Sync {
    /// Explicit opt-in to share a backing pool. None conservatively isolates versions.
    fn resource_key(
        &self,
        _: &DesiredProvider,
    ) -> Result<Option<super::ResourceKey>, RuntimeError> {
        Ok(None)
    }
    fn estimated_peak_bytes(&self, snapshot: &DesiredProvider) -> Result<u64, RuntimeError>;
    fn logical_capacity(&self, snapshot: &DesiredProvider) -> Result<usize, RuntimeError>;
    /// Backing native topology owned by one materialization. Defaults to the logical capacity for
    /// adapters that hold no resident per-worker native state; adapters with fixed replica
    /// topology override it so application concurrency cannot multiply native resources.
    fn physical_capacity(&self, snapshot: &DesiredProvider) -> Result<usize, RuntimeError> {
        self.logical_capacity(snapshot)
    }
    fn global_capacity(&self, _: &DesiredProvider) -> Result<Option<usize>, RuntimeError> {
        Ok(None)
    }
    /// Optional artifact preparation runs inside the same bounded, accounted speculative build.
    /// Whatever it resolves is handed to `build`, so preparation is never repeated there. `None`
    /// means preparation did not run for this build.
    fn prepare_artifacts(
        &self,
        _: &DesiredProvider,
    ) -> Result<Option<PreparedRuntime>, RuntimeError> {
        Ok(None)
    }
    /// Request-scoped credentials remain owned by the manager's native attempt.
    fn build_draft(
        &self,
        snapshot: &DesiredProvider,
        prepared: Option<PreparedRuntime>,
        quota: ProviderRuntimeAdmission,
        secrets: &dyn crate::database::secrets::SecretResolver,
    ) -> Result<Arc<dyn RuntimeResource>, RuntimeError> {
        let _ = (snapshot, prepared, quota, secrets);
        Err(RuntimeError::Configuration)
    }
    /// `prepared` carries whatever `prepare_artifacts` already resolved. A build that receives
    /// `None` must resolve the model itself, but may still reuse an already-trusted process result.
    fn build(
        &self,
        snapshot: &DesiredProvider,
        prepared: Option<PreparedRuntime>,
        quota: ProviderRuntimeAdmission,
    ) -> Result<Arc<dyn RuntimeResource>, RuntimeError>;
}

/// Resident backing resource. Unload returns true only after worker/thread/subprocess exit.
/// A resource with outstanding native work must keep returning false and remain quarantined.
pub trait RuntimeResource: Send + Sync {
    /// Safe metadata captured from the exact materialized adapter. No native execution.
    /// Actual installed-artifact identity captured by the builder, when available.
    fn resource_key(&self) -> Option<super::ResourceKey> {
        None
    }
    /// Cached capacity counters only; queried once at completion, never during GET.
    fn physical_admission(&self) -> Option<ProviderRuntimeAdmission> {
        None
    }
    /// Stage timings for the materialization that produced this resource. Adapters that build no
    /// native state report zeroed timings.
    fn materialization_timings(&self) -> MaterializationTimings {
        MaterializationTimings::default()
    }
    fn readiness(&self) -> crate::workers::NativeReadiness {
        Default::default()
    }
    fn health_flags(&self) -> Vec<Arc<std::sync::atomic::AtomicBool>> {
        Vec::new()
    }
    fn capabilities(&self) -> Option<serde_json::Value> {
        None
    }
    fn unload(&self) -> bool;
    fn runtimes_for(
        &self,
        _: &DesiredProvider,
        _: ProviderRuntimeAdmission,
    ) -> Option<RuntimeCatalog> {
        self.runtimes()
    }
    fn runtimes(&self) -> Option<RuntimeCatalog> {
        None
    }
}
