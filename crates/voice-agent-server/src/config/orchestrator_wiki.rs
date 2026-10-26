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
    #[serde(default)]
    pub mcp: WikiMcpConfig,
}

impl Default for WikiSectionConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            vault_dir: PathBuf::from("data/wiki-vault"),
            template: WikiTemplateId::Research,
            mcp: WikiMcpConfig::default(),
        }
    }
}

impl WikiSectionConfig {
    pub fn valid(&self) -> bool {
        if !self.mcp.valid() {
            return false;
        }
        if !self.enabled && !self.mcp.enabled {
            return true;
        }
        !self.vault_dir.as_os_str().is_empty()
    }
}

/// Streamable HTTP MCP Server configuration for Knowledge/Wiki (`[wiki.mcp]`).
#[derive(Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct WikiMcpConfig {
    pub enabled: bool,
    pub path: String,
    pub allow_write: bool,
    pub token: String,
    pub read_token: String,
    pub write_token: String,
    pub max_concurrency: usize,
    pub call_timeout_ms: u64,
    pub max_result_chars: usize,
}

impl std::fmt::Debug for WikiMcpConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WikiMcpConfig")
            .field("enabled", &self.enabled)
            .field("path", &self.path)
            .field("allow_write", &self.allow_write)
            .field("token", &"[REDACTED]")
            .field("read_token", &"[REDACTED]")
            .field("write_token", &"[REDACTED]")
            .field("max_concurrency", &self.max_concurrency)
            .field("call_timeout_ms", &self.call_timeout_ms)
            .field("max_result_chars", &self.max_result_chars)
            .finish()
    }
}

impl Default for WikiMcpConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            path: "/mcp/knowledge".into(),
            allow_write: true,
            token: String::new(),
            read_token: String::new(),
            write_token: String::new(),
            max_concurrency: 16,
            call_timeout_ms: 30_000,
            max_result_chars: 12_000,
        }
    }
}

impl WikiMcpConfig {
    pub fn valid(&self) -> bool {
        if !(1..=64).contains(&self.max_concurrency)
            || !(1_000..=120_000).contains(&self.call_timeout_ms)
            || !(512..=64_000).contains(&self.max_result_chars)
            || !valid_mcp_path(&self.path)
        {
            return false;
        }
        if !self.enabled {
            return true;
        }
        let has_token = !self.token.is_empty();
        let has_read = !self.read_token.is_empty();
        let has_write = !self.write_token.is_empty();
        if !has_token && !has_read && !has_write {
            return false;
        }
        for candidate in [&self.token, &self.read_token, &self.write_token] {
            if !candidate.is_empty() && !valid_bearer_secret(candidate) {
                return false;
            }
        }
        if has_read && has_write && self.read_token == self.write_token {
            return false;
        }
        true
    }
}

fn valid_mcp_path(path: &str) -> bool {
    path.starts_with("/mcp/")
        && (6..=128).contains(&path.len())
        && !path.ends_with('/')
        && path != "/mcp/system"
        && path != "/mcp/vision/explain"
        && path
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'/' | b'-' | b'_'))
}

fn valid_bearer_secret(secret: &str) -> bool {
    (8..=512).contains(&secret.len())
        && secret.trim() == secret
        && secret.bytes().all(|b| b > 0x20 && b < 0x7f)
}
