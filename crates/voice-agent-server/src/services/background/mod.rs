//! Durable Job foundation. Production submission remains closed until P3 supplies
//! the planner and executable worker; no placeholder executor accepts stuck work.
mod engine;
pub(crate) mod model;
pub(crate) mod providers;
mod runtime;
mod scheduler;
pub use engine::JobEngine;
pub use model::{
    BackgroundSettings, BackgroundSettingsUpdate, JobError, JobReceipt, JobRequest, JobRequester,
    JobSnapshot, JobStepView, JobView, ProviderOption, ProviderPin,
};
pub use runtime::BackgroundRuntime;
#[cfg(test)]
mod tests;
