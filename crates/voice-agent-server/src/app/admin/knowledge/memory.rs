//! Memory Fact & Summary handlers mapped onto `llm_wiki::WikiEngine` (`entities/` + compact index).

use super::{
    PageLines, SearchQuery, format_wiki_markdown, knowledge_health, knowledge_read,
    knowledge_rebuild, knowledge_search, resolve_wiki_scoped_path, wiki, wiki_error,
};
use crate::app::{
    AppState,
    admin::{Json, Path, Query, Request, Response, State, StatusCode, error, json},
};
use llm_wiki::{CollectionScope, slugify};
use serde::Deserialize;
use serde_json::Value;

pub(in crate::app::admin) async fn memory_health(
    State(state): State<AppState>,
    req: Request,
) -> Response {
    knowledge_health(State(state), req).await
}

pub(in crate::app::admin) async fn memory_rebuild(
    State(state): State<AppState>,
    req: Request,
) -> Response {
    knowledge_rebuild(State(state), req).await
}

#[derive(Deserialize)]
pub(in crate::app::admin) struct MemoryQuery {
    q: Option<String>,
    limit: Option<i64>,
}

pub(in crate::app::admin) async fn memory_recall(
    State(state): State<AppState>,
    Query(q): Query<MemoryQuery>,
    req: Request,
) -> Response {
    knowledge_search(
        State(state),
        Query(SearchQuery {
            q: q.q,
            limit: q.limit,
        }),
        req,
    )
    .await
}

pub(in crate::app::admin) async fn memory_summary(
    State(state): State<AppState>,
    req: Request,
) -> Response {
    let engine = match wiki(&state, &req) {
        Ok(engine) => engine,
        Err(resp) => return resp,
    };
    match engine.read_compact_index_md(4_000) {
        Ok(summary) => axum::response::IntoResponse::into_response(Json::<Value>(
            serde_json::json!({ "summary": summary }),
        )),
        Err(err) => wiki_error(&req, err),
    }
}

pub(in crate::app::admin) async fn memory_read(
    State(state): State<AppState>,
    Path(id): Path<String>,
    req: Request,
) -> Response {
    knowledge_read(
        State(state),
        Path(id),
        Query(PageLines {
            from_line: None,
            max_lines: None,
        }),
        req,
    )
    .await
}

#[derive(Deserialize)]
struct FactWritePayload {
    title: String,
    body: String,
    #[serde(default)]
    source_ref: Option<String>,
    #[serde(default)]
    confirmed: Option<bool>,
}

pub(in crate::app::admin) async fn memory_create(
    State(state): State<AppState>,
    req: Request,
) -> Response {
    let (req, input): (_, FactWritePayload) = match json(req).await {
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
    let rel_path = format!("entities/{}.md", slugify(&input.title, 64));
    let markdown = format_wiki_markdown(&input.title, &input.body, input.source_ref.as_deref());
    match engine.write_wiki_file(&rel_path, &markdown) {
        Ok(info) => {
            let _ = engine.rebuild_or_sync_index_md();
            axum::response::IntoResponse::into_response((
                StatusCode::CREATED,
                Json(serde_json::json!({
                    "id": info.rel_path,
                    "docid": info.docid,
                    "title": info.title,
                    "body": input.body,
                    "revision": 1,
                    "status": "verified",
                    "confirmed": input.confirmed.unwrap_or(true),
                })),
            ))
        }
        Err(err) => wiki_error(&req, err),
    }
}

pub(in crate::app::admin) async fn memory_update(
    State(state): State<AppState>,
    Path(id): Path<String>,
    req: Request,
) -> Response {
    let (req, input): (_, FactWritePayload) = match json(req).await {
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
            axum::response::IntoResponse::into_response(Json(serde_json::json!({
                "id": info.rel_path,
                "docid": info.docid,
                "title": info.title,
                "body": input.body,
                "revision": 1,
                "status": "verified",
                "confirmed": input.confirmed.unwrap_or(true),
            })))
        }
        Err(err) => wiki_error(&req, err),
    }
}

pub(in crate::app::admin) async fn memory_forget(
    State(state): State<AppState>,
    Path(id): Path<String>,
    req: Request,
) -> Response {
    let engine = match wiki(&state, &req) {
        Ok(engine) => engine,
        Err(resp) => return resp,
    };
    let target_rel = resolve_wiki_scoped_path(&engine, &id);
    match engine.delete_file(CollectionScope::Wiki, &target_rel) {
        Ok(deleted) => {
            let _ = engine.rebuild_or_sync_index_md();
            axum::response::IntoResponse::into_response(Json(serde_json::json!({
                "id": id,
                "status": "forgotten",
                "deleted": deleted,
            })))
        }
        Err(err) => wiki_error(&req, err),
    }
}
