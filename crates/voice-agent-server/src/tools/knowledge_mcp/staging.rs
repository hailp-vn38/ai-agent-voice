//! JSON Schema sanitization, `add_to_sources` staging execution, and tool result bounding.

use super::KnowledgeToolError;
use crate::providers::llm::ToolDefinition;
use llm_wiki::{WikiEngine, WriteTopicSourceRequest, sanitize_topic_folder, slugify};
use serde_json::{Value, json};
use std::sync::Arc;
use tokio::sync::Notify;

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

/// Recursively strips JSON Schema keywords not in `ALLOWED_SCHEMA_KEYWORDS` (such as `"default"`)
/// so that every tool schema published by `KnowledgeMcp` passes `validate_external_tool_schema`.
pub(crate) fn sanitize_schema_node(node: &mut Value) {
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

pub(crate) fn add_to_sources_tool_definition() -> ToolDefinition {
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

pub(crate) fn execute_add_to_sources(
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

    let mut final_body = normalize_wiki_markdown_tables(body.trim_end());
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

pub(crate) fn bound_result_chars(mut value: Value, max_chars: usize) -> Value {
    let serialized = serde_json::to_string(&value).unwrap_or_default();
    if serialized.len() <= max_chars {
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
    if after_slim.len() <= max_chars {
        return value;
    }

    // Truncate `content` string so the entire JSON object stays within `max_chars`.
    if let Some(obj) = value.as_object_mut()
        && let Some(Value::String(content)) = obj.get_mut("content")
    {
        let budget = max_chars.saturating_sub(256);
        let mut cut = budget.min(content.len());
        while cut > 0 && !content.is_char_boundary(cut) {
            cut -= 1;
        }
        content.truncate(cut);
        content.push_str("\n...[truncated]");
    }

    let after_content = serde_json::to_string(&value).unwrap_or_default();
    if after_content.len() <= max_chars {
        return value;
    }

    json!({
        "truncated": true,
        "content": "...[truncated]",
    })
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

fn is_table_separator_line(line: &str) -> bool {
    let trimmed = line.trim();
    if !trimmed.contains('|') {
        return false;
    }
    let inner = trimmed.strip_prefix('|').unwrap_or(trimmed);
    let inner = inner.strip_suffix('|').unwrap_or(inner);
    let mut count = 0usize;
    for cell in inner.split('|') {
        let c = cell.trim();
        if c.is_empty() {
            return false;
        }
        let c = c.strip_prefix(':').unwrap_or(c);
        let c = c.strip_suffix(':').unwrap_or(c);
        if c.len() < 3 || !c.bytes().all(|b| b == b'-') {
            return false;
        }
        count += 1;
    }
    count > 0
}

fn is_non_table_block_line(line: &str) -> bool {
    let trimmed = line.trim_start();
    trimmed.starts_with("```")
        || trimmed.starts_with("~~~")
        || trimmed.starts_with('#')
        || trimmed.starts_with('>')
}

fn escape_wikilink_pipes_in_table_row(line: &str) -> String {
    let chars: Vec<char> = line.chars().collect();
    let mut out = String::with_capacity(line.len() + 8);
    let mut wiki_depth = 0usize;
    let mut i = 0usize;
    while i < chars.len() {
        let ch = chars[i];
        let next = chars.get(i + 1).copied();
        if ch == '[' && next == Some('[') {
            wiki_depth += 1;
            out.push_str("[[");
            i += 2;
            continue;
        }
        if wiki_depth > 0 && ch == ']' && next == Some(']') {
            wiki_depth -= 1;
            out.push_str("]]");
            i += 2;
            continue;
        }
        if wiki_depth > 0 && ch == '\\' && next == Some('|') {
            out.push_str("\\|");
            i += 2;
            continue;
        }
        if wiki_depth > 0 && ch == '|' {
            out.push_str("\\|");
            i += 1;
            continue;
        }
        out.push(ch);
        i += 1;
    }
    out
}

/// Normalizes GFM tables in Wiki/Source Markdown so that files render cleanly both in Admin Web
/// and in external Markdown applications (Obsidian, VS Code, Typora, GitHub):
/// 1. Preserves YAML frontmatter (`--- ... ---`) and fenced code blocks (` ``` ` / `~~~`) verbatim.
/// 2. Ensures blank lines before and after multi-line GFM table blocks.
/// 3. Escapes unescaped pipes inside `[[target|alias]]` WikiLinks on table rows as `[[target\|alias]]`
///    so external table parsers do not split the cell at the WikiLink pipe.
pub(crate) fn normalize_wiki_markdown_tables(markdown: &str) -> String {
    if !markdown.contains('|') {
        return markdown.to_string();
    }
    let trailing_newline = markdown.ends_with('\n');
    let lines: Vec<&str> = markdown.lines().collect();
    let mut out: Vec<String> = Vec::with_capacity(lines.len() + 4);

    let mut idx = 0usize;
    if let Some(first) = lines.first()
        && first.trim_start_matches('\u{FEFF}').trim() == "---"
        && let Some(end_offset) = lines[1..].iter().position(|l| l.trim() == "---")
    {
        let close_idx = 1 + end_offset;
        for line in &lines[..=close_idx] {
            out.push((*line).to_string());
        }
        idx = close_idx + 1;
    }

    let frontmatter_end = idx;
    let mut fence: Option<char> = None;

    while idx < lines.len() {
        let line = lines[idx];
        let trimmed_start = line.trim_start();
        if trimmed_start.starts_with("```") || trimmed_start.starts_with("~~~") {
            let marker = trimmed_start.chars().next().unwrap_or('`');
            match fence {
                None => fence = Some(marker),
                Some(open) if open == marker => fence = None,
                Some(_) => {}
            }
            out.push(line.to_string());
            idx += 1;
            continue;
        }

        if fence.is_some() {
            out.push(line.to_string());
            idx += 1;
            continue;
        }

        if line.contains('|')
            && !is_non_table_block_line(line)
            && idx + 1 < lines.len()
            && is_table_separator_line(lines[idx + 1])
        {
            if idx > frontmatter_end
                && out
                    .last()
                    .is_some_and(|prev| !prev.trim().is_empty() && !prev.trim().starts_with('>'))
            {
                out.push(String::new());
            }

            out.push(escape_wikilink_pipes_in_table_row(line));
            out.push(lines[idx + 1].to_string());
            idx += 2;

            while idx < lines.len() {
                let row = lines[idx];
                let row_trim = row.trim();
                if row_trim.is_empty()
                    || !row_trim.contains('|')
                    || is_non_table_block_line(row)
                {
                    break;
                }
                out.push(escape_wikilink_pipes_in_table_row(row));
                idx += 1;
            }

            if idx < lines.len() && !lines[idx].trim().is_empty() {
                out.push(String::new());
            }
            continue;
        }

        let trimmed = line.trim();
        if trimmed.starts_with('|') && trimmed.ends_with('|') && !is_table_separator_line(trimmed) {
            out.push(escape_wikilink_pipes_in_table_row(line));
        } else {
            out.push(line.to_string());
        }
        idx += 1;
    }

    let mut joined = out.join("\n");
    if trailing_newline {
        joined.push('\n');
    }
    joined
}

/// Normalizes Markdown body arguments on mutating Wiki/Report tools before persistence.
pub(crate) fn normalize_wiki_tool_arguments(tool_name: &str, args: &mut Value) {
    if !matches!(
        tool_name,
        "wiki_write_file"
            | "wiki_patch_file"
            | "wiki_file_synthesis"
            | "wiki_ingest_source"
            | "add_to_sources"
            | "markdown_create"
    ) {
        return;
    }
    let Some(obj) = args.as_object_mut() else {
        return;
    };
    for key in [
        "content",
        "wiki_markdown_body",
        "markdown_body",
        "raw_source_content",
        "replacement_text",
        "append_text",
        "body",
        "markdown",
    ] {
        if let Some(Value::String(text)) = obj.get_mut(key) {
            *text = normalize_wiki_markdown_tables(text);
        }
    }
}
