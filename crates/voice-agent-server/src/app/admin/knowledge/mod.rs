//! Admin HTTP handlers for Knowledge and Memory routes backed by `llm_wiki::WikiEngine`
//! and `orchestrator_flow_rs::TaskManager`.

mod memory;
mod raw;

pub(super) use memory::{
    memory_create, memory_forget, memory_health, memory_read, memory_rebuild, memory_recall,
    memory_summary, memory_update,
};
pub(super) use raw::{
    knowledge_folder_create, knowledge_page_move, knowledge_raw_create, knowledge_raw_import,
    knowledge_raw_move, knowledge_raw_read, knowledge_raw_update, knowledge_tree,
};

use super::*;
use llm_wiki::{
    CollectionScope, GetOptions, HybridQueryRequest, MultiGetOptions, WikiEngine, WikiError,
    WriteTopicSourceRequest, slugify,
};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::sync::Arc;

#[allow(clippy::result_large_err)]
pub(super) fn wiki(state: &AppState, req: &Request) -> Result<Arc<WikiEngine>, Response> {
    state
        .wiki_engine
        .as_ref()
        .cloned()
        .ok_or_else(|| error(req, StatusCode::SERVICE_UNAVAILABLE, "knowledge_unavailable"))
}

pub(super) fn wiki_error(req: &Request, err: WikiError) -> Response {
    match err {
        WikiError::DocumentNotFound { .. } => error(req, StatusCode::NOT_FOUND, "not_found"),
        WikiError::PathTraversal(_) | WikiError::InvalidFileName(_) => {
            error(req, StatusCode::BAD_REQUEST, "invalid_path")
        }
        WikiError::InvalidMetadataFilter(_) | WikiError::InvalidToolArguments { .. } => {
            error(req, StatusCode::BAD_REQUEST, "invalid_arguments")
        }
        _ => error(req, StatusCode::SERVICE_UNAVAILABLE, "knowledge_unavailable"),
    }
}

pub(super) fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

pub(super) fn resolve_wiki_scoped_path(engine: &WikiEngine, id: &str) -> String {
    if let Ok(existing) = engine.get_document(id, GetOptions::default()) {
        existing.info.scoped_path
    } else {
        id.trim_start_matches("wiki/").to_string()
    }
}

pub(super) async fn trigger_source_analysis(state: &AppState, rel_path: &str, body: &mut Value) {
    if let Some(ref tm) = state.task_manager {
        if let Ok(task) = crate::services::task_flow::submit_wiki_source_analysis(tm, rel_path).await
        {
            body["analysis"] = serde_json::json!({
                "job_id": task.id,
                "session_id": task.session_id,
                "turn_id": task.latest_turn_id,
                "replayed": false,
            });
        }
    } else if let Some(ref notify) = state.autoflow_notify {
        notify.notify_one();
    }
}

pub(super) fn format_wiki_markdown(title: &str, body: &str, source_ref: Option<&str>) -> String {
    if body.trim_start().starts_with("---") {
        return body.to_string();
    }
    let mut out = format!("---\ntitle: \"{}\"\n", title.replace('"', "\\\""));
    if let Some(src) = source_ref.filter(|s| !s.trim().is_empty()) {
        out.push_str(&format!("sources:\n  - \"[[{src}]]\"\n"));
    }
    out.push_str("---\n\n");
    if !body.trim_start().starts_with("# ") {
        out.push_str(&format!("# {title}\n\n"));
    }
    out.push_str(body);
    out
}

#[derive(Deserialize)]
pub(super) struct SearchQuery {
    pub(super) q: Option<String>,
    pub(super) limit: Option<i64>,
}

#[derive(Deserialize)]
pub(super) struct PageLines {
    pub(super) from_line: Option<usize>,
    pub(super) max_lines: Option<usize>,
}

#[derive(Deserialize)]
pub(super) struct GraphQuery {
    #[serde(default)]
    scope: Option<String>,
    #[serde(default)]
    limit: Option<usize>,
}

