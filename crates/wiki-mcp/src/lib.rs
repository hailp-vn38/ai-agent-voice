use llm_wiki::{is_read_only_wiki_tool, tools::definitions::build_llm_tool_definitions};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use std::time::Duration;
use thiserror::Error;
use tokio_util::sync::CancellationToken;
use wiki_core::{Change, DocPath, IngestSourceDetails, Origin, ReadWindow, Scope, Vault};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Access {
    ReadOnly,
    ReadWrite,
}

#[derive(Debug, Clone)]
pub struct CallCtx {
    pub access: Access,
    pub cancel: CancellationToken,
    pub deadline: tokio::time::Instant,
}

#[derive(Debug, Clone, Copy)]
pub struct Policy {
    pub allow_write: bool,
    pub max_concurrency: usize,
    pub call_timeout: Duration,
    pub max_result_chars: usize,
}

impl Default for Policy {
    fn default() -> Self {
        Self {
            allow_write: true,
            max_concurrency: 16,
            call_timeout: Duration::from_secs(30),
            max_result_chars: 12_000,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolSpec {
    pub name: String,
    pub description: String,
    pub parameters: Value,
}

#[derive(Debug, PartialEq, Eq, Error)]
pub enum ToolError {
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
    #[error("knowledge vault is read-only")]
    ReadOnly,
}

impl From<wiki_core::VaultError> for ToolError {
    fn from(error: wiki_core::VaultError) -> Self {
        match error {
            wiki_core::VaultError::ReadOnly => Self::ReadOnly,
            wiki_core::VaultError::InvalidPath(message) => Self::InvalidArguments(message),
            other => Self::ExecutionFailed(other.to_string()),
        }
    }
}

pub struct ToolHost {
    vault: Arc<Vault>,
    policy: std::sync::RwLock<Policy>,
    active_requests: Arc<AtomicUsize>,
    read_tools: Vec<ToolSpec>,
    write_tools: Vec<ToolSpec>,
}

impl ToolHost {
    pub fn new(vault: Arc<Vault>, policy: Policy) -> Self {
        let mut read_tools = Vec::new();
        let mut write_tools = Vec::new();
        for spec in build_llm_tool_definitions() {
            let mut parameters = spec.parameters;
            sanitize_schema(&mut parameters);
            let tool = ToolSpec {
                name: spec.name.clone(),
                description: spec.description,
                parameters,
            };
            if is_read_only_wiki_tool(&spec.name) {
                read_tools.push(tool.clone());
            }
            write_tools.push(tool);
        }
        Self {
            vault,
            policy: std::sync::RwLock::new(policy),
            active_requests: Arc::new(AtomicUsize::new(0)),
            read_tools,
            write_tools,
        }
    }

    pub fn update_policy(&self, policy: Policy) {
        *self.policy.write().expect("ToolHost policy lock") = policy;
    }

    pub fn active_requests(&self) -> usize {
        self.active_requests.load(Ordering::Acquire)
    }

    pub fn list_tools(&self, access: Access) -> Vec<ToolSpec> {
        let policy = *self.policy.read().expect("ToolHost policy lock");
        if access == Access::ReadWrite && policy.allow_write {
            self.write_tools.clone()
        } else {
            self.read_tools.clone()
        }
    }

    pub async fn call(&self, ctx: CallCtx, name: &str, args: Value) -> Result<Value, ToolError> {
        let policy = *self.policy.read().expect("ToolHost policy lock");
        let write_allowed = ctx.access == Access::ReadWrite && policy.allow_write;
        if !self
            .list_tools(ctx.access)
            .iter()
            .any(|tool| tool.name == name)
        {
            return Err(ToolError::Denied);
        }
        if !is_read_only_wiki_tool(name) && !write_allowed {
            return Err(ToolError::Denied);
        }
        if !args.is_object() {
            return Err(ToolError::InvalidArguments(
                "tool arguments must be a JSON object".into(),
            ));
        }
        if ctx.cancel.is_cancelled() {
            return Err(ToolError::Cancelled);
        }
        if tokio::time::Instant::now() >= ctx.deadline {
            return Err(ToolError::Timeout);
        }
        let max_concurrency = policy.max_concurrency.max(1);
        let mut active = self.active_requests.load(Ordering::Acquire);
        loop {
            if active >= max_concurrency {
                return Err(ToolError::Unavailable);
            }
            match self.active_requests.compare_exchange_weak(
                active,
                active + 1,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => break,
                Err(current) => active = current,
            }
        }
        let _active = ActiveRequest(Arc::clone(&self.active_requests));
        let vault = Arc::clone(&self.vault);
        let name = name.to_owned();
        let max_chars = policy.max_result_chars;
        let deadline = ctx
            .deadline
            .min(tokio::time::Instant::now() + policy.call_timeout);
        if tokio::time::Instant::now() >= deadline {
            return Err(ToolError::Timeout);
        }
        let mut task =
            tokio::task::spawn_blocking(move || call_sync(&vault, &name, args, max_chars));
        let value = tokio::select! {
            biased;
            _ = ctx.cancel.cancelled() => {
                task.abort();
                return Err(ToolError::Cancelled);
            }
            _ = tokio::time::sleep_until(deadline) => {
                task.abort();
                return Err(ToolError::Timeout);
            }
            result = &mut task => result.map_err(|_| ToolError::Unavailable)??,
        };
        Ok(bound_result_chars(value, max_chars))
    }
}

struct ActiveRequest(Arc<AtomicUsize>);
impl Drop for ActiveRequest {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::AcqRel);
    }
}

fn sanitize_schema(value: &mut Value) {
    const ALLOWED: &[&str] = &[
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
    let Some(object) = value.as_object_mut() else {
        return;
    };
    object.retain(|key, _| ALLOWED.contains(&key.as_str()));
    if let Some(Value::Object(properties)) = object.get_mut("properties") {
        properties.values_mut().for_each(sanitize_schema);
    }
    if let Some(items) = object.get_mut("items") {
        sanitize_schema(items);
    }
}

fn bound_result_chars(mut value: Value, max_chars: usize) -> Value {
    let serialized = serde_json::to_string(&value).unwrap_or_default();
    if serialized.len() <= max_chars {
        return value;
    }

    if let Some(object) = value.as_object_mut() {
        object.insert("truncated".into(), Value::Bool(true));
        if let Some(metadata) = object.get("metadata").cloned() {
            let metadata = match metadata {
                Value::Object(mut values) => {
                    for key in ["results", "tree", "entries", "backlinks", "outgoing_links"] {
                        values.remove(key);
                    }
                    Value::Object(values)
                }
                _ => Value::Null,
            };
            object.insert("metadata".into(), metadata);
        }
    }
    if serde_json::to_string(&value).unwrap_or_default().len() <= max_chars {
        return value;
    }

    if let Some(Value::String(content)) = value
        .as_object_mut()
        .and_then(|object| object.get_mut("content"))
    {
        let budget = max_chars.saturating_sub(256);
        let mut escaped_len = 0;
        let mut cut = content.len();
        for (index, character) in content.char_indices() {
            escaped_len += match character {
                '"' | '\\' | '\n' | '\r' | '\t' | '\u{8}' | '\u{c}' => 2,
                c if c < ' ' => 6,
                c => c.len_utf8(),
            };
            if escaped_len > budget {
                cut = index;
                break;
            }
        }
        content.truncate(cut);
        content.push_str("\n...[truncated]");
    }
    if serde_json::to_string(&value).unwrap_or_default().len() <= max_chars {
        return value;
    }
    json!({"truncated": true, "content": "...[truncated]"})
}

fn call_sync(vault: &Vault, name: &str, args: Value, max_chars: usize) -> Result<Value, ToolError> {
    if is_read_only_wiki_tool(name) {
        return Ok(vault.read_tool(name, args, max_chars)?);
    }
    let scope = parse_scope(args.get("scope").and_then(Value::as_str).unwrap_or("wiki"))?;
    let output = match name {
        "wiki_tree" => {
            let scope = parse_scope(args.get("scope").and_then(Value::as_str).unwrap_or("all"))?;
            let mut tree = vault.tree(scope)?;
            if let Some(sub_path) = args
                .get("sub_path")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|path| !path.is_empty() && *path != ".")
            {
                for part in sub_path.trim_matches('/').split('/') {
                    tree = tree
                        .children
                        .into_iter()
                        .find(|child| child.name == part)
                        .ok_or_else(|| {
                            ToolError::InvalidArguments(format!("tree path not found: {sub_path}"))
                        })?;
                }
            }
            if let Some(depth) = args.get("max_depth").and_then(Value::as_u64) {
                limit_tree_depth(&mut tree, depth as usize);
            }
            let mut nodes = 0usize;
            let mut lines = String::new();
            render_tree(&tree, "", true, &mut lines, &mut nodes);
            json!({"tool": name, "content": lines, "metadata": {"tree": tree, "total_files": nodes}})
        }
        "wiki_query" => {
            let query = args
                .get("query")
                .and_then(Value::as_str)
                .or_else(|| {
                    args.get("searches")
                        .and_then(Value::as_array)
                        .and_then(|a| a.first())
                        .and_then(|v| v.get("query"))
                        .and_then(Value::as_str)
                })
                .ok_or_else(|| ToolError::InvalidArguments("missing query".into()))?;
            let mut hits = vault.search(query)?;
            if let Some(collections) = args.get("collections").and_then(Value::as_array) {
                hits.retain(|hit| {
                    collections.iter().filter_map(Value::as_str).any(|scope| {
                        hit.path
                            .starts_with(if scope == "sources" || scope == "raw" {
                                "sources/"
                            } else {
                                "wiki/"
                            })
                    })
                });
            } else {
                hits.retain(|hit| hit.path.starts_with("wiki/"));
            }
            let limit = args.get("limit").and_then(Value::as_u64).unwrap_or(10) as usize;
            hits.truncate(limit);
            let content = serde_json::to_string_pretty(&hits).unwrap_or_default();
            json!({"tool": name, "content": content, "metadata": {"results": hits}})
        }
        "wiki_get" => {
            let file = args
                .get("file")
                .and_then(Value::as_str)
                .ok_or_else(|| ToolError::InvalidArguments("missing file".into()))?;
            let (space, path) = scoped_doc_path(file)?;
            let page = vault.read(
                &path,
                ReadWindow {
                    from_line: args
                        .get("from_line")
                        .and_then(Value::as_u64)
                        .map(|n| n as usize),
                    max_lines: args
                        .get("max_lines")
                        .and_then(Value::as_u64)
                        .map(|n| n as usize),
                },
            )?;
            json!({"tool": name, "content": format!("FILE: {}\n\n{}", page.path, page.content), "metadata": {"rel_path": page.path, "title": page.title, "revision": page.revision.0, "space": space}})
        }
        "wiki_multi_get" => {
            let pattern = args
                .get("pattern")
                .and_then(Value::as_str)
                .ok_or_else(|| ToolError::InvalidArguments("missing pattern".into()))?;
            let mut docs = Vec::new();
            for raw_path in pattern.split(',').map(str::trim).filter(|s| !s.is_empty()) {
                let (_, path) = scoped_doc_path(raw_path)?;
                docs.push(vault.read(&path, ReadWindow::default())?);
            }
            let content = serde_json::to_string_pretty(&docs).unwrap_or_default();
            json!({"tool": name, "content": content, "metadata": {"count": docs.len(), "results": docs}})
        }
        "wiki_metadata" => {
            json!({"tool": name, "content": serde_json::to_string_pretty(&vault.status()?).unwrap_or_default(), "metadata": vault.status()?})
        }
        "wiki_status" => {
            let status = vault.status()?;
            json!({"tool": name, "content": serde_json::to_string_pretty(&status).unwrap_or_default(), "metadata": status})
        }
        "wiki_graph_neighbors" => {
            if let Some(file) = args.get("rel_path").and_then(Value::as_str) {
                let (_, path) = scoped_doc_path(file)?;
                let graph = vault.graph(Some(&path))?;
                let outgoing: Vec<_> = graph
                    .edges
                    .iter()
                    .filter(|edge| edge.source == path.vault_relative().to_string_lossy())
                    .collect();
                let backlinks: Vec<_> = graph
                    .edges
                    .iter()
                    .filter(|edge| {
                        edge.target.as_deref()
                            == Some(path.vault_relative().to_string_lossy().as_ref())
                    })
                    .collect();
                json!({"tool": name, "content": serde_json::to_string_pretty(&graph).unwrap_or_default(), "metadata": {"target": file, "backlinks_count": backlinks.len(), "outgoing_count": outgoing.len(), "backlinks": backlinks, "outgoing_links": outgoing}})
            } else {
                let graph = vault.graph(None)?;
                json!({"tool": name, "content": serde_json::to_string_pretty(&graph).unwrap_or_default(), "metadata": graph})
            }
        }
        "wiki_write_file" => {
            let rel = args
                .get("rel_path")
                .and_then(Value::as_str)
                .ok_or_else(|| ToolError::InvalidArguments("missing rel_path".into()))?;
            let (_space, path) = scoped_doc_path_with_scope(scope, rel)?;
            let content = args
                .get("content")
                .and_then(Value::as_str)
                .ok_or_else(|| ToolError::InvalidArguments("missing content".into()))?;
            vault.apply_from(
                Origin::Mcp,
                Change::Write {
                    path: path.clone(),
                    content: content.to_owned(),
                    expect: None,
                },
            )?;
            let info = vault.document_metadata(&path)?;
            let docid = info.get("docid").and_then(Value::as_str).unwrap_or("");
            json!({"tool": name, "content": format!("Updated `{}` (docid: {docid})", path.vault_relative().display()), "metadata": info})
        }
        "wiki_patch_file" => {
            let rel = args
                .get("rel_path")
                .and_then(Value::as_str)
                .ok_or_else(|| ToolError::InvalidArguments("missing rel_path".into()))?;
            let (_space, path) = scoped_doc_path_with_scope(scope, rel)?;
            let page = vault.read(&path, ReadWindow::default())?;
            let mut content = page.content;
            if let (Some(target), Some(replacement)) = (
                args.get("target_text").and_then(Value::as_str),
                args.get("replacement_text").and_then(Value::as_str),
            ) {
                if !target.is_empty() && content.contains(target) {
                    content = content.replace(target, replacement);
                } else if !target.is_empty() {
                    let wrapped = format!(
                        "[[{}]]",
                        target.trim_start_matches("[[").trim_end_matches("]]"),
                    );
                    if !target.is_empty() && content.contains(&wrapped) {
                        let replacement = if replacement.trim().starts_with("[[")
                            && replacement.trim().ends_with("]]")
                        {
                            replacement.trim().to_owned()
                        } else {
                            format!("[[{}]]", replacement.trim())
                        };
                        content = content.replace(&wrapped, &replacement);
                    } else {
                        return Err(ToolError::InvalidArguments("target_text not found".into()));
                    }
                }
            }
            if let Some(append) = args
                .get("append_text")
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
            {
                if !content.contains(append.trim()) {
                    if !content.ends_with('\n') {
                        content.push('\n')
                    }
                    content.push_str(append);
                    if !content.ends_with('\n') {
                        content.push('\n')
                    }
                }
            }
            vault.apply_from(
                Origin::Mcp,
                Change::Write {
                    path: path.clone(),
                    content,
                    expect: Some(page.revision),
                },
            )?;
            let info = vault.document_metadata(&path)?;
            let docid = info.get("docid").and_then(Value::as_str).unwrap_or("");
            json!({"tool": name, "content": format!("Updated `{}` (docid: {docid})", path.vault_relative().display()), "metadata": info})
        }
        "wiki_ingest_source" => {
            let req: llm_wiki::IngestSourceRequest = serde_json::from_value(args)
                .map_err(|error| ToolError::InvalidArguments(error.to_string()))?;
            let source = DocPath::parse(
                Scope::Raw,
                req.source_rel_path.trim_start_matches("sources/"),
            )?;
            let details = IngestSourceDetails {
                target_wiki_path: req.target_wiki_path,
                raw_source_content: req.raw_source_content,
                title: req.title,
                one_line_summary: req.one_line_summary,
                wiki_markdown_body: req.wiki_markdown_body,
                tags: req.tags,
                related_entities: req.related_entities,
                related_concepts: req.related_concepts,
            };
            let applied = vault.apply_from(
                Origin::Mcp,
                Change::IngestSource {
                    source: source.clone(),
                    details: Some(details),
                },
            )?;
            let wiki_rel_path = applied
                .paths
                .iter()
                .find(|path| {
                    path.starts_with("wiki/")
                        && !matches!(path.as_str(), "wiki/index.md" | "wiki/log.md")
                })
                .cloned()
                .ok_or_else(|| {
                    ToolError::ExecutionFailed("ingest did not produce a wiki page".into())
                })?;
            let wiki_scoped_path = wiki_rel_path.trim_start_matches("wiki/").to_owned();
            let wiki_path = DocPath::parse(Scope::Wiki, &wiki_scoped_path)?;
            let document = vault.document_metadata(&wiki_path)?;
            let file_name = wiki_scoped_path
                .rsplit('/')
                .next()
                .unwrap_or(&wiki_scoped_path)
                .to_owned();
            let result = json!({
                "source_rel_path": source.vault_relative(),
                "wiki_scoped_path": wiki_scoped_path,
                "wiki_rel_path": wiki_rel_path,
                "file_name": file_name,
                "wiki_docid": document.get("docid").cloned().unwrap_or(Value::Null),
            });
            let docid = result["wiki_docid"].as_str().unwrap_or("");
            json!({"tool": name, "content": format!("Ingested `{wiki_rel_path}` (docid: {docid})"), "metadata": result})
        }
        "wiki_file_synthesis" => {
            let req: llm_wiki::FileSynthesisRequest = serde_json::from_value(args)
                .map_err(|error| ToolError::InvalidArguments(error.to_string()))?;
            let rel_path = req.rel_path.trim_start_matches("wiki/");
            let rel_path = if rel_path.starts_with("syntheses/") {
                rel_path.to_owned()
            } else {
                format!("syntheses/{rel_path}")
            };
            let path = DocPath::parse(Scope::Wiki, &rel_path)?;
            let content = llm_wiki::wiki::ingest::render_synthesis_markdown(&req);
            vault.apply_from(
                Origin::Mcp,
                Change::Write {
                    path: path.clone(),
                    content,
                    expect: None,
                },
            )?;
            let info = vault.document_metadata(&path)?;
            let docid = info.get("docid").and_then(Value::as_str).unwrap_or("");
            json!({"tool": name, "content": format!("Filed synthesis `{}` (docid: {docid})", path.vault_relative().display()), "metadata": info})
        }
        "wiki_delete_file" => {
            let rel = args
                .get("rel_path")
                .and_then(Value::as_str)
                .filter(|path| !path.trim().is_empty())
                .ok_or_else(|| ToolError::InvalidArguments("missing rel_path".into()))?;
            let (space, path) = scoped_doc_path_with_scope(scope, rel)?;
            let deleted = match vault.read(&path, ReadWindow::default()) {
                Ok(_) => true,
                Err(wiki_core::VaultError::NotFound(_)) => false,
                Err(error) => return Err(error.into()),
            };
            if deleted {
                vault.apply_from(Origin::Mcp, Change::Delete { path: path.clone() })?;
            }
            json!({"tool": name, "content": format!("deleted={deleted} path={}", path.vault_relative().display()), "metadata": {"deleted": deleted, "rel_path": path.vault_relative(), "scope": space}})
        }
        _ => return Err(ToolError::Unavailable),
    };
    Ok(output)
}

fn parse_scope(raw: &str) -> Result<Scope, ToolError> {
    match raw {
        "raw" | "sources" => Ok(Scope::Raw),
        "wiki" => Ok(Scope::Wiki),
        "all" => Ok(Scope::All),
        _ => Err(ToolError::InvalidArguments(format!("invalid scope: {raw}"))),
    }
}

fn scoped_doc_path(file: &str) -> Result<(Scope, DocPath), ToolError> {
    let clean = file.trim().trim_start_matches('/');
    let (scope, relative) = if let Some(relative) = clean.strip_prefix("sources/") {
        (Scope::Raw, relative)
    } else if let Some(relative) = clean.strip_prefix("wiki/") {
        (Scope::Wiki, relative)
    } else {
        (Scope::Wiki, clean)
    };
    Ok((scope, DocPath::parse(scope, relative)?))
}

fn scoped_doc_path_with_scope(scope: Scope, file: &str) -> Result<(Scope, DocPath), ToolError> {
    let clean = file.trim().trim_start_matches('/');
    let prefixed = match scope {
        Scope::Raw if clean.starts_with("sources/") => &clean[8..],
        Scope::Wiki if clean.starts_with("wiki/") => &clean[5..],
        _ => clean,
    };
    Ok((scope, DocPath::parse(scope, prefixed)?))
}

fn render_tree(
    node: &wiki_core::TreeNode,
    prefix: &str,
    last: bool,
    out: &mut String,
    count: &mut usize,
) {
    let connector = if last { "└── " } else { "├── " };
    out.push_str(prefix);
    out.push_str(connector);
    out.push_str(&node.name);
    if node.is_directory {
        out.push('/');
    } else {
        *count += 1;
    }
    out.push('\n');
    let next = format!("{prefix}{}", if last { "    " } else { "│   " });
    for (index, child) in node.children.iter().enumerate() {
        render_tree(child, &next, index + 1 == node.children.len(), out, count);
    }
}

fn limit_tree_depth(node: &mut wiki_core::TreeNode, depth: usize) {
    if depth == 0 {
        node.children.clear();
        return;
    }
    for child in &mut node.children {
        limit_tree_depth(child, depth - 1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn host(allow_write: bool) -> (tempfile::TempDir, ToolHost) {
        let dir = tempdir().unwrap();
        let vault = Arc::new(Vault::open(wiki_core::VaultConfig::new(dir.path())).unwrap());
        let host = ToolHost::new(
            vault,
            Policy {
                allow_write,
                max_concurrency: 2,
                call_timeout: Duration::from_secs(2),
                max_result_chars: 12_000,
            },
        );
        (dir, host)
    }

    fn context(access: Access) -> CallCtx {
        CallCtx {
            access,
            cancel: CancellationToken::new(),
            deadline: tokio::time::Instant::now() + Duration::from_secs(5),
        }
    }

    #[test]
    fn read_write_access_lists_only_read_tools_when_policy_disables_writes() {
        let (_dir, host) = host(false);
        let tools = host.list_tools(Access::ReadWrite);
        assert!(tools.iter().any(|tool| tool.name == "wiki_status"));
        assert!(!tools.iter().any(|tool| tool.name == "wiki_write_file"));
    }

    #[tokio::test]
    async fn calls_read_vault_status_and_apply_writes_through_vault() {
        let (_dir, host) = host(true);
        let status = host
            .call(context(Access::ReadOnly), "wiki_status", json!({}))
            .await
            .unwrap();
        assert_eq!(status["tool"], "wiki_status");
        let write = host.call(context(Access::ReadWrite), "wiki_write_file", json!({"scope":"wiki","rel_path":"concepts/mcp.md","content":"# MCP\n\nShared host."})).await.unwrap();
        assert!(
            write["content"]
                .as_str()
                .unwrap()
                .contains("concepts/mcp.md")
        );
        let query = host
            .call(
                context(Access::ReadOnly),
                "wiki_query",
                json!({"query":"Shared host"}),
            )
            .await
            .unwrap();
        assert!(query["content"].as_str().unwrap().contains("mcp.md"));
    }

    #[tokio::test]
    async fn expired_and_cancelled_calls_return_typed_errors() {
        let (_dir, host) = host(true);
        let mut ctx = context(Access::ReadOnly);
        ctx.deadline = tokio::time::Instant::now() - Duration::from_millis(1);
        assert_eq!(
            host.call(ctx, "wiki_status", json!({})).await,
            Err(ToolError::Timeout)
        );
        let ctx = CallCtx {
            access: Access::ReadOnly,
            cancel: CancellationToken::new(),
            deadline: tokio::time::Instant::now() + Duration::from_secs(1),
        };
        ctx.cancel.cancel();
        assert_eq!(
            host.call(ctx, "wiki_status", json!({})).await,
            Err(ToolError::Cancelled)
        );
    }

    #[tokio::test]
    async fn call_timeout_and_concurrency_saturation_return_typed_errors() {
        let (_dir, host) = host(true);
        host.update_policy(Policy {
            call_timeout: Duration::ZERO,
            ..Policy::default()
        });
        assert_eq!(
            host.call(context(Access::ReadOnly), "wiki_status", json!({}))
                .await,
            Err(ToolError::Timeout)
        );

        host.update_policy(Policy {
            max_concurrency: 1,
            ..Policy::default()
        });
        host.active_requests.store(1, Ordering::Release);
        assert_eq!(
            host.call(context(Access::ReadOnly), "wiki_status", json!({}))
                .await,
            Err(ToolError::Unavailable)
        );
        host.active_requests.store(0, Ordering::Release);
    }

    #[tokio::test]
    async fn write_tools_preserve_patch_ingest_and_delete_compatibility() {
        let (dir, host) = host(true);
        host.call(
            context(Access::ReadWrite),
            "wiki_write_file",
            json!({"scope":"wiki","rel_path":"concepts/patch.md","content":"# Patch\n\nSee [[Old Page]].\n"}),
        )
        .await
        .unwrap();
        let patched = host
            .call(
                context(Access::ReadWrite),
                "wiki_patch_file",
                json!({"rel_path":"concepts/patch.md","target_text":"Old Page","replacement_text":"New Page"}),
            )
            .await
            .unwrap();
        assert!(patched["content"].as_str().unwrap().contains("(docid:"));
        assert!(dir.path().join("wiki/concepts/patch.md").exists());

        let ingested = host
            .call(
                context(Access::ReadWrite),
                "wiki_ingest_source",
                json!({
                    "source_rel_path":"notes/provided.md",
                    "raw_source_content":"# Source\n\nSupplied source content.",
                    "title":"Provided source",
                    "one_line_summary":"Supplied summary.",
                    "wiki_markdown_body":"# Provided source\n\nSupplied wiki content.",
                    "tags":["provided"],
                    "related_entities":[],
                    "related_concepts":[]
                }),
            )
            .await
            .unwrap();
        assert_eq!(
            ingested["metadata"]["source_rel_path"],
            "sources/notes/provided.md"
        );
        assert!(dir.path().join("sources/notes/provided.md").exists());
        let wiki_path = ingested["metadata"]["wiki_rel_path"].as_str().unwrap();
        assert!(dir.path().join(wiki_path).exists());

        host.call(
            context(Access::ReadWrite),
            "wiki_delete_file",
            json!({"scope":"wiki","rel_path":"concepts/patch.md"}),
        )
        .await
        .unwrap();
        assert!(!dir.path().join("wiki/concepts/patch.md").exists());
    }
}
