//! Durable Job foundation. Production submission remains closed until P3 supplies
//! the planner and executable worker; no placeholder executor accepts stuck work.
mod artifacts;
mod engine;
// P2 exposes this path only to the internal Step integration harness.
#[allow(dead_code)]
mod execution;
pub(crate) mod model;
pub mod plugins;
pub(crate) mod providers;
mod runtime;
mod scheduler;
pub use engine::JobEngine;
pub use model::{
    BackgroundSettings, BackgroundSettingsUpdate, JobArtifactView, JobError, JobReceipt,
    JobRequest, JobRequester, JobSnapshot, JobStepView, JobView, ProviderOption, ProviderPin,
};
pub use runtime::BackgroundRuntime;
#[cfg(test)]
mod tests;
