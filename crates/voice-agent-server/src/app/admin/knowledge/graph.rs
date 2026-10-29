use super::{wiki, wiki_error, *};
use llm_wiki::CollectionScope;
use serde::Deserialize;

#[derive(Deserialize)]
pub(in crate::app::admin) struct GraphQuery {
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

pub(in crate::app::admin) async fn knowledge_graph(
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

pub(in crate::app::admin) async fn knowledge_health(
    State(state): State<AppState>,
    req: Request,
) -> Response {
    let engine = match wiki(&state, &req) {
        Ok(engine) => engine,
        Err(resp) => return resp,
    };
    match (engine.status(), engine.lint_wiki()) {
        (Ok(status), Ok(lint)) => Json(serde_json::json!({
            "health": if lint.is_healthy { "ready" } else { "needs_attention" },
            "document_count": status.total_documents,
            "last_complete_scan_at": now(),
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

pub(in crate::app::admin) async fn knowledge_rebuild(
    State(state): State<AppState>,
    req: Request,
) -> Response {
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
