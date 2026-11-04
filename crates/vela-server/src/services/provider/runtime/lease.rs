use super::{ProviderRuntimeManager, ProviderVersion, RuntimeResource};
use std::sync::Arc;

/// Clones share one lease lifetime; each independent acquire gets its own registry lease.
#[derive(Clone)]
pub struct ResourceLease(pub(super) Arc<LeaseLifetime>);
pub(super) struct LeaseLifetime {
    pub manager: Arc<ProviderRuntimeManager>,
    pub snapshot: crate::database::DesiredProvider,
    pub quota: crate::workers::ProviderRuntimeAdmission,
    pub version: ProviderVersion,
    pub generation: u64,
    pub resource: Arc<dyn RuntimeResource>,
}
impl ResourceLease {
    pub fn version(&self) -> &ProviderVersion {
        &self.0.version
    }
    /// Runtime handles must remain accompanied by this lease while they can be used.
    pub fn runtimes(&self) -> Option<crate::providers::RuntimeCatalog> {
        self.0
            .resource
            .runtimes_for(&self.0.snapshot, self.0.quota.clone())
    }
}
impl std::fmt::Debug for ResourceLease {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ResourceLease")
            .field("version", self.version())
            .finish_non_exhaustive()
    }
}
impl Drop for LeaseLifetime {
    fn drop(&mut self) {
        self.manager.release(&self.version, self.generation);
    }
}
