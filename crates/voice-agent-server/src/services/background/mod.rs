//! Durable Background Jobs with managed LLM orchestration and isolated Plugin workers.
pub(crate) mod agent_loop;
mod artifacts;
pub(crate) mod engine;
// The P2 single-Step harness shares P3's production attempt/checkpoint path.
mod coverage;
#[allow(dead_code)]
mod execution;
pub(crate) mod model;
mod models;
mod orchestration;
pub(crate) mod plan;
pub mod plugins;
mod proposal;
pub(crate) mod runtime;
mod scheduler;
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
