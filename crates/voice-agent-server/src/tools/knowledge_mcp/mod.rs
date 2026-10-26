//! Transport-independent Knowledge (LLM Wiki) MCP authority, schema sanitization, runtime settings,
//! token management, and tool dispatch.

use crate::{config::WikiMcpConfig, providers::llm::ToolDefinition};
use llm_wiki::{
    AdaptiveReadBudget, WikiEngine, WikiError, WriteTopicSourceRequest, is_read_only_wiki_tool,
    sanitize_topic_folder, slugify,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    path::{Path, PathBuf},
    sync::{Arc, RwLock},
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tokio::sync::Notify;
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;

const SETTINGS_FILENAME: &str = "knowledge_mcp_settings.json";

/// Allowed JSON Schema keywords accepted by `ExternalMcpClient` (`validate_external_tool_schema`).
const ALLOWED_SCHEMA_KEYWORDS: &[&str] = &[
    "type",
    "description",
    "properties",
    "required",
    "additionalProperties",
    "enum",
    "const",
    "items",
    "minimum",
    "maximum",
    "minLength",
    "maxLength",
];

const MIN_SOURCE_SUBSTANCE_CHARS: usize = 20;

const SECRET_MARKERS: &[&str] = &[
    "-----BEGIN PRIVATE KEY-----",
    "-----BEGIN RSA PRIVATE KEY-----",
    "-----BEGIN OPENSSH PRIVATE KEY-----",
    "-----BEGIN EC PRIVATE KEY-----",
    "sk-ant-api",
    "sk-proj-",
    "ghp_",
    "github_pat_",
    "AIzaSy",
];

const INJECTION_MARKERS: &[&str] = &[
    "ignore all previous instructions",
    "ignore previous instructions",
    "disregard system prompt",
    "disregard all prior instructions",
    "<|im_start|>system",
    "[inst] <<sys>>",
];

/// Access level resolved for an authenticated Knowledge MCP caller.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum KnowledgeMcpAccess {
    ReadOnly,
    ReadWrite,
}

impl KnowledgeMcpAccess {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::ReadOnly => "read_only",
            Self::ReadWrite => "read_write",
        }
    }

    pub(crate) fn from_str_opt(raw: &str) -> Option<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "read_only" | "readonly" | "read" => Some(Self::ReadOnly),
            "read_write" | "readwrite" | "write" | "full" => Some(Self::ReadWrite),
            _ => None,
        }
    }
}

/// One API token entry managed via the System MCP Web UI or seeded from `config.toml`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct KnowledgeMcpTokenEntry {
    pub id: String,
    pub name: String,
    pub token: String,
    pub access: KnowledgeMcpAccess,
    pub created_at: i64,
}

/// Persisted Knowledge MCP runtime configuration editable from the Admin Web UI.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct PersistedKnowledgeMcpSettings {
    pub revision: u64,
    pub enabled: bool,
    pub path: String,
    pub allow_write: bool,
    pub max_concurrency: usize,
    pub call_timeout_ms: u64,
    pub max_result_chars: usize,
    pub tokens: Vec<KnowledgeMcpTokenEntry>,
}

#[derive(Clone, Debug, Deserialize)]
pub(crate) struct UpdateKnowledgeMcpInput {
    pub enabled: bool,
    pub allow_write: bool,
    pub max_concurrency: Option<usize>,
    pub call_timeout_ms: Option<u64>,
    pub max_result_chars: Option<usize>,
}

/// Per-request execution context for Knowledge MCP tool calls.
#[derive(Clone)]
pub(crate) struct KnowledgeToolContext {
    pub access: KnowledgeMcpAccess,
    pub cancellation: CancellationToken,
    pub deadline: Instant,
}

#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub(crate) enum KnowledgeToolError {
    #[error("knowledge_tool_denied")]
    Denied,
    #[error("invalid_arguments: {0}")]
    InvalidArguments(String),
    #[error("knowledge_tool_error: {0}")]
    ExecutionFailed(String),
    #[error("knowledge_tool_unavailable")]
    Unavailable,
    #[error("knowledge_tool_cancelled")]
    Cancelled,
    #[error("knowledge_tool_timeout")]
    Timeout,
}

impl From<WikiError> for KnowledgeToolError {
    fn from(err: WikiError) -> Self {
        match err {
            WikiError::InvalidToolArguments { message, .. } => Self::InvalidArguments(message),
            WikiError::UnknownTool(_) => Self::Denied,
            other => Self::ExecutionFailed(other.to_string()),
        }
    }
}

