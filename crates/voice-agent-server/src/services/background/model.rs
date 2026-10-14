use crate::config::BackgroundLimits;
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct JobRequest {
    pub goal: String,
    #[serde(default = "default_format")]
    pub output_format: String,
    #[serde(default)]
    pub allow_partial: bool,
    #[serde(default)]
    pub required_sources: Vec<String>,
    #[serde(default)]
    pub required_capabilities: Vec<String>,
}
fn default_format() -> String {
    "summary".into()
}
impl JobRequest {
    pub(crate) fn normalize(mut self) -> Result<Self, JobError> {
        self.goal = self.goal.trim().to_owned();
        if self.goal.is_empty()
            || self.goal.chars().count() > 3000
            || !matches!(
                self.output_format.as_str(),
                "summary" | "text" | "json" | "markdown"
            )
        {
            return Err(JobError::InvalidArguments);
        }
        for tokens in [&mut self.required_sources, &mut self.required_capabilities] {
            if tokens.len() > 16
                || tokens.iter().any(|s| {
                    s.is_empty()
                        || s.len() > 64
                        || !s
                            .bytes()
                            .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
                })
            {
                return Err(JobError::InvalidArguments);
            }
            tokens.sort();
            tokens.dedup();
        }
        Ok(self)
    }
}

