//! Admin HTTP handlers for Knowledge Processing routes backed by `orchestrator-agent` and `llm-wiki-rs`.
use super::knowledge::{wiki, wiki_error};
use super::*;
use serde::Deserialize;

fn settings_payload(state: &AppState) -> Value {
    let runtime_ready = state.wiki_engine.is_some() && state.task_manager.is_some();
    let provider_ref = "orchestrator:default";
    let (overrides, revision) = {
        let guard = state
            .admin_task_control
            .lock()
            .expect("admin_task_control mutex poisoned");
        (
            guard.knowledge_processing_overrides.clone(),
            guard.knowledge_processing_revision,
        )
    };
    let mut base = serde_json::json!({
        "enabled": state.config.wiki.enabled && state.config.orchestrator.enabled,
        "analysis_provider_ref": provider_ref,
        "trigger_mode": "auto_after_import",
        "execution_version": 2,
        "max_pages_per_source": 5,
        "revision": revision,
        "runtime_ready": runtime_ready,
        "llm_providers": [
            {
                "reference": provider_ref,
                "name": state.config.orchestrator.model,
                "runtime_ready": runtime_ready,
                "structured_qualified": true,
                "tool_calling": true,
                "availability": if runtime_ready { "ready" } else { "unavailable" },
            }
        ],
    });
    if let Some(obj) = base.as_object_mut() {
        for (k, v) in overrides {
            obj.insert(k, v);
        }
        obj.insert("revision".into(), serde_json::json!(revision));
    }
    base
}

pub(super) async fn settings(State(state): State<AppState>) -> Response {
    Json(settings_payload(&state)).into_response()
}

pub(super) async fn save_settings(State(state): State<AppState>, req: Request) -> Response {
    let expected_rev = if req.headers().contains_key(header::IF_MATCH) {
        match expected(req.headers()) {
            Ok(v) => Some(v),
            Err(code) => return error(&req, StatusCode::BAD_REQUEST, code),
        }
    } else {
        None
    };
    let (req, body): (_, Value) = match json(req).await {
        Ok(v) => v,
        Err(resp) => return resp,
    };
    let Some(obj) = body.as_object() else {
        return error(&req, StatusCode::BAD_REQUEST, "invalid_arguments");
    };
    {
        let mut guard = state
            .admin_task_control
            .lock()
            .expect("admin_task_control mutex poisoned");
        if let Some(exp) = expected_rev
            && guard.knowledge_processing_revision != exp
        {
            return error(&req, StatusCode::CONFLICT, "revision_conflict");
        }
        for (k, v) in obj {
            if matches!(
                k.as_str(),
                "enabled"
                    | "analysis_provider_ref"
                    | "trigger_mode"
                    | "execution_version"
                    | "max_pages_per_source"
            ) {
                guard
                    .knowledge_processing_overrides
                    .insert(k.clone(), v.clone());
            }
        }
        guard.knowledge_processing_revision = guard.knowledge_processing_revision.saturating_add(1);
    }
    Json(settings_payload(&state)).into_response()
}

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
