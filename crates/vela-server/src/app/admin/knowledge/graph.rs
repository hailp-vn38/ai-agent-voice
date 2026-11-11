use super::answer_graph::{
    AnswerGraphLimits, Evidence, EvidenceTier, aggregate_graph_edges, answer_graph,
};
use super::{wiki, wiki_error, *};
use llm_wiki::{CollectionScope, HybridQueryRequest};
use serde::Deserialize;
use std::collections::HashSet;

const QUERY_GRAPH_MIN_SCORE: f32 = 0.12;
const QUERY_GRAPH_MAX_NODES: usize = 60;

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
    #[serde(default)]
    q: Option<String>,
}

pub(in crate::app::admin) async fn knowledge_graph(
    State(state): State<AppState>,
    Query(q): Query<GraphQuery>,
    req: Request,
) -> Response {
    let query = if q.scope.as_deref() == Some("query") {
        match q
            .q
            .as_deref()
            .map(str::trim)
            .filter(|query| !query.is_empty())
        {
            Some(query) => Some(query.to_owned()),
            None => return error(&req, StatusCode::BAD_REQUEST, "invalid_input"),
        }
    } else {
        None
    };
    let engine = match wiki(&state, &req) {
        Ok(engine) => engine,
        Err(resp) => return resp,
    };
    if q.scope.as_deref() == Some("query") {
        let engine_for_query = engine.clone();
        let query_for_search = query.expect("query is validated for query scope");
        let results = match tokio::task::spawn_blocking(move || {
            engine_for_query.query(HybridQueryRequest {
                query: Some(query_for_search),
                collections: vec![CollectionScope::Wiki, CollectionScope::Sources],
                limit: 12,
                min_score: QUERY_GRAPH_MIN_SCORE,
                rerank: false,
                ..Default::default()
            })
        })
        .await
        {
            Ok(Ok(results)) => results,
            Ok(Err(err)) => return wiki_error(&req, err),
            Err(_) => {
                return error(
                    &req,
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "knowledge_unavailable",
                );
            }
        };
        let engine_for_graph = engine.clone();
        let data = match tokio::task::spawn_blocking(move || engine_for_graph.get_graph(None)).await
        {
            Ok(Ok(data)) => data,
            Ok(Err(err)) => return wiki_error(&req, err),
            Err(_) => {
                return error(
                    &req,
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "knowledge_unavailable",
                );
            }
        };
        let mut evidence = Evidence::default();
        for result in results {
            evidence.record(
                &result.rel_path,
                EvidenceTier::Retrieved,
                Some(result.score),
            );
        }
        return Json(answer_graph(
            data,
            &evidence,
            AnswerGraphLimits {
                max_nodes: QUERY_GRAPH_MAX_NODES,
            },
        ))
        .into_response();
    }
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
            let kept_node_ids: HashSet<String> =
                kept_nodes.iter().map(|n| n.rel_path.clone()).collect();
            let mut unresolved = Vec::new();

            for e in &graph.edges {
                if e.resolved_rel_path.is_none() && kept_node_ids.contains(&e.source_rel_path) {
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
            let edges = aggregate_graph_edges(&graph.edges, &kept_node_ids);

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
            // The engine does not record scan times. Reporting the request time made every
            // poll look like a change, so clients reloaded the workspace every interval.
            "last_complete_scan_at": serde_json::Value::Null,
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
    if sync.indexed_new + sync.updated + sync.removed > 0
        && let Some(ref notify) = state.autoflow_notify
    {
        notify.notify_one();
    }
    let indexed = sync.indexed_new + sync.updated + sync.unchanged;
    Json(serde_json::json!({
        "indexed": indexed,
        "sync": sync,
        "embed": embed,
    }))
    .into_response()
}
