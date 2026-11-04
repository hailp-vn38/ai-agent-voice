use serde::Serialize;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeState {
    Cold,
    Queued,
    Loading,
    Ready,
    Failed,
    Draining,
    Quarantined,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum RuntimeError {
    #[error("provider_runtime_busy")]
    Busy,
    #[error("provider_runtime_memory_pressure")]
    MemoryPressure,
    #[error("provider_runtime_timeout")]
    Timeout,
    #[error("provider_runtime_unavailable")]
    Unavailable,
    #[error("provider_runtime_quarantined")]
    Quarantined,
    #[error("provider_artifacts_not_ready")]
    ArtifactsNotReady,
    #[error("provider_config_invalid")]
    Configuration,
    #[error("server_is_shutting_down")]
    ShuttingDown,
}
impl RuntimeError {
    pub fn code(self) -> &'static str {
        match self {
            Self::Busy => "provider_runtime_busy",
            Self::MemoryPressure => "provider_runtime_memory_pressure",
            Self::Timeout => "provider_runtime_timeout",
            Self::Unavailable => "provider_runtime_unavailable",
            Self::Quarantined => "provider_runtime_quarantined",
            Self::ArtifactsNotReady => "provider_artifacts_not_ready",
            Self::Configuration => "provider_config_invalid",
            Self::ShuttingDown => "server_is_shutting_down",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
pub struct RuntimeAccounting {
    /// Conservative peak estimates retained through resident/draining/quarantined states.
    pub reserved_bytes: u64,
    pub resources: usize,
    pub version_entries: usize,
    pub logical_inference_usage: usize,
    pub global_inference_usage: usize,
    pub physical_inference_usage: usize,
    pub reserved_build_bytes: u64,
    pub draining_bytes: u64,
    pub quarantined_bytes: u64,
    pub active_leases: usize,
    pub waiters: usize,
    pub build_attempts: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct RuntimeInspection {
    pub desired_revision: i64,
    pub desired_state: RuntimeState,
    pub ready_revisions: Vec<i64>,
    pub can_prepare: bool,
    pub failure_code: Option<String>,
}
