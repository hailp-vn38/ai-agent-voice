//! Admin HTTP handlers for Knowledge Processing routes backed by `orchestrator-agent` and `llm-wiki-rs`.
use super::knowledge::{wiki, wiki_error};
use super::*;
use serde::Deserialize;

pub(super) async fn analyze(
    State(state): State<AppState>,
    Path(source_id): Path<String>,
    req: Request,
) -> Response {
    let _engine = match wiki(&state, &req) {
        Ok(engine) => engine,
        Err(resp) => return resp,
    };
    let Some(ref tm) = state.task_manager else {
        return error(&req, StatusCode::SERVICE_UNAVAILABLE, "runtime_unavailable");
    };
    match crate::services::task_flow::submit_wiki_source_analysis(tm, &source_id).await {
        Ok(task) => (
            StatusCode::ACCEPTED,
            Json(serde_json::json!({
                "job_id": task.id,
                "session_id": task.session_id,
                "turn_id": task.latest_turn_id,
                "source_ref": source_id,
                "replayed": false,
            })),
        )
            .into_response(),
        Err(_) => error(&req, StatusCode::SERVICE_UNAVAILABLE, "runtime_unavailable"),
    }
}

#[derive(Deserialize)]
pub(super) struct ReviewsQuery {
    #[serde(default)]
    limit: Option<usize>,
}

pub(super) async fn reviews(
    State(state): State<AppState>,
    Query(query): Query<ReviewsQuery>,
    req: Request,
) -> Response {
    let engine = match wiki(&state, &req) {
        Ok(engine) => engine,
        Err(resp) => return resp,
    };
    let limit = query.limit.unwrap_or(20).clamp(1, 200);
    match (engine.read_recent_logs(limit), engine.lint_wiki()) {
        (Ok(logs), Ok(lint)) => Json(serde_json::json!({
            "items": logs,
            "lint": lint,
            "next_cursor": Value::Null,
        }))
        .into_response(),
        (Err(err), _) | (_, Err(err)) => wiki_error(&req, err),
    }
}

pub(super) async fn review(
    State(state): State<AppState>,
    Path(id): Path<String>,
    req: Request,
) -> Response {
    let engine = match wiki(&state, &req) {
        Ok(engine) => engine,
        Err(resp) => return resp,
    };
    match engine.lint_wiki() {
        Ok(lint) => Json(serde_json::json!({
            "id": id,
            "status": if lint.is_healthy { "approved" } else { "needs_attention" },
            "lint": lint,
        }))
        .into_response(),
        Err(err) => wiki_error(&req, err),
    }
}

pub(super) async fn approve(
    State(state): State<AppState>,
    Path(id): Path<String>,
    req: Request,
) -> Response {
    let engine = match wiki(&state, &req) {
        Ok(engine) => engine,
        Err(resp) => return resp,
    };
    let _ = engine.rebuild_or_sync_index_md();
    Json(serde_json::json!({
        "id": id,
        "status": "approved",
    }))
    .into_response()
}

pub(super) async fn reject(
    State(state): State<AppState>,
    Path(id): Path<String>,
    req: Request,
) -> Response {
    let _engine = match wiki(&state, &req) {
        Ok(engine) => engine,
        Err(resp) => return resp,
    };
    Json(serde_json::json!({
        "id": id,
        "status": "rejected",
    }))
    .into_response()
}

pub(super) async fn resolve_draft(
    State(state): State<AppState>,
    Path(wiki_id): Path<String>,
    req: Request,
) -> Response {
    let engine = match wiki(&state, &req) {
        Ok(engine) => engine,
        Err(resp) => return resp,
    };
    let _ = engine.rebuild_or_sync_index_md();
    Json(serde_json::json!({
        "id": wiki_id,
        "status": "resolved",
    }))
    .into_response()
}