pub(super) async fn knowledge_graph(
    State(state): State<AppState>,
    Query(q): Query<GraphQuery>,
    req: Request,
) -> Response {
    let engine = match wiki(&state, &req) {
        Ok(engine) => engine,
        Err(resp) => return resp,
    };
    let filter_scope = match q.scope.as_deref() {
        Some("sources" | "source" | "raw") => Some(CollectionScope::Sources),
        Some("wiki" | "local") => Some(CollectionScope::Wiki),
        _ => None,
    };
    match engine.get_graph(filter_scope) {
        Ok(graph) => {
            let max_nodes = q.limit.unwrap_or(500).clamp(1, 5_000);
            let truncated = graph.nodes.len() > max_nodes;
            let nodes: Vec<Value> = graph
                .nodes
                .into_iter()
                .take(max_nodes)
                .map(|n| {
                    serde_json::json!({
                        "id": n.rel_path,
                        "path": n.rel_path,
                        "title": n.title,
                        "status": "verified",
                        "revision": 1,
                        "docid": n.docid,
                        "category": n.category,
                        "tags": n.tags,
                    })
                })
                .collect();
            let mut edges = Vec::new();
            let mut unresolved = Vec::new();
            for e in graph.edges {
                if let Some(target_id) = e.resolved_rel_path {
                    edges.push(serde_json::json!({
                        "source_id": e.source_rel_path,
                        "target_id": target_id,
                        "kind": e.link_type,
                        "lines": [e.line_number],
                        "references": [{
                            "line": e.line_number,
                            "raw_target": e.raw_target,
                            "alias": e.display_alias,
                            "anchor": e.target_heading,
                        }],
                    }));
                } else {
                    unresolved.push(serde_json::json!({
                        "source_id": e.source_rel_path,
                        "source_line": e.line_number,
                        "raw_target": e.raw_target,
                        "reason": "unresolved",
                    }));
                }
            }
            Json(serde_json::json!({
                "nodes": nodes,
                "edges": edges,
                "unresolved": unresolved,
                "unresolved_targets": graph.unresolved_targets,
                "truncated": truncated,
            }))
            .into_response()
        }
        Err(err) => wiki_error(&req, err),
    }
}

pub(super) async fn knowledge_health(State(state): State<AppState>, req: Request) -> Response {
    let engine = match wiki(&state, &req) {
        Ok(engine) => engine,
        Err(resp) => return resp,
    };
    match (engine.status(), engine.lint_wiki()) {
        (Ok(status), Ok(lint)) => Json(serde_json::json!({
            "health": if lint.is_healthy { "ready" } else { "needs_attention" },
            "document_count": status.total_documents,
            "last_complete_scan_at": super::now(),
            "healthy": lint.is_healthy,
            "indexed_documents": status.total_documents,
            "sources_count": status.sources_count,
            "wiki_count": status.wiki_count,
            "total_chunks": status.total_chunks,
            "embedded_chunks": status.embedded_chunks,
            "pending_embedding_docs": status.pending_embedding_docs,
            "status": status,
            "lint": lint,
        }))
        .into_response(),
        (Err(err), _) | (_, Err(err)) => wiki_error(&req, err),
    }
}

pub(super) async fn knowledge_rebuild(State(state): State<AppState>, req: Request) -> Response {
    let engine = match wiki(&state, &req) {
        Ok(engine) => engine,
        Err(resp) => return resp,
    };
    let sync = match engine.sync_index() {
        Ok(stats) => stats,
        Err(err) => return wiki_error(&req, err),
    };
    let embed = engine.embed_pending(false).unwrap_or_default();
    let _ = engine.rebuild_or_sync_index_md();
    let indexed = sync.indexed_new + sync.updated + sync.unchanged;
    Json(serde_json::json!({
        "indexed": indexed,
        "sync": sync,
        "embed": embed,
    }))
    .into_response()
}

pub(super) async fn knowledge_search(
    State(state): State<AppState>,
    Query(q): Query<SearchQuery>,
    req: Request,
) -> Response {
    let engine = match wiki(&state, &req) {
        Ok(engine) => engine,
        Err(resp) => return resp,
    };
    let limit = q.limit.unwrap_or(50).clamp(1, 200) as usize;
    if let Some(query) = q.q.as_deref().filter(|query| !query.trim().is_empty()) {
        let hybrid_req = HybridQueryRequest {
            query: Some(query.to_owned()),
            collections: vec![CollectionScope::Wiki],
            limit,
            ..Default::default()
        };
        match engine.query(hybrid_req) {
            Ok(results) => {
                let items: Vec<Value> = results
                    .into_iter()
                    .map(|item| {
                        serde_json::json!({
                            "id": item.rel_path,
                            "docid": item.docid,
                            "title": item.title,
                            "revision": 1,
                            "status": "verified",
                            "source_ref": item.virtual_uri,
                            "excerpt": item.snippet,
                            "relative_path": item.rel_path,
                            "score": item.score,
                        })
                    })
                    .collect();
                Json(serde_json::json!({
                    "items": items,
                    "next_cursor": Value::Null,
                }))
                .into_response()
            }
            Err(err) => wiki_error(&req, err),
        }
    } else {
        match engine.multi_get("wiki/**/*.md", MultiGetOptions::default()) {
            Ok(batch) => {
                let items: Vec<Value> = batch
                    .docs
                    .into_iter()
                    .take(limit)
                    .map(|doc| {
                        let excerpt: String = doc.content.chars().take(240).collect();
                        serde_json::json!({
                            "id": doc.info.rel_path,
                            "docid": doc.info.docid,
                            "title": doc.info.title,
                            "revision": 1,
                            "status": "verified",
                            "source_ref": doc.info.source_rel_path,
                            "excerpt": excerpt,
                            "relative_path": doc.info.rel_path,
                        })
                    })
                    .collect();
                Json(serde_json::json!({
                    "items": items,
                    "next_cursor": Value::Null,
                }))
                .into_response()
            }
            Err(err) => wiki_error(&req, err),
        }
    }
}

