use super::RuntimeError;
use crate::workers::ProviderRuntimeAdmission;
use crate::{database::DesiredProvider, providers::RuntimeCatalog};
use std::sync::Arc;

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
    fn global_capacity(&self, _: &DesiredProvider) -> Result<Option<usize>, RuntimeError> {
        Ok(None)
    }
    /// Optional artifact preparation runs inside the same bounded, accounted speculative build.
    fn prepare_artifacts(&self, _: &DesiredProvider) -> Result<(), RuntimeError> {
        Ok(())
    }
    fn build(
        &self,
        snapshot: &DesiredProvider,
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
