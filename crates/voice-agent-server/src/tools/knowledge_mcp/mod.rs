//! Transport-independent Knowledge (LLM Wiki) MCP authority, schema sanitization, runtime settings,
//! token management, and tool dispatch.

mod settings;
mod staging;
#[cfg(test)]
mod tests;

pub(crate) use settings::{
    KnowledgeMcpAccess, KnowledgeMcpAdminError, KnowledgeMcpTokenEntry, KnowledgeMcpTokenSummary,
    UpdateKnowledgeMcpInput, persisted_settings_exist,
};

use crate::{config::WikiMcpConfig, providers::llm::ToolDefinition};
use llm_wiki::{AdaptiveReadBudget, WikiEngine, WikiError, is_read_only_wiki_tool};
use serde_json::{Value, json};
use settings::{
    CALL_TIMEOUT_MS_RANGE, CONCURRENCY_RANGE, MAX_TOKENS, PersistedKnowledgeMcpSettings,
    RESULT_CHARS_RANGE, SETTINGS_FILENAME, constant_time_eq, load_or_seed_settings,
    persist_settings, unix_now,
};
pub(crate) use staging::{
    add_to_sources_tool_definition, bound_result_chars, execute_add_to_sources,
    normalize_wiki_markdown_tables, normalize_wiki_tool_arguments, sanitize_schema_node,
};
use std::{
    path::{Path, PathBuf},
    sync::{
        Arc, RwLock,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio::sync::Notify;
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;

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

/// RAII concurrency guard enforcing the live `max_concurrency` limit on `/mcp/knowledge`.
pub(crate) struct KnowledgeConcurrencyPermit {
    active_requests: Arc<AtomicUsize>,
}

impl Drop for KnowledgeConcurrencyPermit {
    fn drop(&mut self) {
        self.active_requests.fetch_sub(1, Ordering::AcqRel);
    }
}

pub(crate) struct KnowledgeMcp {
    engine: Arc<WikiEngine>,
    autoflow_notify: Option<Arc<Notify>>,
    settings: RwLock<PersistedKnowledgeMcpSettings>,
    store_path: Option<PathBuf>,
    active_requests: Arc<AtomicUsize>,
    read_only_tools: Vec<ToolDefinition>,
    read_write_tools: Vec<ToolDefinition>,
}

impl KnowledgeMcp {
    #[cfg(test)]
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
            max_result_chars: max_result_chars
                .clamp(*RESULT_CHARS_RANGE.start(), *RESULT_CHARS_RANGE.end()),
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
        let settings = load_or_seed_settings(cfg, store_path.as_ref());

        Self {
            engine,
            autoflow_notify,
            settings: RwLock::new(settings),
            store_path,
            active_requests: Arc::new(AtomicUsize::new(0)),
            read_only_tools,
            read_write_tools,
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
            .clamp(*CONCURRENCY_RANGE.start(), *CONCURRENCY_RANGE.end())
    }

    pub(crate) fn try_acquire_permit(&self) -> Option<KnowledgeConcurrencyPermit> {
        let limit = self.max_concurrency();
        let mut current = self.active_requests.load(Ordering::Acquire);
        loop {
            if current >= limit {
                return None;
            }
            match self.active_requests.compare_exchange_weak(
                current,
                current + 1,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => {
                    return Some(KnowledgeConcurrencyPermit {
                        active_requests: Arc::clone(&self.active_requests),
                    });
                }
                Err(actual) => current = actual,
            }
        }
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
        let timeout = Duration::from_millis(
            guard
                .call_timeout_ms
                .clamp(*CALL_TIMEOUT_MS_RANGE.start(), *CALL_TIMEOUT_MS_RANGE.end()),
        );
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
        let ro_names: Vec<&str> = self
            .read_only_tools
            .iter()
            .map(|t| t.name.as_str())
            .collect();
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

        let token_summaries: Vec<KnowledgeMcpTokenSummary> = guard
            .tokens
            .iter()
            .map(KnowledgeMcpTokenEntry::to_summary)
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
            "tokens": token_summaries,
            "tools": tools_view,
            "read_only_tools_count": self.read_only_tools.len(),
            "read_write_tools_count": self.read_write_tools.len(),
            "wiki_status": wiki_status,
        })
    }

    pub(crate) fn update_settings(
        &self,
        input: UpdateKnowledgeMcpInput,
        header_revision: Option<i64>,
    ) -> Result<Value, KnowledgeMcpAdminError> {
        let mut guard = self.settings.write().expect("settings lock");
        if let Some(expected_rev) = header_revision.or(input.expected_revision)
            && expected_rev != guard.revision
        {
            return Err(KnowledgeMcpAdminError::RevisionConflict);
        }
        if let Some(mc) = input.max_concurrency {
            if !CONCURRENCY_RANGE.contains(&mc) {
                return Err(KnowledgeMcpAdminError::ValidationFailed);
            }
            guard.max_concurrency = mc;
        }
        if let Some(timeout) = input.call_timeout_ms {
            if !CALL_TIMEOUT_MS_RANGE.contains(&timeout) {
                return Err(KnowledgeMcpAdminError::ValidationFailed);
            }
            guard.call_timeout_ms = timeout;
        }
        if let Some(chars) = input.max_result_chars {
            if !RESULT_CHARS_RANGE.contains(&chars) {
                return Err(KnowledgeMcpAdminError::ValidationFailed);
            }
            guard.max_result_chars = chars;
        }
        guard.enabled = input.enabled;
        guard.allow_write = input.allow_write;
        guard.revision = guard.revision.saturating_add(1);
        persist_settings(self.store_path.as_ref(), &guard);
        drop(guard);
        Ok(self.admin_snapshot())
    }

    pub(crate) fn create_token(
        &self,
        name: String,
        access: KnowledgeMcpAccess,
        custom_token: Option<String>,
    ) -> Result<(KnowledgeMcpTokenEntry, Value), KnowledgeMcpAdminError> {
        let clean_name = name.trim();
        if clean_name.is_empty() || clean_name.len() > 128 {
            return Err(KnowledgeMcpAdminError::ValidationFailed);
        }
        let token_str = match custom_token
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            Some(custom) => {
                if custom.len() < 8 || custom.len() > 512 {
                    return Err(KnowledgeMcpAdminError::ValidationFailed);
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
        if guard.tokens.len() >= MAX_TOKENS {
            return Err(KnowledgeMcpAdminError::ValidationFailed);
        }
        guard.tokens.push(entry.clone());
        guard.revision = guard.revision.saturating_add(1);
        persist_settings(self.store_path.as_ref(), &guard);
        drop(guard);
        Ok((entry, self.admin_snapshot()))
    }

    pub(crate) fn delete_token(&self, id: &str) -> Result<Value, KnowledgeMcpAdminError> {
        let mut guard = self.settings.write().expect("settings lock");
        let before = guard.tokens.len();
        guard.tokens.retain(|t| t.id != id);
        if guard.tokens.len() == before {
            return Err(KnowledgeMcpAdminError::NotFound);
        }
        guard.revision = guard.revision.saturating_add(1);
        persist_settings(self.store_path.as_ref(), &guard);
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
        normalize_wiki_tool_arguments(name, &mut arguments);

        let max_chars = self
            .settings
            .read()
            .expect("settings lock")
            .max_result_chars
            .clamp(*RESULT_CHARS_RANGE.start(), *RESULT_CHARS_RANGE.end());
        let engine = Arc::clone(&self.engine);
        let autoflow_notify = self.autoflow_notify.clone();
        let tool_name = name.to_owned();

        let mut handle =
            tokio::task::spawn_blocking(move || -> Result<Value, KnowledgeToolError> {
                if tool_name == "add_to_sources" {
                    execute_add_to_sources(&engine, autoflow_notify.as_ref(), arguments)
                } else {
                    let budget = AdaptiveReadBudget {
                        default_lines: 160,
                        max_window_chars: max_chars.saturating_sub(1024).max(1024),
                        compact_index_max_chars: max_chars.saturating_sub(1024).max(1024),
                    };
                    let outcome =
                        engine.execute_llm_tool_with_budget(&tool_name, arguments, budget)?;
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

        Ok(bound_result_chars(raw_value, max_chars))
    }
}