pub(crate) fn persisted_settings_exist(store_dir: &Path) -> bool {
    store_dir.join(SETTINGS_FILENAME).is_file()
}

fn unix_now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (&x, &y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

pub(crate) struct KnowledgeMcp {
    engine: Arc<WikiEngine>,
    autoflow_notify: Option<Arc<Notify>>,
    settings: RwLock<PersistedKnowledgeMcpSettings>,
    store_path: Option<PathBuf>,
    read_only_tools: Vec<ToolDefinition>,
    read_write_tools: Vec<ToolDefinition>,
}

impl KnowledgeMcp {
    pub(crate) fn new(
        engine: Arc<WikiEngine>,
        autoflow_notify: Option<Arc<Notify>>,
        allow_write: bool,
        max_result_chars: usize,
    ) -> Self {
        let cfg = WikiMcpConfig {
            enabled: true,
            path: "/mcp/knowledge".to_string(),
            allow_write,
            token: String::new(),
            read_token: String::new(),
            write_token: String::new(),
            max_concurrency: 16,
            call_timeout_ms: 30_000,
            max_result_chars: max_result_chars.max(512),
        };
        Self::from_config(engine, autoflow_notify, &cfg, None)
    }

    pub(crate) fn from_config(
        engine: Arc<WikiEngine>,
        autoflow_notify: Option<Arc<Notify>>,
        cfg: &WikiMcpConfig,
        store_dir: Option<&Path>,
    ) -> Self {
        let specs = engine.llm_tool_definitions();
        let mut read_only_tools = Vec::new();
        let mut read_write_tools = Vec::new();

        for spec in specs {
            let mut parameters = spec.parameters;
            sanitize_schema_node(&mut parameters);
            let def = ToolDefinition {
                name: spec.name.clone(),
                description: spec.description,
                parameters,
            };
            if is_read_only_wiki_tool(&spec.name) {
                read_only_tools.push(def.clone());
            }
            read_write_tools.push(def);
        }

        let mut add_sources = add_to_sources_tool_definition();
        sanitize_schema_node(&mut add_sources.parameters);
        read_write_tools.push(add_sources);

        let store_path = store_dir.map(|d| d.join(SETTINGS_FILENAME));
        let mut settings = store_path
            .as_ref()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .and_then(|raw| serde_json::from_str::<PersistedKnowledgeMcpSettings>(&raw).ok())
            .unwrap_or_else(|| seed_settings_from_config(cfg));

        // Always ensure any static tokens configured in `config.toml` / env vars are present.
        merge_static_tokens(&mut settings.tokens, cfg);

        Self {
            engine,
            autoflow_notify,
            settings: RwLock::new(settings),
            store_path,
            read_only_tools,
            read_write_tools,
        }
    }

    fn persist_locked(&self, settings: &PersistedKnowledgeMcpSettings) {
        let Some(ref path) = self.store_path else {
            return;
        };
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Ok(serialized) = serde_json::to_string_pretty(settings) {
            let _ = std::fs::write(path, serialized);
        }
    }

    pub(crate) fn is_enabled(&self) -> bool {
        self.settings.read().expect("settings lock").enabled
    }

    pub(crate) fn max_concurrency(&self) -> usize {
        self.settings
            .read()
            .expect("settings lock")
            .max_concurrency
            .clamp(1, 64)
    }

    pub(crate) fn call_timeout(&self) -> Duration {
        let ms = self
            .settings
            .read()
            .expect("settings lock")
            .call_timeout_ms
            .clamp(1_000, 120_000);
        Duration::from_millis(ms)
    }

    pub(crate) fn verify_token(&self, presented: &str) -> Option<KnowledgeToolContext> {
        let presented = presented.trim();
        if presented.is_empty() || presented.len() > 4096 {
            return None;
        }
        let guard = self.settings.read().expect("settings lock");
        if !guard.enabled {
            return None;
        }

        let mut matched_access: Option<KnowledgeMcpAccess> = None;
        for entry in &guard.tokens {
            if constant_time_eq(presented.as_bytes(), entry.token.as_bytes()) {
                matched_access = Some(match entry.access {
                    KnowledgeMcpAccess::ReadWrite if guard.allow_write => {
                        KnowledgeMcpAccess::ReadWrite
                    }
                    _ => KnowledgeMcpAccess::ReadOnly,
                });
                if matched_access == Some(KnowledgeMcpAccess::ReadWrite) {
                    break;
                }
            }
        }

        let access = matched_access?;
        let timeout = Duration::from_millis(guard.call_timeout_ms.clamp(1_000, 120_000));
        Some(KnowledgeToolContext {
            access,
            cancellation: CancellationToken::new(),
            deadline: Instant::now() + timeout,
        })
    }

    pub(crate) fn effective_write_allowed(&self, ctx: &KnowledgeToolContext) -> bool {
        let allow_write = self.settings.read().expect("settings lock").allow_write;
        allow_write && ctx.access == KnowledgeMcpAccess::ReadWrite
    }

    pub(crate) fn list_tools(&self, ctx: &KnowledgeToolContext) -> Vec<ToolDefinition> {
        if self.effective_write_allowed(ctx) {
            self.read_write_tools.clone()
        } else {
            self.read_only_tools.clone()
        }
    }

    fn is_tool_allowed(&self, ctx: &KnowledgeToolContext, name: &str) -> bool {
        if self.effective_write_allowed(ctx) {
            self.read_write_tools.iter().any(|t| t.name == name)
        } else {
            self.read_only_tools.iter().any(|t| t.name == name)
        }
    }

    pub(crate) fn admin_snapshot(&self) -> Value {
        let guard = self.settings.read().expect("settings lock").clone();
        let ro_names: Vec<&str> = self.read_only_tools.iter().map(|t| t.name.as_str()).collect();
        let tools_view: Vec<Value> = self
            .read_write_tools
            .iter()
            .map(|t| {
                let is_ro = ro_names.contains(&t.name.as_str());
                json!({
                    "name": t.name,
                    "description": t.description,
                    "access": if is_ro { "read_only" } else { "read_write" },
                })
            })
            .collect();

        let wiki_status = self
            .engine
            .status()
            .ok()
            .and_then(|s| serde_json::to_value(s).ok())
            .unwrap_or(Value::Null);

        json!({
            "revision": guard.revision,
            "enabled": guard.enabled,
            "path": guard.path,
            "allow_write": guard.allow_write,
            "max_concurrency": guard.max_concurrency,
            "call_timeout_ms": guard.call_timeout_ms,
            "max_result_chars": guard.max_result_chars,
            "tokens": guard.tokens,
            "tools": tools_view,
            "read_only_tools_count": self.read_only_tools.len(),
            "read_write_tools_count": self.read_write_tools.len(),
            "wiki_status": wiki_status,
        })
    }

    pub(crate) fn update_settings(&self, input: UpdateKnowledgeMcpInput) -> Result<Value, String> {
        let mut guard = self.settings.write().expect("settings lock");
        if let Some(mc) = input.max_concurrency {
            if !(1..=64).contains(&mc) {
                return Err("max_concurrency must be between 1 and 64".into());
            }
            guard.max_concurrency = mc;
        }
        if let Some(timeout) = input.call_timeout_ms {
            if !(1_000..=120_000).contains(&timeout) {
                return Err("call_timeout_ms must be between 1000 and 120000".into());
            }
            guard.call_timeout_ms = timeout;
        }
        if let Some(chars) = input.max_result_chars {
            if !(512..=65_536).contains(&chars) {
                return Err("max_result_chars must be between 512 and 65536".into());
            }
            guard.max_result_chars = chars;
        }
        guard.enabled = input.enabled;
        guard.allow_write = input.allow_write;
        guard.revision = guard.revision.saturating_add(1);
        self.persist_locked(&guard);
        drop(guard);
        Ok(self.admin_snapshot())
    }

    pub(crate) fn create_token(
        &self,
        name: String,
        access: KnowledgeMcpAccess,
        custom_token: Option<String>,
    ) -> Result<(KnowledgeMcpTokenEntry, Value), String> {
        let clean_name = name.trim();
        if clean_name.is_empty() || clean_name.len() > 128 {
            return Err("token name must be 1..=128 characters".into());
        }
        let token_str = match custom_token.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
            Some(custom) => {
                if custom.len() < 8 || custom.len() > 512 {
                    return Err("custom token must be between 8 and 512 characters".into());
                }
                custom.to_owned()
            }
            None => format!(
                "kmcp_{}{}",
                uuid::Uuid::new_v4().simple(),
                uuid::Uuid::new_v4().simple()
            ),
        };

        let entry = KnowledgeMcpTokenEntry {
            id: uuid::Uuid::new_v4().to_string(),
            name: clean_name.to_owned(),
            token: token_str,
            access,
            created_at: unix_now(),
        };

        let mut guard = self.settings.write().expect("settings lock");
        if guard.tokens.len() >= 64 {
            return Err("maximum of 64 tokens reached".into());
        }
        guard.tokens.push(entry.clone());
        guard.revision = guard.revision.saturating_add(1);
        self.persist_locked(&guard);
        drop(guard);
        Ok((entry, self.admin_snapshot()))
    }

    pub(crate) fn delete_token(&self, id: &str) -> Result<Value, String> {
        let mut guard = self.settings.write().expect("settings lock");
        let before = guard.tokens.len();
        guard.tokens.retain(|t| t.id != id);
        if guard.tokens.len() == before {
            return Err("token_not_found".into());
        }
        guard.revision = guard.revision.saturating_add(1);
        self.persist_locked(&guard);
        drop(guard);
        Ok(self.admin_snapshot())
    }

    pub(crate) async fn call_tool(
        &self,
        ctx: &KnowledgeToolContext,
        name: &str,
        mut arguments: Value,
    ) -> Result<Value, KnowledgeToolError> {
        if !self.is_tool_allowed(ctx, name) {
            return Err(KnowledgeToolError::Denied);
        }
        if ctx.cancellation.is_cancelled() {
            return Err(KnowledgeToolError::Cancelled);
        }
        if Instant::now() >= ctx.deadline {
            return Err(KnowledgeToolError::Timeout);
        }
        if !arguments.is_object() {
            return Err(KnowledgeToolError::InvalidArguments(
                "tool arguments must be a JSON object".into(),
            ));
        }

        if name == "wiki_tree"
            && let Some(obj) = arguments.as_object_mut()
            && let Some(sub) = obj.get("sub_path").and_then(Value::as_str)
        {
            let trimmed = sub.trim().trim_matches('/');
            if trimmed.is_empty() || trimmed == "." {
                obj.remove("sub_path");
            }
        }

        let max_chars = self
            .settings
            .read()
            .expect("settings lock")
            .max_result_chars
            .max(512);
        let engine = Arc::clone(&self.engine);
        let autoflow_notify = self.autoflow_notify.clone();
        let tool_name = name.to_owned();

        let mut handle = tokio::task::spawn_blocking(move || -> Result<Value, KnowledgeToolError> {
            if tool_name == "add_to_sources" {
                execute_add_to_sources(&engine, autoflow_notify.as_ref(), arguments)
            } else {
                let budget = AdaptiveReadBudget {
                    default_lines: 160,
                    max_window_chars: max_chars.saturating_sub(1024).max(1024),
                    compact_index_max_chars: max_chars.saturating_sub(1024).max(1024),
                };
                let outcome = engine.execute_llm_tool_with_budget(&tool_name, arguments, budget)?;
                Ok(json!({
                    "tool": tool_name,
                    "content": outcome.content,
                    "metadata": outcome.metadata,
                }))
            }
        });

        let raw_value = tokio::select! {
            biased;
            _ = ctx.cancellation.cancelled() => {
                handle.abort();
                return Err(KnowledgeToolError::Cancelled);
            }
            _ = tokio::time::sleep_until(ctx.deadline) => {
                handle.abort();
                return Err(KnowledgeToolError::Timeout);
            }
            joined = &mut handle => {
                match joined {
                    Ok(res) => res?,
                    Err(_) => return Err(KnowledgeToolError::Unavailable),
                }
            }
        };

        Ok(bound_result_value(raw_value, max_chars))
    }
}