pub(super) async fn knowledge_read(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(lines): Query<PageLines>,
    req: Request,
) -> Response {
    let engine = match wiki(&state, &req) {
        Ok(engine) => engine,
        Err(resp) => return resp,
    };
    let opts = GetOptions {
        from_line: lines.from_line,
        max_lines: lines.max_lines,
        line_numbers: false,
        include_frontmatter: true,
    };
    match engine.get_document(&id, opts) {
        Ok(doc) => Json(serde_json::json!({
            "id": doc.info.rel_path,
            "docid": doc.info.docid,
            "title": doc.info.title,
            "body": doc.content,
            "content": doc.content,
            "revision": 1,
            "status": "verified",
            "source_ref": doc.info.source_rel_path,
            "relative_path": doc.info.rel_path,
            "virtual_uri": doc.virtual_uri,
            "tags": doc.info.tags,
            "metadata": doc.info.metadata,
        }))
        .into_response(),
        Err(err) => wiki_error(&req, err),
    }
}

#[derive(Deserialize)]
struct WikiWritePayload {
    title: String,
    body: String,
    #[serde(default)]
    source_ref: Option<String>,
    #[serde(default)]
    folder_path: Option<String>,
    #[serde(default)]
    file_name: Option<String>,
}

pub(super) async fn knowledge_create(State(state): State<AppState>, req: Request) -> Response {
    let (req, input): (_, WikiWritePayload) = match json(req).await {
        Ok(v) => v,
        Err(resp) => return resp,
    };
    let engine = match wiki(&state, &req) {
        Ok(engine) => engine,
        Err(resp) => return resp,
    };
    if input.title.trim().is_empty() {
        return error(&req, StatusCode::BAD_REQUEST, "invalid_arguments");
    }
    let file_stem = input
        .file_name
        .as_deref()
        .filter(|s| !s.trim().is_empty())
        .map(|s| slugify(s, 64))
        .unwrap_or_else(|| slugify(&input.title, 64));
    let folder = input
        .folder_path
        .as_deref()
        .map(|f| f.trim().trim_matches('/').trim_start_matches("wiki/"))
        .filter(|f| !f.is_empty())
        .unwrap_or("concepts");
    let rel_path = format!("{folder}/{file_stem}.md");
    let markdown = format_wiki_markdown(&input.title, &input.body, input.source_ref.as_deref());
    match engine.write_wiki_file(&rel_path, &markdown) {
        Ok(info) => {
            let _ = engine.rebuild_or_sync_index_md();
            (
                StatusCode::CREATED,
                Json(serde_json::json!({
                    "id": info.rel_path,
                    "docid": info.docid,
                    "title": info.title,
                    "body": input.body,
                    "revision": 1,
                    "status": "verified",
                    "source_ref": info.source_rel_path,
                    "relative_path": info.rel_path,
                })),
            )
                .into_response()
        }
        Err(err) => wiki_error(&req, err),
    }
}

pub(super) async fn knowledge_update(
    State(state): State<AppState>,
    Path(id): Path<String>,
    req: Request,
) -> Response {
    let (req, input): (_, WikiWritePayload) = match json(req).await {
        Ok(v) => v,
        Err(resp) => return resp,
    };
    let engine = match wiki(&state, &req) {
        Ok(engine) => engine,
        Err(resp) => return resp,
    };
    let target_rel = resolve_wiki_scoped_path(&engine, &id);
    let markdown = format_wiki_markdown(&input.title, &input.body, input.source_ref.as_deref());
    match engine.write_wiki_file(&target_rel, &markdown) {
        Ok(info) => {
            let _ = engine.rebuild_or_sync_index_md();
            Json(serde_json::json!({
                "id": info.rel_path,
                "docid": info.docid,
                "title": info.title,
                "body": input.body,
                "revision": 1,
                "status": "verified",
                "source_ref": info.source_rel_path,
                "relative_path": info.rel_path,
            }))
            .into_response()
        }
        Err(err) => wiki_error(&req, err),
    }
}