/// Only authenticated server boundaries construct requester scope. AUTOINCREMENT IDs
/// identify the original rows, including after a public Device ID is deleted/recreated.
#[derive(Clone, Debug)]
pub enum JobRequester {
    Admin,
    Voice {
        device_incarnation: i64,
        agent_incarnation: i64,
    },
}
impl JobRequester {
    pub(crate) fn scope(&self) -> String {
        match self {
            Self::Admin => "admin:deployment".into(),
            Self::Voice {
                device_incarnation,
                agent_incarnation,
            } => format!("voice:{device_incarnation}:{agent_incarnation}"),
        }
    }
    pub(crate) fn incarnations(&self) -> (Option<i64>, Option<i64>) {
        match self {
            Self::Admin => (None, None),
            Self::Voice {
                device_incarnation,
                agent_incarnation,
            } => (Some(*device_incarnation), Some(*agent_incarnation)),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct BackgroundSettingsUpdate {
    pub enabled: bool,
    pub orchestrator_llm_provider: Option<String>,
    pub worker_llm_provider: Option<String>,
}
#[derive(Clone, Debug, Serialize)]
pub struct BackgroundSettings {
    #[serde(flatten)]
    pub settings: BackgroundSettingsUpdate,
    pub revision: i64,
    pub llm_providers: Vec<ProviderOption>,
    pub runtime_ready: bool,
}
#[derive(Clone, Debug, Serialize)]
pub struct ProviderOption {
    pub r#ref: String,
    pub name: String,
    pub source: &'static str,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProviderPin {
    pub reference: String,
    pub incarnation: i64,
    pub revision: i64,
    pub digest: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct JobSnapshot {
    pub orchestrator: ProviderPin,
    pub worker: ProviderPin,
    pub limits: BackgroundLimits,
}
#[derive(Clone, Debug, Serialize)]
pub struct JobView {
    pub id: String,
    #[serde(flatten)]
    pub request: JobRequest,
    pub status: String,
    pub requester_scope: String,
    pub snapshot: JobSnapshot,
    pub plugin_choices: Value,
    pub settings_revision: i64,
    pub deadline_at: i64,
    pub created_at: String,
    pub error_code: Option<String>,
    pub scheduler_seen_at: Option<i64>,
    pub plan_revision: i64,
    pub retry_of_job_id: Option<String>,
    pub result: Option<Value>,
    pub budget_used: Value,
    pub steps: Vec<JobStepView>,
    pub artifacts: Vec<JobArtifactView>,
}
#[derive(Clone, Debug, Serialize, PartialEq, Eq, sqlx::FromRow)]
pub struct JobArtifactView {
    pub id: String,
    pub media_type: String,
    pub size_bytes: i64,
    pub sha256: String,
}
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct JobStepView {
    pub step_key: String,
    pub status: String,
    pub plugin_id: Option<String>,
    pub operation: Option<String>,
    pub error_code: Option<String>,
    pub attempt_state: Option<String>,
    pub instruction: String,
    pub required: bool,
    pub dependencies: Vec<String>,
    pub input_refs: Vec<String>,
    pub arguments: Value,
    pub output_schema: Value,
    pub result: Option<Value>,
}
#[derive(Debug)]
pub struct JobReceipt {
    pub job: JobView,
    pub replayed: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JobError {
    InvalidArguments,
    RuntimeUnavailable,
    Disabled,
    AuthorityRevoked,
    ProviderUnavailable,
    ConfigurationChanged,
    CapacityUnavailable,
    QueueFull,
    CapabilityUnavailable,
    IdempotencyConflict,
    RevisionConflict,
    Terminal,
    NotFound,
    DeadlineExceeded,
    DatabaseUnavailable,
    DeploymentLock,
    ClockUnavailable,
    InvalidPlan,
    ModelFailed,
    Cancelled,
}
impl JobError {
    pub(crate) fn from_plugin(error: super::plugins::PluginError) -> Self {
        match error {
            super::plugins::PluginError::JobRejected(error) => error,
            super::plugins::PluginError::DatabaseUnavailable => Self::DatabaseUnavailable,
            super::plugins::PluginError::SandboxUnavailable => Self::RuntimeUnavailable,
            _ => Self::CapabilityUnavailable,
        }
    }
    pub fn code(self) -> &'static str {
        match self {
            Self::InvalidArguments => "invalid_arguments",
            Self::RuntimeUnavailable => "runtime_unavailable",
            Self::Disabled => "background_disabled",
            Self::AuthorityRevoked => "authority_revoked",
            Self::ProviderUnavailable => "llm_provider_unavailable",
            Self::ConfigurationChanged => "configuration_changed",
            Self::CapacityUnavailable => "background_capacity_unavailable",
            Self::QueueFull => "queue_full",
            Self::CapabilityUnavailable => "capability_unavailable",
            Self::IdempotencyConflict => "idempotency_conflict",
            Self::RevisionConflict => "revision_conflict",
            Self::Terminal => "job_terminal",
            Self::NotFound => "not_found",
            Self::DeadlineExceeded => "deadline_exceeded",
            Self::DatabaseUnavailable => "database_unavailable",
            Self::DeploymentLock => "deployment_lock_unavailable",
            Self::ClockUnavailable => "clock_unavailable",
            Self::InvalidPlan => "invalid_plan",
            Self::ModelFailed => "llm_execution_failed",
            Self::Cancelled => "cancelled",
        }
    }
}
impl std::fmt::Display for JobError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.code())
    }
}
impl std::error::Error for JobError {}

impl From<sqlx::Error> for JobError {
    fn from(_: sqlx::Error) -> Self {
        Self::DatabaseUnavailable
    }
}
pub(crate) fn encode(value: &impl Serialize) -> Result<String, JobError> {
    serde_json::to_string(value).map_err(|_| JobError::InvalidArguments)
}
pub(crate) fn digest(value: &str) -> String {
    use sha2::{Digest, Sha256};
    Sha256::digest(value.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

#[derive(Clone, Debug)]
pub(crate) struct LlmRunContext {
    pub run_id: String,
    pub job_id: String,
    pub step_key: Option<String>,
    pub role: &'static str,
    pub phase: &'static str,
    pub attempt_no: u32,
    pub plan_revision: i64,
}
impl LlmRunContext {
    pub(crate) fn new(job_id: &str, step_key: Option<&str>, role: &'static str, phase: &'static str, attempt_no: u32, plan_revision: i64) -> Self {
        Self {
            run_id: uuid::Uuid::new_v4().to_string(), job_id: job_id.to_owned(),
            step_key: step_key.map(str::to_owned), role, phase, attempt_no, plan_revision
        }
    }
}
