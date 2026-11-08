use crate::{
    ToolError, limit_tree_depth, parse_scope, render_tree, scoped_doc_path,
    scoped_doc_path_with_scope,
};
use llm_wiki::is_read_only_wiki_tool;
use serde_json::{Value, json};
use wiki_core::{
    Change, DocPath, IngestSourceDetails, Origin, ReadWindow, Scope, SynthesisDetails, Vault,
};

pub(super) fn call_sync(
    vault: &Vault,
    name: &str,
    args: Value,
    max_chars: usize,
) -> Result<Value, ToolError> {
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
                && !content.contains(append.trim())
            {
                if !content.ends_with('\n') {
                    content.push('\n')
                }
                content.push_str(append);
                if !content.ends_with('\n') {
                    content.push('\n')
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
            let rel_path = if rel_path.ends_with(".md") {
                rel_path
            } else {
                format!("{rel_path}.md")
            };
            let path = DocPath::parse(Scope::Wiki, &rel_path)?;
            vault.apply_from(
                Origin::Mcp,
                Change::FileSynthesis {
                    path: path.clone(),
                    details: SynthesisDetails {
                        query: req.query,
                        title: req.title,
                        one_line_summary: req.one_line_summary,
                        markdown_body: req.markdown_body,
                        tags: req.tags,
                    },
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
            let (_space, path) = scoped_doc_path_with_scope(scope, rel)?;
            let deleted = match vault.read(&path, ReadWindow::default()) {
                Ok(_) => true,
                Err(wiki_core::VaultError::NotFound(_)) => false,
                Err(error) => return Err(error.into()),
            };
            vault.apply_from(Origin::Mcp, Change::Delete { path: path.clone() })?;
            json!({"tool": name, "content": format!("deleted={deleted} path={rel}"), "metadata": {"deleted": deleted, "rel_path": rel}})
        }
        _ => return Err(ToolError::Unavailable),
    };
    Ok(output)
}
