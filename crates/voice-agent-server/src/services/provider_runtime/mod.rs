//! Application-owned exact-version acquisition. No database access or Voice Session state.
mod budget;
mod factory;
mod identity;
mod lease;
mod lifecycle;
mod materialize;
mod plan;
mod registry;
mod status;

pub use budget::RuntimeLimits;
pub use factory::{FactoryDiagnostics, FactoryMaterializer};
pub use identity::{ProviderIdentity, ProviderVersion, ResourceKey};
pub use lease::ResourceLease;
pub use materialize::{MaterializationTimings, RuntimeMaterializer, RuntimeResource};
pub use plan::{LocalExecutionRequirements, LocalRuntimePlan, PhysicalCapacity};
pub use registry::ProviderRuntimeManager;
pub use status::{RuntimeAccounting, RuntimeError, RuntimeInspection, RuntimeState};

mod metrics;
pub use metrics::{RuntimeCounter, RuntimeMetrics, RuntimePhase};
