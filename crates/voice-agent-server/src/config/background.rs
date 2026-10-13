use serde::{Deserialize, Serialize};

/// Deployment resource policy only. Enablement and model choices are revisioned Admin data.
#[derive(Clone, Debug, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct BackgroundConfig {
    #[serde(default)]
    pub limits: BackgroundLimits,
    /// OCI digests supplied by deployment; mutable tags are rejected by preflight.
    pub plugin_python_image: Option<String>,
    pub plugin_uv_image: Option<String>,
    /// Offline Cargo build image pinned by OCI repository digest.
    pub plugin_rust_builder_image: Option<String>,
    /// Minimal Rust runtime image pinned by OCI repository digest.
    pub plugin_rust_runtime_image: Option<String>,
    /// Locally built, digest-pinned image from deploy/agent-egress-proxy/.
    pub plugin_egress_image: Option<String>,
    /// Exact hosts qualified by adversarial egress tests on this deployment.
    #[serde(default)]
    pub plugin_egress_qualified_hosts: Vec<String>,
    /// Dedicated rootless Engine socket, e.g. unix:///run/user/1001/docker.sock.
    pub plugin_docker_host: Option<String>,
    /// Directory of operator-provided ZIPs named <curated-id>.zip.
    pub plugin_curated_dir: Option<std::path::PathBuf>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct BackgroundLimits {
    pub max_queued_jobs: u32,
    pub max_active_jobs: u32,
    pub max_container_workers: u32,
    pub max_steps: u32,
    pub max_replans: u32,
    pub max_plan_repairs: u32,
    pub max_worker_rounds: u32,
    pub job_deadline_seconds: u32,
    pub step_timeout_seconds: u32,
    pub artifact_bytes_per_job: u64,
    pub scheduler_scan_ms: u64,
}
impl Default for BackgroundLimits {
    fn default() -> Self {
        Self {
            max_queued_jobs: 64,
            max_active_jobs: 2,
            max_container_workers: 2,
            max_steps: 12,
            max_replans: 2,
            max_plan_repairs: 2,
            max_worker_rounds: 8,
            job_deadline_seconds: 1800,
            step_timeout_seconds: 300,
            artifact_bytes_per_job: 20 * 1024 * 1024,
            scheduler_scan_ms: 1000,
        }
    }
}
impl BackgroundLimits {
    pub fn valid(&self) -> bool {
        (1..=256).contains(&self.max_queued_jobs)
            && (1..=2).contains(&self.max_active_jobs)
            && (1..=2).contains(&self.max_container_workers)
            && (1..=12).contains(&self.max_steps)
            && self.max_replans <= 2
            && self.max_plan_repairs <= 2
            && (1..=8).contains(&self.max_worker_rounds)
            && (1..=1800).contains(&self.job_deadline_seconds)
            && (1..=300).contains(&self.step_timeout_seconds)
            && self.step_timeout_seconds <= self.job_deadline_seconds
            && (1..=20 * 1024 * 1024).contains(&self.artifact_bytes_per_job)
            && (10..=5000).contains(&self.scheduler_scan_ms)
    }
}
