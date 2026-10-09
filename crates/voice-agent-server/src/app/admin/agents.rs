//! Admin agents resources.
use super::*;

use crate::database::agents::{AgentChanges, NewAgent};
#[derive(Deserialize)]
struct CreateAgent {
    key: String,
    name: String,
    #[serde(default)]
    description: Option<String>,
}
#[derive(Deserialize)]
struct PatchAgent {
    #[serde(default)]
    key: Patch<String>,
    #[serde(default)]
    name: Patch<String>,
    #[serde(default)]
    description: Patch<String>,
    #[serde(default)]
    enabled: Patch<bool>,
}
pub(super) async fn create_agent(State(state): State<AppState>, request: Request) -> Response {
    let (request, body): (_, CreateAgent) = match json(request).await {
        Ok(v) => v,
        Err(e) => return e,
    };
    if !valid_key(&body.key)
        || !valid_text(&body.name, 128, false)
        || body
            .description
            .as_ref()
            .is_some_and(|v| !valid_text(v, 2048, true))
    {
        return error(&request, StatusCode::BAD_REQUEST, "validation_failed");
    };
    let database = match database(&state) {
        Ok(v) => v,
        Err(_) => {
            return error(
                &request,
                StatusCode::SERVICE_UNAVAILABLE,
                "database_unavailable",
            );
        }
    };
    if let Err(cause) = database
        .create_agent(
            NewAgent {
                key: &body.key,
                name: &body.name,
                description: body.description.as_deref(),
            },
            id(&request),
        )
        .await
    {
        return write_error(&request, cause);
    }
    get_agent_by(database, &body.key)
        .await
        .map(|agent| (StatusCode::CREATED, Json(agent)).into_response())
        .unwrap_or_else(|_| {
            error(
                &request,
                StatusCode::SERVICE_UNAVAILABLE,
                "database_unavailable",
            )
        })
}
pub(super) async fn get_agent(
    State(state): State<AppState>,
    Path(key): Path<String>,
    request: Request,
) -> Response {
    let database = match database(&state) {
        Ok(v) => v,
        Err(_) => {
            return error(
                &request,
                StatusCode::SERVICE_UNAVAILABLE,
                "database_unavailable",
            );
        }
    };
    match get_agent_by(database, &key).await {
        Ok(v) => Json(v).into_response(),
        Err(sqlx::Error::RowNotFound) => error(&request, StatusCode::NOT_FOUND, "not_found"),
        Err(error_value) => sql_error(&request, &error_value),
    }
}
use crate::database::agents::get_agent_by;
pub(super) async fn list_agents(
    State(state): State<AppState>,
    Query(query): Query<PageQuery>,
    request: Request,
) -> Response {
    let (page, page_size) = match page_bounds(query.page, query.page_size) {
        Ok(value) => value,
        Err(code) => return error(&request, StatusCode::BAD_REQUEST, code),
    };
    if query
        .sort
        .as_deref()
        .is_some_and(|v| !matches!(v, "key" | "name" | "-key" | "-name"))
    {
        return error(&request, StatusCode::BAD_REQUEST, "invalid_query");
    };
    let database = match database(&state) {
        Ok(v) => v,
        Err(_) => {
            return error(
                &request,
                StatusCode::SERVICE_UNAVAILABLE,
                "database_unavailable",
            );
        }
    };
    let rows = database
        .list_agents(query.enabled, query.sort.as_deref(), page, page_size)
        .await;
    match rows {Ok(items)=>Json(serde_json::json!({"items":items,"page":page,"page_size":page_size,"max_page_size":PAGE_MAX})).into_response(),Err(error_value)=>sql_error(&request, &error_value)}
}
#[axum::debug_handler]
pub(super) async fn patch_agent(
    State(state): State<AppState>,
    Path(key): Path<String>,
    request: Request,
) -> Response {
    let (request, body): (_, PatchAgent) = match json(request).await {
        Ok(v) => v,
        Err(e) => return e,
    };
    if !matches!(body.key, Patch::Absent) {
        return error(&request, StatusCode::BAD_REQUEST, "immutable_field");
    };
    let expected = match expected(request.headers()) {
        Ok(v) => v,
        Err(c) => return error(&request, StatusCode::BAD_REQUEST, c),
    };
    let database = match database(&state) {
        Ok(v) => v,
        Err(_) => {
            return error(
                &request,
                StatusCode::SERVICE_UNAVAILABLE,
                "database_unavailable",
            );
        }
    };
    let old = match get_agent_by(database, &key).await {
        Ok(v) => v,
        Err(sqlx::Error::RowNotFound) => {
            return error(&request, StatusCode::NOT_FOUND, "not_found");
        }
        Err(error_value) => return sql_error(&request, &error_value),
    };
    if old.revision != expected {
        {
            database
                .agent_conflict(id(&request), old.id, expected)
                .await;
            return error(&request, StatusCode::CONFLICT, "revision_conflict");
        }
    };
    let name = match body.name.value() {
        Some(Some(v)) if valid_text(&v, 128, false) => v,
        Some(_) => return error(&request, StatusCode::BAD_REQUEST, "validation_failed"),
        None => old.name.clone(),
    };
    let description = body.description.value().unwrap_or(old.description.clone());
    if description
        .as_ref()
        .is_some_and(|v| !valid_text(v, 2048, true))
    {
        return error(&request, StatusCode::BAD_REQUEST, "validation_failed");
    };
    let enabled = match body.enabled.value() {
        Some(Some(v)) => i64::from(v),
        Some(None) => return error(&request, StatusCode::BAD_REQUEST, "validation_failed"),
        None => old.enabled,
    };
    if let Err(cause) = database
        .update_agent(
            old.id,
            expected,
            AgentChanges {
                name: &name,
                description: description.as_deref(),
                enabled,
            },
            id(&request),
        )
        .await
    {
        return write_error(&request, cause);
    }
    // An Agent update re-scopes its sessions' profile snapshot; close them rather than hot-reload.
    if let Some(security) = security(&state) {
        security.invalidate_agent_speakers(old.id);
    }
    get_agent_by(database, &key)
        .await
        .map(|v| Json(v).into_response())
        .unwrap_or_else(|_| {
            error(
                &request,
                StatusCode::SERVICE_UNAVAILABLE,
                "database_unavailable",
            )
        })
}
