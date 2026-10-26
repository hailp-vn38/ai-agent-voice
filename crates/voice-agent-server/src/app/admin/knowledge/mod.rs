//! Admin HTTP handlers for Knowledge and Memory routes backed by `llm_wiki::WikiEngine`
//! and `orchestrator_flow_rs::TaskManager`.

mod memory;
mod raw;
#[cfg(test)]
mod tests;

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
    CollectionScope, GetOptions, HybridQueryRequest, MultiGetOptions, SearchOptions, WikiEngine,
    WikiError, WriteTopicSourceRequest, slugify,
};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::sync::Arc;

#[allow(clippy::result_large_err)]
pub(super) fn wiki(state: &AppState, req: &Request) -> Result<Arc<WikiEngine>, Response> {
    state.wiki_engine.as_ref().cloned().ok_or_else(|| {
        error(
            req,
            StatusCode::SERVICE_UNAVAILABLE,
            "knowledge_unavailable",
        )
    })
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
        _ => error(
            req,
            StatusCode::SERVICE_UNAVAILABLE,
            "knowledge_unavailable",
        ),
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
    let disabled = state
        .admin_task_control
        .lock()
        .expect("admin_task_control mutex poisoned")
        .background_enabled_override
        == Some(false);
    if disabled {
        return;
    }
    if let Some(ref tm) = state.task_manager {
        if let Ok(task) =
            crate::services::task_flow::submit_wiki_source_analysis(tm, rel_path).await
        {
            body["analysis"] = serde_json::json!({
                "job_id": task.id,
                "session_id": task.session_id,
                "turn_id": task.latest_turn_id,
                "replayed": false,
            });
        }
    }
    if let Some(ref notify) = state.autoflow_notify {
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

#[derive(Default, Deserialize)]
pub(super) struct SearchQuery {
    pub(super) q: Option<String>,
    pub(super) limit: Option<i64>,
    #[serde(default)]
    pub(super) space: Option<String>,
    #[serde(default)]
    pub(super) mode: Option<String>,
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
    center_id: Option<String>,
    #[serde(default)]
    depth: Option<usize>,
    #[serde(default)]
    limit: Option<usize>,
}

struct AggregatedGraphEdge {
    source_id: String,
    target_id: String,
    kind: String,
    lines: Vec<usize>,
    references: Vec<Value>,
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
            let all_node_ids: std::collections::HashSet<&str> =
                graph.nodes.iter().map(|n| n.rel_path.as_str()).collect();
            let candidate_nodes: Vec<_> = if q.scope.as_deref() == Some("local") {
                if let Some(center_id) = q
                    .center_id
                    .as_deref()
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                {
                    if all_node_ids.contains(center_id) {
                        let mut reachable = std::collections::HashSet::from([center_id.to_owned()]);
                        let mut frontier = std::collections::HashSet::from([center_id.to_owned()]);
                        let max_hops = q.depth.unwrap_or(1).clamp(1, 2);
                        for _ in 0..max_hops {
                            let mut next = std::collections::HashSet::new();
                            for e in &graph.edges {
                                let Some(ref target_id) = e.resolved_rel_path else {
                                    continue;
                                };
                                if e.source_rel_path == *target_id
                                    || !all_node_ids.contains(e.source_rel_path.as_str())
                                    || !all_node_ids.contains(target_id.as_str())
                                {
                                    continue;
                                }
                                if frontier.contains(&e.source_rel_path)
                                    && reachable.insert(target_id.clone())
                                {
                                    next.insert(target_id.clone());
                                }
                                if frontier.contains(target_id)
                                    && reachable.insert(e.source_rel_path.clone())
                                {
                                    next.insert(e.source_rel_path.clone());
                                }
                            }
                            if next.is_empty() {
                                break;
                            }
                            frontier = next;
                        }
                        graph
                            .nodes
                            .into_iter()
                            .filter(|n| reachable.contains(&n.rel_path))
                            .collect()
                    } else {
                        Vec::new()
                    }
                } else {
                    graph.nodes
                }
            } else {
                graph.nodes
            };

            let max_nodes = q.limit.unwrap_or(500).clamp(1, 5_000);
            let truncated = candidate_nodes.len() > max_nodes;
            let kept_nodes: Vec<_> = candidate_nodes.into_iter().take(max_nodes).collect();
            let kept_node_ids: std::collections::HashSet<&str> =
                kept_nodes.iter().map(|n| n.rel_path.as_str()).collect();

            let mut aggregated: Vec<AggregatedGraphEdge> = Vec::new();
            let mut edge_index: std::collections::HashMap<(String, String, String), usize> =
                std::collections::HashMap::new();
            let mut unresolved = Vec::new();

            for e in graph.edges {
                if let Some(target_id) = e.resolved_rel_path {
                    if e.source_rel_path == target_id
                        || !kept_node_ids.contains(e.source_rel_path.as_str())
                        || !kept_node_ids.contains(target_id.as_str())
                    {
                        continue;
                    }
                    let reference = serde_json::json!({
                        "line": e.line_number,
                        "raw_target": e.raw_target,
                        "alias": e.display_alias,
                        "anchor": e.target_heading,
                    });
                    let key = (
                        e.source_rel_path.clone(),
                        target_id.clone(),
                        e.link_type.clone(),
                    );
                    if let Some(&idx) = edge_index.get(&key) {
                        let entry = &mut aggregated[idx];
                        if !entry.lines.contains(&e.line_number) {
                            entry.lines.push(e.line_number);
                        }
                        entry.references.push(reference);
                    } else {
                        edge_index.insert(key, aggregated.len());
                        aggregated.push(AggregatedGraphEdge {
                            source_id: e.source_rel_path,
                            target_id,
                            kind: e.link_type,
                            lines: vec![e.line_number],
                            references: vec![reference],
                        });
                    }
                } else if kept_node_ids.contains(e.source_rel_path.as_str()) {
                    unresolved.push(serde_json::json!({
                        "source_id": e.source_rel_path,
                        "source_line": e.line_number,
                        "raw_target": e.raw_target,
                        "reason": "unresolved",
                    }));
                }
            }

            let nodes: Vec<Value> = kept_nodes
                .into_iter()
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
            let edges: Vec<Value> = aggregated
                .into_iter()
                .map(|e| {
                    serde_json::json!({
                        "source_id": e.source_id,
                        "target_id": e.target_id,
                        "kind": e.kind,
                        "lines": e.lines,
                        "references": e.references,
                    })
                })
                .collect();

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

#[derive(Clone, Copy)]
enum BrowserSpaceParam {
    Wiki,
    Raw,
}

impl BrowserSpaceParam {
    fn parse(raw: Option<&str>) -> Option<Self> {
        match raw.map(str::trim).filter(|s| !s.is_empty()) {
            None | Some("wiki") => Some(Self::Wiki),
            Some("raw") => Some(Self::Raw),
            Some(_) => None,
        }
    }

    fn from_collection(collection: CollectionScope) -> Self {
        if collection == CollectionScope::Sources {
            Self::Raw
        } else {
            Self::Wiki
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Wiki => "wiki",
            Self::Raw => "raw",
        }
    }

    fn collection_scope(self) -> CollectionScope {
        match self {
            Self::Wiki => CollectionScope::Wiki,
            Self::Raw => CollectionScope::Sources,
        }
    }

    fn list_glob(self) -> &'static str {
        match self {
            Self::Wiki => "wiki/**/*.md",
            Self::Raw => "sources/**/*.md",
        }
    }

    fn ui_path(self, rel_path: &str) -> String {
        match self {
            Self::Wiki => rel_path.to_owned(),
            Self::Raw => rel_path.replacen("sources/", "raw/", 1),
        }
    }
}

#[derive(Clone, Copy)]
enum SearchModeParam {
    Hybrid,
    Lex,
    Vec,
}

impl SearchModeParam {
    fn parse(raw: Option<&str>) -> Option<Self> {
        match raw.map(str::trim).filter(|s| !s.is_empty()) {
            None | Some("hybrid") => Some(Self::Hybrid),
            Some("lex") => Some(Self::Lex),
            Some("vec") => Some(Self::Vec),
            Some(_) => None,
        }
    }

    fn execute(
        self,
        engine: &WikiEngine,
        query: &str,
        space: BrowserSpaceParam,
        limit: usize,
    ) -> llm_wiki::WikiResult<Vec<llm_wiki::SearchResultItem>> {
        let collections = vec![space.collection_scope()];
        match self {
            Self::Hybrid => engine.query(HybridQueryRequest {
                query: Some(query.to_owned()),
                collections,
                limit,
                ..Default::default()
            }),
            Self::Lex => engine.search_lex(
                query,
                SearchOptions {
                    collections,
                    filter: None,
                    limit,
                    min_score: 0.0,
                },
            ),
            Self::Vec => engine.search_vec(
                query,
                SearchOptions {
                    collections,
                    filter: None,
                    limit,
                    min_score: 0.0,
                },
            ),
        }
    }
}

fn base_page_item_json(
    collection: CollectionScope,
    rel_path: &str,
    docid: &str,
    title: &str,
    source_ref: impl Into<Value>,
    excerpt: &str,
) -> Value {
    let space = BrowserSpaceParam::from_collection(collection);
    serde_json::json!({
        "id": rel_path,
        "docid": docid,
        "space": space.as_str(),
        "path": space.ui_path(rel_path),
        "title": title,
        "revision": 1,
        "status": "verified",
        "source_ref": source_ref.into(),
        "excerpt": excerpt,
        "relative_path": rel_path,
    })
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
    let Some(space) = BrowserSpaceParam::parse(q.space.as_deref()) else {
        return error(&req, StatusCode::BAD_REQUEST, "invalid_arguments");
    };
    let Some(mode) = SearchModeParam::parse(q.mode.as_deref()) else {
        return error(&req, StatusCode::BAD_REQUEST, "invalid_arguments");
    };
    let limit = q.limit.unwrap_or(50).clamp(1, 200) as usize;
    if let Some(query) = q.q.as_deref().map(str::trim).filter(|query| !query.is_empty()) {
        match mode.execute(&engine, query, space, limit) {
            Ok(results) => {
                let items: Vec<Value> = results
                    .into_iter()
                    .map(|item| {
                        let mut json = base_page_item_json(
                            item.collection,
                            &item.rel_path,
                            &item.docid,
                            &item.title,
                            item.virtual_uri,
                            &item.snippet,
                        );
                        if let Some(obj) = json.as_object_mut() {
                            obj.insert("score".to_owned(), serde_json::json!(item.score));
                            obj.insert("match_line".to_owned(), serde_json::json!(item.match_line));
                            obj.insert("context".to_owned(), serde_json::json!(item.context));
                        }
                        json
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
        match engine.multi_get(space.list_glob(), MultiGetOptions::default()) {
            Ok(batch) => {
                let items: Vec<Value> = batch
                    .docs
                    .into_iter()
                    .take(limit)
                    .map(|doc| {
                        let excerpt: String = doc.content.chars().take(240).collect();
                        base_page_item_json(
                            doc.info.collection,
                            &doc.info.rel_path,
                            &doc.info.docid,
                            &doc.info.title,
                            doc.info.source_rel_path,
                            &excerpt,
                        )
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