fn seed_settings_from_config(cfg: &WikiMcpConfig) -> PersistedKnowledgeMcpSettings {
    let mut tokens = Vec::new();
    merge_static_tokens(&mut tokens, cfg);
    PersistedKnowledgeMcpSettings {
        revision: 1,
        enabled: cfg.enabled,
        path: if cfg.path.trim().is_empty() {
            "/mcp/knowledge".to_string()
        } else {
            cfg.path.trim().to_string()
        },
        allow_write: cfg.allow_write,
        max_concurrency: cfg.max_concurrency.max(1),
        call_timeout_ms: cfg.call_timeout_ms.max(1_000),
        max_result_chars: cfg.max_result_chars.max(512),
        tokens,
    }
}

fn merge_static_tokens(tokens: &mut Vec<KnowledgeMcpTokenEntry>, cfg: &WikiMcpConfig) {
    let now = unix_now();
    let mut push_unique = |id: &str, name: &str, raw: &str, access: KnowledgeMcpAccess| {
        let clean = raw.trim();
        if clean.is_empty() {
            return;
        }
        if !tokens.iter().any(|t| t.token == clean) {
            tokens.push(KnowledgeMcpTokenEntry {
                id: id.to_string(),
                name: name.to_string(),
                token: clean.to_string(),
                access,
                created_at: now,
            });
        }
    };
    push_unique(
        "config-default-token",
        "Default Config Token",
        &cfg.token,
        KnowledgeMcpAccess::ReadWrite,
    );
    push_unique(
        "config-write-token",
        "Config Read-Write Token",
        &cfg.write_token,
        KnowledgeMcpAccess::ReadWrite,
    );
    push_unique(
        "config-read-token",
        "Config Read-Only Token",
        &cfg.read_token,
        KnowledgeMcpAccess::ReadOnly,
    );
}

