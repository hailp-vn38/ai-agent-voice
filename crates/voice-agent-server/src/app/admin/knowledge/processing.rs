//! Admin HTTP handlers for Knowledge Processing routes backed by `orchestrator-agent` and `llm-wiki-rs`.
use super::{wiki, wiki_error, *};
use serde::Deserialize;
use sha2::{Digest, Sha256};

async fn publish_wiki_lint_if_needed(state: &AppState, lint: &llm_wiki::WikiLintReport) {
    if lint.is_healthy {
        return;
    }
    let Some(ref hub) = state.notification_hub else {
        return;
    };
    let issue_count = lint.broken_links.len()
        + lint.orphan_wiki_pages.len()
        + lint.uningested_sources.len()
        + lint.stale_wiki_pages.len();
    let count = issue_count.max(1);
    let payload = serde_json::to_vec(&(
        &lint.broken_links,
        &lint.orphan_wiki_pages,
        &lint.uningested_sources,
        &lint.stale_wiki_pages,
    ))
    .unwrap_or_default();
    let digest = Sha256::digest(&payload);
    let fingerprint: String = digest.iter().take(12).map(|b| format!("{b:02x}")).collect();
    let summary = format!(
        "Phát hiện {} liên kết hỏng, {} trang mồ côi, {} nguồn chưa biên dịch.",
        lint.broken_links.len(),
        lint.orphan_wiki_pages.len(),
        lint.uningested_sources.len(),
    );
    let input = crate::services::notification_hub::bridge::from_wiki_lint_attention(
        count,
        &fingerprint,
        &summary,
    );
    let _ = hub.publish(input).await;
}

pub(in crate::app::admin) async fn analyze(
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
pub(in crate::app::admin) struct ReviewsQuery {
    #[serde(default)]
    limit: Option<usize>,
}

pub(in crate::app::admin) async fn reviews(
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
        (Ok(logs), Ok(lint)) => {
            publish_wiki_lint_if_needed(&state, &lint).await;
            Json(serde_json::json!({
                "items": logs,
                "lint": lint,
                "next_cursor": Value::Null,
            }))
            .into_response()
        }
        (Err(err), _) | (_, Err(err)) => wiki_error(&req, err),
    }
}

pub(in crate::app::admin) async fn review(
    State(state): State<AppState>,
    Path(id): Path<String>,
    req: Request,
) -> Response {
    let engine = match wiki(&state, &req) {
        Ok(engine) => engine,
        Err(resp) => return resp,
    };
    match engine.lint_wiki() {
        Ok(lint) => {
            publish_wiki_lint_if_needed(&state, &lint).await;
            Json(serde_json::json!({
                "id": id,
                "status": if lint.is_healthy { "approved" } else { "needs_attention" },
                "lint": lint,
            }))
            .into_response()
        }
        Err(err) => wiki_error(&req, err),
    }
}

pub(in crate::app::admin) async fn approve(
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

pub(in crate::app::admin) async fn reject(
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

pub(in crate::app::admin) async fn resolve_draft(
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
