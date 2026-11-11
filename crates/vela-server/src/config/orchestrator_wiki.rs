use llm_wiki::WikiTemplateId;
use serde::Deserialize;
use std::{fs, path::PathBuf};

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
    #[serde(default)]
    pub sync: WikiSyncConfig,
}

impl Default for WikiSectionConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            vault_dir: PathBuf::from("data/wiki-vault"),
            template: WikiTemplateId::Research,
            mcp: WikiMcpConfig::default(),
            sync: WikiSyncConfig::default(),
        }
    }
}

impl WikiSectionConfig {
    pub fn valid(&self) -> bool {
        if !self.mcp.valid() {
            return false;
        }
        if !self.enabled && !self.mcp.enabled && !self.sync.enabled {
            return true;
        }
        !self.vault_dir.as_os_str().is_empty() && self.sync.valid()
    }
}

/// Git-backed vault replication configuration (`[wiki.sync]`).
#[derive(Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct WikiSyncConfig {
    pub enabled: bool,
    pub remote_url: String,
    pub branch: String,
    pub ssh_key_path: PathBuf,
    pub known_hosts_path: PathBuf,
    pub author_name: String,
    pub author_email: String,
    pub interval_min: u64,
    pub debounce_sec: u64,
    pub max_file_mb: u64,
}

impl std::fmt::Debug for WikiSyncConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WikiSyncConfig")
            .field("enabled", &self.enabled)
            .field("remote_url", &redact_remote(&self.remote_url))
            .field("branch", &self.branch)
            .field("ssh_key_path", &self.ssh_key_path)
            .field("known_hosts_path", &self.known_hosts_path)
            .field("author_name", &self.author_name)
            .field("author_email", &self.author_email)
            .field("interval_min", &self.interval_min)
            .field("debounce_sec", &self.debounce_sec)
            .field("max_file_mb", &self.max_file_mb)
            .finish()
    }
}

impl Default for WikiSyncConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            remote_url: String::new(),
            branch: "main".into(),
            ssh_key_path: PathBuf::from("/app/docker/wiki-deploy-key"),
            known_hosts_path: PathBuf::from("/app/docker/github-known-hosts"),
            author_name: "Vela Server".into(),
            author_email: "vela@localhost".into(),
            interval_min: 5,
            debounce_sec: 30,
            max_file_mb: 50,
        }
    }
}

impl WikiSyncConfig {
    pub fn valid(&self) -> bool {
        if !(1..=10_080).contains(&self.interval_min)
            || !(1..=3_600).contains(&self.debounce_sec)
            || !(1..=1_024).contains(&self.max_file_mb)
            || self.branch.trim().is_empty()
            || self.branch.starts_with('-')
            || self.branch.chars().any(char::is_control)
        {
            return false;
        }
        if !self.enabled {
            return true;
        }
        if self.remote_url.trim().is_empty()
            || remote_has_credentials(&self.remote_url)
            || self.author_name.trim().is_empty()
            || self.author_email.trim().is_empty()
        {
            return false;
        }
        let Ok(key_metadata) = fs::metadata(&self.ssh_key_path) else {
            return false;
        };
        let Ok(known_hosts_metadata) = fs::metadata(&self.known_hosts_path) else {
            return false;
        };
        if !key_metadata.is_file() || !known_hosts_metadata.is_file() {
            return false;
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if key_metadata.permissions().mode() & 0o077 != 0 {
                return false;
            }
        }
        true
    }

    pub(crate) fn to_core_config(&self) -> wiki_core::SyncConfig {
        wiki_core::SyncConfig {
            remote_url: Some(self.remote_url.clone()),
            branch: self.branch.clone(),
            author: Some(wiki_core::GitAuthor {
                name: self.author_name.clone(),
                email: self.author_email.clone(),
            }),
            auth: wiki_core::GitAuth::SshKey {
                key: self.ssh_key_path.clone(),
                known_hosts: self.known_hosts_path.clone(),
            },
            max_file_bytes: self.max_file_mb.saturating_mul(1024 * 1024),
            command_timeout: std::time::Duration::from_secs(60),
        }
    }
}

fn remote_has_credentials(remote: &str) -> bool {
    let Some((scheme, authority_and_path)) = remote.split_once("://") else {
        // GitHub's SCP-style SSH URL (git@github.com:owner/repo.git) is not a password URL.
        return false;
    };
    let authority = authority_and_path.split('/').next().unwrap_or_default();
    let Some((user, _host)) = authority.rsplit_once('@') else {
        return false;
    };
    if user.is_empty() {
        return true;
    }
    scheme.eq_ignore_ascii_case("http")
        || scheme.eq_ignore_ascii_case("https")
        || user.contains(':')
        || !scheme.eq_ignore_ascii_case("ssh")
        || user != "git"
}

fn redact_remote(remote: &str) -> String {
    let Some((scheme, rest)) = remote.split_once("://") else {
        return remote.into();
    };
    let Some((_, host_and_path)) = rest.rsplit_once('@') else {
        return remote.into();
    };
    format!("{scheme}://[redacted]@{host_and_path}")
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