/// Recursively strips JSON Schema keywords not in `ALLOWED_SCHEMA_KEYWORDS` (such as `"default"`)
/// so that every tool schema published by `KnowledgeMcp` passes `validate_external_tool_schema`.
fn sanitize_schema_node(node: &mut Value) {
    let Some(obj) = node.as_object_mut() else {
        return;
    };
    obj.retain(|k, _| ALLOWED_SCHEMA_KEYWORDS.contains(&k.as_str()));
    if let Some(Value::Object(props)) = obj.get_mut("properties") {
        for prop_schema in props.values_mut() {
            sanitize_schema_node(prop_schema);
        }
    }
    if let Some(items) = obj.get_mut("items") {
        sanitize_schema_node(items);
    }
}

fn add_to_sources_tool_definition() -> ToolDefinition {
    ToolDefinition {
        name: "add_to_sources".to_string(),
        description: "Stage a raw or synthesized Markdown source into `sources/<domain>/<topic>/<slug>.md` and trigger Flow 2 Wiki compilation (`autoflow_notify`). Requires `topic_folder`, `title`, and `markdown_body` (>= 20 non-whitespace chars, free of secrets/prompt injection).".to_string(),
        parameters: json!({
            "type": "object",
            "required": ["topic_folder", "title", "markdown_body"],
            "properties": {
                "topic_folder": {
                    "type": "string",
                    "description": "Two-level kebab-case folder '<domain>/<topic>' inside sources/ (e.g. 'software-architecture/agent-tasks')."
                },
                "file_stem": {
                    "type": "string",
                    "description": "Optional kebab-case filename stem without .md."
                },
                "title": {
                    "type": "string",
                    "description": "Document title."
                },
                "markdown_body": {
                    "type": "string",
                    "description": "Self-contained Markdown body (>= 20 non-whitespace chars, free of secrets or prompt injection)."
                },
                "tags": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Optional hierarchical tags in kebab-case."
                },
                "provenance": {
                    "type": "string",
                    "enum": ["user", "source", "verified_synthesis", "assistant"],
                    "description": "Origin of the knowledge: 'user', 'source', 'verified_synthesis' (default), or 'assistant' (blocked if unverified)."
                },
                "target_hint": {
                    "type": "string",
                    "description": "Optional hint for Flow 2 (`wiki_ingestor`) indicating existing Wiki pages to update or cross-link."
                },
                "conflict_with": {
                    "type": "string",
                    "description": "Optional existing Wiki page path/slug that this source contradicts so Flow 2 preserves both perspectives."
                },
                "open_questions": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Optional list of unresolved questions or unverified hypotheses."
                }
            },
            "additionalProperties": false
        }),
    }
}

