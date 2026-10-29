use super::{task_failure, task_manager, *};
use crate::services::task_flow::project_artifact_catalog_page;
use orchestrator_flow_rs::{
    TaskArtifactCatalogPage, TaskArtifactFilter, TaskArtifactKind, TaskArtifactRole,
};
use serde::Deserialize;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(in crate::app::admin) struct ArtifactQuery {
    page: Option<u32>,
    page_size: Option<u32>,
    kind: Option<String>,
    role: Option<String>,
    job_id: Option<String>,
    task_id: Option<String>,
    flow_id: Option<String>,
    agent_id: Option<String>,
    q: Option<String>,
}

pub(in crate::app::admin) async fn list_artifacts(
    State(state): State<AppState>,
    Query(query): Query<ArtifactQuery>,
    request: Request,
) -> Response {
    let page = query.page.unwrap_or(1).max(1);
    let size = query.page_size.unwrap_or(30).clamp(1, 200);
    let Some(tm) = state.task_manager.as_ref() else {
        let empty = TaskArtifactCatalogPage::default();
        return Json(project_artifact_catalog_page(&empty, page, size)).into_response();
    };

    let kind = match query
        .kind
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty() && *s != "all")
    {
        Some(raw) => match TaskArtifactKind::parse(raw) {
            Some(k) => Some(k),
            None => return error(&request, StatusCode::BAD_REQUEST, "invalid_arguments"),
        },
        None => None,
    };
    let role = match query
        .role
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty() && *s != "all")
    {
        Some(raw) => match TaskArtifactRole::parse(raw) {
            Some(r) => Some(r),
            None => return error(&request, StatusCode::BAD_REQUEST, "invalid_arguments"),
        },
        None => None,
    };

    let offset = (page - 1).saturating_mul(size);
    let filter = TaskArtifactFilter {
        kind,
        role,
        task_id: query.job_id.or(query.task_id),
        flow_id: query.flow_id,
        producer_agent_id: query.agent_id,
        query: query.q,
        limit: Some(size),
        offset: Some(offset),
    };

    match tm.list_all_artifacts(&filter).await {
        Ok(catalog_page) => {
            Json(project_artifact_catalog_page(&catalog_page, page, size)).into_response()
        }
        Err(e) => task_failure(&request, e),
    }
}

pub(in crate::app::admin) async fn artifact(
    State(state): State<AppState>,
    Path((id, artifact_id)): Path<(String, String)>,
    request: Request,
) -> Response {
    let tm = match task_manager(&state, &request) {
        Ok(tm) => tm,
        Err(resp) => return resp,
    };
    match tm
        .read_task_artifact_bytes_any_scope(&id, &artifact_id)
        .await
    {
        Ok((meta, bytes)) => {
            let safe_filename: String = meta
                .name
                .chars()
                .filter(|c| !matches!(c, '"' | '\\' | '\r' | '\n'))
                .collect();
            let disposition = format!("inline; filename=\"{safe_filename}\"");
            let etag = format!("\"{}\"", meta.sha256);
            (
                StatusCode::OK,
                [
                    (axum::http::header::CONTENT_TYPE, meta.mime_type),
                    (axum::http::header::CONTENT_DISPOSITION, disposition),
                    (axum::http::header::ETAG, etag),
                ],
                bytes,
            )
                .into_response()
        }
        Err(e) => task_failure(&request, e),
    }
}
