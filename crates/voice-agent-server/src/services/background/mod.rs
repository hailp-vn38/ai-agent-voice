//! Durable Background Jobs with managed LLM orchestration and isolated Plugin workers.
pub(crate) mod agent_loop;
mod artifacts;
mod coverage;
pub(crate) mod engine;
mod execution;
mod manual;
mod manual_admission;
pub(crate) mod model;
mod models;
mod orchestration;
pub mod plugins;
pub(crate) mod runtime;
mod scheduler;
mod validation;
pub(crate) mod voice;
pub use engine::JobEngine;
pub use model::{
    AgentRunView, BackgroundSettings, BackgroundSettingsUpdate, JobArtifactView, JobError,
    JobReceipt, JobRequest, JobRequester, JobSnapshot, JobStepView, JobView, ProviderOption,
    ProviderPin,
};
pub use runtime::BackgroundRuntime;
pub use voice::VoiceJobSubmit;
#[cfg(test)]
pub(crate) mod tests;