fn validate_source_content_safety(title: &str, body: &str) -> Result<(), KnowledgeToolError> {
    let non_ws_chars = body.chars().filter(|c| !c.is_whitespace()).count();
    if non_ws_chars < MIN_SOURCE_SUBSTANCE_CHARS {
        return Err(KnowledgeToolError::InvalidArguments(format!(
            "markdown_body is too short ({non_ws_chars} non-whitespace chars; minimum is {MIN_SOURCE_SUBSTANCE_CHARS})"
        )));
    }

    let combined = format!("{title}\n{body}");
    for marker in SECRET_MARKERS {
        if combined.contains(marker) {
            return Err(KnowledgeToolError::InvalidArguments(format!(
                "source content rejected: detected secret/credential pattern '{marker}'"
            )));
        }
    }

    let lowered = combined.to_ascii_lowercase();
    for marker in INJECTION_MARKERS {
        if lowered.contains(marker) {
            return Err(KnowledgeToolError::InvalidArguments(format!(
                "source content rejected: detected prompt-injection directive '{marker}'"
            )));
        }
    }

    Ok(())
}

fn normalize_and_validate_provenance(
    raw: Option<&str>,
) -> Result<&'static str, KnowledgeToolError> {
    let val = raw.map(str::trim).filter(|s| !s.is_empty());
    match val {
        None => Ok("verified_synthesis"),
        Some(p) => {
            let lower = p.to_ascii_lowercase();
            match lower.as_str() {
                "user" => Ok("user"),
                "source" => Ok("source"),
                "verified_synthesis" | "verified" | "mixed" => Ok("verified_synthesis"),
                "assistant" | "assistant_only" | "unverified" => {
                    Err(KnowledgeToolError::InvalidArguments(
                        "cannot stage assistant-only unverified speculation into `sources/`".into(),
                    ))
                }
                _ => Err(KnowledgeToolError::InvalidArguments(format!(
                    "invalid provenance '{p}': expected 'user', 'source', 'verified_synthesis', or 'assistant'"
                ))),
            }
        }
    }
}