pub(super) async fn knowledge_publish(
    State(state): State<AppState>,
    Path(id): Path<String>,
    req: Request,
) -> Response {
    let engine = match wiki(&state, &req) {
        Ok(engine) => engine,
        Err(resp) => return resp,
    };
    match engine.get_document(&id, GetOptions::default()) {
        Ok(doc) => {
            let _ = engine.rebuild_or_sync_index_md();
            Json(serde_json::json!({
                "id": doc.info.rel_path,
                "docid": doc.info.docid,
                "title": doc.info.title,
                "body": doc.content,
                "revision": 1,
                "status": "verified",
                "source_ref": doc.info.source_rel_path,
                "relative_path": doc.info.rel_path,
            }))
            .into_response()
        }
        Err(err) => wiki_error(&req, err),
    }
}

pub(super) async fn knowledge_sources(State(state): State<AppState>, req: Request) -> Response {
    let engine = match wiki(&state, &req) {
        Ok(engine) => engine,
        Err(resp) => return resp,
    };
    match engine.multi_get("sources/**/*.md", MultiGetOptions::default()) {
        Ok(batch) => {
            let items: Vec<Value> = batch
                .docs
                .into_iter()
                .map(|doc| {
                    serde_json::json!({
                        "id": doc.info.rel_path,
                        "docid": doc.info.docid,
                        "version": 1,
                        "name": doc.info.title,
                        "provenance": doc.info.rel_path,
                        "sha256": doc.info.content_hash,
                        "size_bytes": doc.info.byte_size,
                        "media_type": "text/markdown",
                        "rel_path": doc.info.rel_path,
                    })
                })
                .collect();
            Json(serde_json::json!({ "items": items })).into_response()
        }
        Err(err) => wiki_error(&req, err),
    }
}

#[derive(Deserialize)]
pub(super) struct SourceImportPayload {
    pub name: String,
    pub content: String,
    #[serde(default)]
    pub provenance: Option<String>,
    #[serde(default)]
    pub domain: Option<String>,
    #[serde(default)]
    pub topic: Option<String>,
}

pub(super) async fn knowledge_import_source(
    State(state): State<AppState>,
    req: Request,
) -> Response {
    let (req, input): (_, SourceImportPayload) = match json(req).await {
        Ok(v) => v,
        Err(resp) => return resp,
    };
    let engine = match wiki(&state, &req) {
        Ok(engine) => engine,
        Err(resp) => return resp,
    };
    if input.name.trim().is_empty() || input.content.trim().is_empty() {
        return error(&req, StatusCode::BAD_REQUEST, "invalid_arguments");
    }
    let domain = input.domain.unwrap_or_else(|| "general".into());
    let topic = input.topic.unwrap_or_else(|| "notes".into());
    let mut header_bullets = Vec::new();
    if let Some(ref prov) = input.provenance {
        header_bullets.push(("Provenance".to_string(), prov.clone()));
    }
    let write_req = WriteTopicSourceRequest {
        topic_folder: format!("{domain}/{topic}"),
        file_stem: Some(slugify(&input.name, 64)),
        title: input.name.clone(),
        raw_body: input.content.clone(),
        body_heading: Some("Raw Content".into()),
        tags: Vec::new(),
        extra_frontmatter: Vec::new(),
        header_bullets,
        preview_chars: Some(400),
    };
    match engine.write_topic_source(write_req) {
        Ok(saved) => {
            let mut body = serde_json::json!({
                "id": saved.info.rel_path,
                "docid": saved.info.docid,
                "version": 1,
                "name": saved.info.title,
                "provenance": input.provenance.unwrap_or_else(|| saved.info.rel_path.clone()),
                "sha256": saved.info.content_hash,
                "size_bytes": saved.info.byte_size,
                "media_type": "text/markdown",
                "rel_path": saved.info.rel_path,
                "expected_wiki_path": saved.expected_wiki_path,
            });
            trigger_source_analysis(&state, &saved.info.rel_path, &mut body).await;
            (StatusCode::CREATED, Json(body)).into_response()
        }
        Err(err) => wiki_error(&req, err),
    }
}

pub(super) async fn knowledge_read_source(
    State(state): State<AppState>,
    Path(id): Path<String>,
    req: Request,
) -> Response {
    let engine = match wiki(&state, &req) {
        Ok(engine) => engine,
        Err(resp) => return resp,
    };
    match engine.get_document(&id, GetOptions::default()) {
        Ok(doc) => Json(serde_json::json!({
            "text": doc.content,
            "info": doc.info,
        }))
        .into_response(),
        Err(err) => wiki_error(&req, err),
    }
}
