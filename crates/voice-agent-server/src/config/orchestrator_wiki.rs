use llm_wiki::WikiTemplateId;
use serde::Deserialize;
use std::path::PathBuf;

/// Deployment configuration for the unified `orchestrator-flow` + `orchestrator-agent` engine (`[orchestrator]`).
#[derive(Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct OrchestratorSectionConfig {
    pub enabled: bool,
    pub endpoint: String,
    pub api_key: String,
    pub model: String,
    pub store_dir: PathBuf,
    pub tasks_store_dir: PathBuf,
    pub reports_dir: PathBuf,
    pub work_dir: PathBuf,
    pub skills_dir: Option<PathBuf>,
    pub prompts_dir: PathBuf,
    pub default_timezone: String,
    pub context_limit_tokens: usize,
    pub max_output_tokens: u32,
    pub request_timeout_ms: u64,
    pub max_concurrent_root_runs: Option<usize>,
}

impl std::fmt::Debug for OrchestratorSectionConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OrchestratorSectionConfig")
            .field("enabled", &self.enabled)
            .field("endpoint", &self.endpoint)
            .field("api_key", &"[REDACTED]")
            .field("model", &self.model)
            .field("store_dir", &self.store_dir)
            .field("tasks_store_dir", &self.tasks_store_dir)
            .field("reports_dir", &self.reports_dir)
            .field("work_dir", &self.work_dir)
            .field("skills_dir", &self.skills_dir)
            .field("prompts_dir", &self.prompts_dir)
            .field("default_timezone", &self.default_timezone)
            .field("context_limit_tokens", &self.context_limit_tokens)
            .field("max_output_tokens", &self.max_output_tokens)
            .field("request_timeout_ms", &self.request_timeout_ms)
            .field("max_concurrent_root_runs", &self.max_concurrent_root_runs)
            .finish()
    }
}

impl Default for OrchestratorSectionConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            endpoint: "http://localhost:20128/v1".into(),
            api_key: String::new(),
            model: "cx/gpt-5.5".into(),
            store_dir: PathBuf::from("data/orchestrator-store"),
            tasks_store_dir: PathBuf::from("data/tasks-store"),
            reports_dir: PathBuf::from("data/reports"),
            work_dir: PathBuf::from("data/orchestrator-work"),
            skills_dir: Some(PathBuf::from("skills")),
            prompts_dir: PathBuf::from("data/orchestrator-prompts"),
            default_timezone: "Asia/Ho_Chi_Minh".into(),
            context_limit_tokens: 200_000,
            max_output_tokens: 16_384,
            request_timeout_ms: 240_000,
            max_concurrent_root_runs: Some(2),
        }
    }
}

impl OrchestratorSectionConfig {
    pub fn valid(&self) -> bool {
        if !self.enabled {
            return true;
        }
        !self.endpoint.trim().is_empty()
            && !self.model.trim().is_empty()
            && !self.store_dir.as_os_str().is_empty()
            && !self.tasks_store_dir.as_os_str().is_empty()
            && !self.reports_dir.as_os_str().is_empty()
            && !self.work_dir.as_os_str().is_empty()
            && !self.prompts_dir.as_os_str().is_empty()
            && !self.default_timezone.trim().is_empty()
            && self.context_limit_tokens >= 4_096
            && self.max_output_tokens >= 256
            && self.request_timeout_ms >= 1_000
            && self.max_concurrent_root_runs != Some(0)
    }
}

/// Independent deployment configuration for the `llm-wiki-rs` engine (`[wiki]`).
#[derive(Clone, Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct WikiSectionConfig {
    pub enabled: bool,
    pub vault_dir: PathBuf,
    pub template: WikiTemplateId,
}

impl Default for WikiSectionConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            vault_dir: PathBuf::from("data/wiki-vault"),
            template: WikiTemplateId::Research,
        }
    }
}

impl WikiSectionConfig {
    pub fn valid(&self) -> bool {
        if !self.enabled {
            return true;
        }
        !self.vault_dir.as_os_str().is_empty()
    }
}