fn execute_add_to_sources(
    engine: &WikiEngine,
    autoflow_notify: Option<&Arc<Notify>>,
    args: Value,
) -> Result<Value, KnowledgeToolError> {
    let raw_topic = args
        .get("topic_folder")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| {
            KnowledgeToolError::InvalidArguments("missing required `topic_folder`".into())
        })?;
    let title = args
        .get("title")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| KnowledgeToolError::InvalidArguments("missing required `title`".into()))?;
    let body = args
        .get("markdown_body")
        .and_then(Value::as_str)
        .filter(|s| !s.trim().is_empty())
        .ok_or_else(|| {
            KnowledgeToolError::InvalidArguments("missing required `markdown_body`".into())
        })?;

    let provenance =
        normalize_and_validate_provenance(args.get("provenance").and_then(Value::as_str))?;
    validate_source_content_safety(title, body)?;

    let topic_folder = sanitize_topic_folder(raw_topic);
    let custom_stem = args
        .get("file_stem")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty());
    let file_stem = custom_stem
        .map(|s| slugify(s, 64))
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| slugify(title, 64));

    let tags: Vec<String> = args
        .get("tags")
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(Value::as_str)
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect()
        })
        .unwrap_or_default();

    let target_hint = args
        .get("target_hint")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(ToString::to_string);
    let conflict_with = args
        .get("conflict_with")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(ToString::to_string);
    let open_questions: Vec<String> = args
        .get("open_questions")
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(Value::as_str)
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect()
        })
        .unwrap_or_default();

    let mut extra_frontmatter = vec![
        (
            "provenance".to_string(),
            Value::String(provenance.to_string()),
        ),
        (
            "origin".to_string(),
            Value::String("knowledge-mcp-staged".to_string()),
        ),
    ];
    let mut header_bullets = vec![("Provenance".to_string(), format!("`{provenance}`"))];

    if let Some(ref hint) = target_hint {
        extra_frontmatter.push(("target_hint".to_string(), Value::String(hint.clone())));
        header_bullets.push(("Target Wiki Hint".to_string(), hint.clone()));
    }
    if let Some(ref conflict) = conflict_with {
        extra_frontmatter.push(("conflict_with".to_string(), Value::String(conflict.clone())));
        header_bullets.push((
            "Conflict / Non-Destructive Merge Target".to_string(),
            conflict.clone(),
        ));
    }

    let mut final_body = body.trim_end().to_string();
    if conflict_with.is_some() || !open_questions.is_empty() {
        final_body.push_str("\n\n## Mâu thuẫn & Câu hỏi mở (Conflict & Open Questions)\n\n");
        final_body.push_str("> [!WARNING] Contradiction / Caveat\n");
        if let Some(ref conflict) = conflict_with {
            final_body.push_str(&format!(
                "> - **Mâu thuẫn với trang hiện có (`{conflict}`)**: Khi biên dịch sang `wiki/`, giữ nguyên cả quan điểm cũ lẫn phát hiện mới từ nguồn này thay vì ghi đè.\n"
            ));
        }
        for q in &open_questions {
            final_body.push_str(&format!("> - **Câu hỏi mở (Chưa xác minh)**: {q}\n"));
        }
    }

    let written = engine.write_topic_source(WriteTopicSourceRequest {
        topic_folder,
        file_stem: Some(file_stem),
        title: title.to_string(),
        tags,
        extra_frontmatter,
        header_bullets,
        body_heading: None,
        raw_body: final_body,
        preview_chars: Some(500),
    })?;

    let autoflow_triggered = if let Some(notify) = autoflow_notify {
        notify.notify_one();
        true
    } else {
        false
    };

    Ok(json!({
        "tool": "add_to_sources",
        "content": written.agent_output,
        "metadata": {
            "source_rel_path": written.info.scoped_path,
            "vault_rel_path": written.info.rel_path,
            "docid": written.info.docid,
            "provenance": provenance,
            "target_hint": target_hint,
            "conflict_with": conflict_with,
            "open_questions_count": open_questions.len(),
            "autoflow_triggered": autoflow_triggered,
        }
    }))
}

fn bound_result_value(mut value: Value, max_bytes: usize) -> Value {
    let serialized = serde_json::to_string(&value).unwrap_or_default();
    if serialized.len() <= max_bytes {
        return value;
    }

    // First try dropping bulky `metadata` details while keeping `content`.
    if let Some(obj) = value.as_object_mut() {
        obj.insert("truncated".to_string(), Value::Bool(true));
        if let Some(meta) = obj.get("metadata").cloned() {
            let slim_meta = match meta {
                Value::Object(mut m) => {
                    m.remove("results");
                    m.remove("tree");
                    m.remove("entries");
                    m.remove("backlinks");
                    m.remove("outgoing_links");
                    Value::Object(m)
                }
                _ => Value::Null,
            };
            obj.insert("metadata".to_string(), slim_meta);
        }
    }

    let after_slim = serde_json::to_string(&value).unwrap_or_default();
    if after_slim.len() <= max_bytes {
        return value;
    }

    // Truncate `content` string so the entire JSON object stays within `max_bytes`.
    if let Some(obj) = value.as_object_mut()
        && let Some(Value::String(content)) = obj.get_mut("content")
    {
        let budget = max_bytes.saturating_sub(256);
        let mut cut = budget.min(content.len());
        while cut > 0 && !content.is_char_boundary(cut) {
            cut -= 1;
        }
        content.truncate(cut);
        content.push_str("\n...[truncated]");
    }

    let after_content = serde_json::to_string(&value).unwrap_or_default();
    if after_content.len() <= max_bytes {
        return value;
    }

    json!({
        "truncated": true,
        "content": "...[truncated]",
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::external_mcp::registry::{
        ExternalToolCatalog, validate_external_tool_schema,
    };

    struct TempVaultDir(PathBuf);

    impl Drop for TempVaultDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn test_engine() -> (TempVaultDir, Arc<WikiEngine>) {
        let path = std::env::temp_dir().join(format!(
            "voice-agent-knowledge-mcp-test-{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&path).expect("create temp vault dir");
        let engine = WikiEngine::open(&path).expect("open wiki engine");
        (TempVaultDir(path), Arc::new(engine))
    }

    fn test_ctx(access: KnowledgeMcpAccess) -> KnowledgeToolContext {
        KnowledgeToolContext {
            access,
            cancellation: CancellationToken::new(),
            deadline: Instant::now() + Duration::from_secs(10),
        }
    }

    #[test]
    fn all_published_schemas_pass_external_mcp_schema_validation() {
        let (_dir, engine) = test_engine();
        let expected_ro = engine
            .llm_tool_definitions()
            .iter()
            .filter(|s| is_read_only_wiki_tool(&s.name))
            .count();
        let expected_rw = engine.llm_tool_definitions().len() + 1;
        let mcp = KnowledgeMcp::new(engine, None, true, 12_000);

        let ro_tools = mcp.list_tools(&test_ctx(KnowledgeMcpAccess::ReadOnly));
        assert_eq!(ro_tools.len(), expected_ro);
        for tool in &ro_tools {
            validate_external_tool_schema(&tool.parameters)
                .unwrap_or_else(|e| panic!("RO tool {} failed schema validation: {e}", tool.name));
        }

        let rw_tools = mcp.list_tools(&test_ctx(KnowledgeMcpAccess::ReadWrite));
        assert_eq!(rw_tools.len(), expected_rw);
        assert!(rw_tools.iter().any(|t| t.name == "add_to_sources"));
        for tool in &rw_tools {
            validate_external_tool_schema(&tool.parameters)
                .unwrap_or_else(|e| panic!("RW tool {} failed schema validation: {e}", tool.name));
        }

        let discovered: Vec<(String, String, Value)> = rw_tools
            .iter()
            .map(|t| (t.name.clone(), t.description.clone(), t.parameters.clone()))
            .collect();
        let catalog = ExternalToolCatalog::publish("external.knowledge", discovered)
            .expect("catalog should be accepted by ExternalToolCatalog");
        assert_eq!(catalog.tools().len(), expected_rw);
        assert_eq!(catalog.dropped(), 0);
    }

    #[tokio::test]
    async fn read_only_caller_cannot_invoke_write_tools() {
        let (_dir, engine) = test_engine();
        let mcp = KnowledgeMcp::new(engine, None, true, 12_000);
        let ro_ctx = test_ctx(KnowledgeMcpAccess::ReadOnly);

        let status = mcp
            .call_tool(&ro_ctx, "wiki_status", json!({}))
            .await
            .expect("wiki_status should succeed for ReadOnly");
        assert_eq!(status.get("tool").and_then(Value::as_str), Some("wiki_status"));

        let write_err = mcp
            .call_tool(
                &ro_ctx,
                "wiki_write_file",
                json!({
                    "scope": "wiki",
                    "rel_path": "entities/rust.md",
                    "content": "# Rust\n\nSystems programming language.\n"
                }),
            )
            .await
            .expect_err("ReadOnly caller must be denied wiki_write_file");
        assert_eq!(write_err, KnowledgeToolError::Denied);
    }

    #[tokio::test]
    async fn read_write_caller_can_write_query_and_stage_sources() {
        let (_dir, engine) = test_engine();
        let notify = Arc::new(Notify::new());
        let mcp = KnowledgeMcp::new(engine, Some(Arc::clone(&notify)), true, 12_000);
        let rw_ctx = test_ctx(KnowledgeMcpAccess::ReadWrite);

        let write_res = mcp
            .call_tool(
                &rw_ctx,
                "wiki_write_file",
                json!({
                    "scope": "wiki",
                    "rel_path": "entities/voice-agent.md",
                    "content": "---\ntitle: Voice Agent\ntags:\n  - voice\n---\n\n# Voice Agent\n\nConnects to [[knowledge-mcp]] over Streamable HTTP.\n"
                }),
            )
            .await
            .expect("wiki_write_file should succeed for ReadWrite");
        assert!(
            write_res
                .get("content")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .contains("entities/voice-agent.md")
        );

        let get_res = mcp
            .call_tool(
                &rw_ctx,
                "wiki_get",
                json!({ "file": "wiki/entities/voice-agent.md" }),
            )
            .await
            .expect("wiki_get should return written page");
        assert!(
            get_res
                .get("content")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .contains("Streamable HTTP")
        );

        let stage_res = mcp
            .call_tool(
                &rw_ctx,
                "add_to_sources",
                json!({
                    "topic_folder": "architecture/mcp",
                    "title": "Knowledge MCP Design Notes",
                    "markdown_body": "Knowledge MCP exposes WikiEngine tools over /mcp/knowledge with Bearer token authentication and schema sanitization.",
                    "provenance": "user"
                }),
            )
            .await
            .expect("add_to_sources should succeed for ReadWrite");
        assert_eq!(
            stage_res
                .pointer("/metadata/autoflow_triggered")
                .and_then(Value::as_bool),
            Some(true)
        );
    }

    #[test]
    fn runtime_token_lifecycle_and_settings_update() {
        let (dir, engine) = test_engine();
        let cfg = WikiMcpConfig::default();
        let mcp = KnowledgeMcp::from_config(engine, None, &cfg, Some(&dir.0));
        assert!(!mcp.is_enabled());

        let snap = mcp
            .update_settings(UpdateKnowledgeMcpInput {
                enabled: true,
                allow_write: true,
                max_concurrency: Some(8),
                call_timeout_ms: Some(15_000),
                max_result_chars: Some(8_000),
            })
            .expect("update settings");
        assert_eq!(snap["enabled"], Value::Bool(true));

        let (entry, _) = mcp
            .create_token("Web Client".into(), KnowledgeMcpAccess::ReadOnly, None)
            .expect("create token");
        assert!(entry.token.starts_with("kmcp_"));

        let verified = mcp
            .verify_token(&entry.token)
            .expect("verify newly created token");
        assert_eq!(verified.access, KnowledgeMcpAccess::ReadOnly);

        mcp.delete_token(&entry.id).expect("delete token");
        assert!(mcp.verify_token(&entry.token).is_none());
    }
}
