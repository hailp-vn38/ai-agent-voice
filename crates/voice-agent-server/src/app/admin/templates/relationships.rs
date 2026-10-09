//! Read and unlink operations for persisted Template relationships.

use super::*;

#[allow(clippy::result_large_err)] // Handler callers return this HTTP response unchanged.
fn relationship_page(query: PageQuery, request: &Request) -> Result<(u32, u32), Response> {
    match page_bounds(query.page, query.page_size) {
        Ok(value) if query.enabled.is_none() && query.sort.is_none() => Ok(value),
        _ => Err(error(request, StatusCode::BAD_REQUEST, "invalid_query")),
    }
}

pub(in crate::app::admin) async fn list_agent_templates(
    State(state): State<AppState>,
    Path(agent_key): Path<String>,
    Query(query): Query<PageQuery>,
    request: Request,
) -> Response {
    let (page, page_size) = match relationship_page(query, &request) {
        Ok(value) => value,
        Err(response) => return response,
    };
    let database = match database(&state) {
        Ok(value) => value,
        Err(response) => return response,
    };
    let agent = match get_agent_by(database, &agent_key).await {
        Ok(value) => value,
        Err(sqlx::Error::RowNotFound) => {
            return error(&request, StatusCode::NOT_FOUND, "not_found");
        }
        Err(error_value) => return sql_error(&request, &error_value),
    };
    let rows = database.agent_templates(agent.id, page, page_size).await;
    match rows {
        Ok(rows) => Json(serde_json::json!({
            "agent_key": agent_key,
            "revision": agent.revision,
            "page": page,
            "page_size": page_size,
            "max_page_size": PAGE_MAX,
            "items": rows.into_iter().map(|(key, name, language, enabled, is_default)| serde_json::json!({
                "key": key,
                "name": name,
                "language": language,
                "enabled": enabled != 0,
                "is_default": is_default != 0,
            })).collect::<Vec<_>>(),
        }))
        .into_response(),
        Err(error_value) => sql_error(&request, &error_value),
    }
}

pub(in crate::app::admin) async fn list_template_agents(
    State(state): State<AppState>,
    Path(template_key): Path<String>,
    Query(query): Query<PageQuery>,
    request: Request,
) -> Response {
    let (page, page_size) = match relationship_page(query, &request) {
        Ok(value) => value,
        Err(response) => return response,
    };
    let database = match database(&state) {
        Ok(value) => value,
        Err(response) => return response,
    };
    let template = match template_by(database, &template_key).await {
        Ok(value) => value,
        Err(sqlx::Error::RowNotFound) => {
            return error(&request, StatusCode::NOT_FOUND, "not_found");
        }
        Err(error_value) => return sql_error(&request, &error_value),
    };
    let (total, rows) = match database.template_agents(template.id, page, page_size).await {
        Ok(value) => value,
        Err(cause) => return write_error(&request, cause),
    };
    let rows: Result<_, sqlx::Error> = Ok(rows);
    match rows {
        Ok(rows) => Json(serde_json::json!({
            "template_key": template_key,
            "revision": template.revision,
            "page": page,
            "page_size": page_size,
            "max_page_size": PAGE_MAX,
            "total": total,
            "items": rows.into_iter().map(|(key, name, enabled, is_default)| serde_json::json!({
                "key": key,
                "name": name,
                "enabled": enabled != 0,
                "is_default": is_default != 0,
            })).collect::<Vec<_>>(),
        }))
        .into_response(),
        Err(error_value) => sql_error(&request, &error_value),
    }
}

pub(in crate::app::admin) async fn list_template_providers(
    State(state): State<AppState>,
    Path(template_key): Path<String>,
    request: Request,
) -> Response {
    let database = match database(&state) {
        Ok(value) => value,
        Err(response) => return response,
    };
    let template = match template_by(database, &template_key).await {
        Ok(value) => value,
        Err(sqlx::Error::RowNotFound) => {
            return error(&request, StatusCode::NOT_FOUND, "not_found");
        }
        Err(error_value) => return sql_error(&request, &error_value),
    };
    let rows = database.template_providers(template.id).await;
    match rows {
        Ok(rows) => Json(serde_json::json!({
            "template_key": template_key,
            "revision": template.revision,
            "bindings": rows.into_iter().map(|(provider_type, provider_key, enabled)| (
                provider_type,
                serde_json::json!({"provider_key": provider_key, "enabled": enabled != 0}),
            )).collect::<serde_json::Map<String, Value>>(),
        }))
        .into_response(),
        Err(error_value) => sql_error(&request, &error_value),
    }
}

pub(in crate::app::admin) async fn unlink_template_provider(
    State(state): State<AppState>,
    Path((key, provider_type)): Path<(String, String)>,
    request: Request,
) -> Response {
    if !matches!(
        provider_type.as_str(),
        "vad" | "asr" | "llm" | "tts" | "speaker"
    ) {
        return error(&request, StatusCode::BAD_REQUEST, "validation_failed");
    }
    let expected = match expected(request.headers()) {
        Ok(value) => value,
        Err(code) => return error(&request, StatusCode::BAD_REQUEST, code),
    };
    let database = match database(&state) {
        Ok(value) => value,
        Err(response) => return response,
    };
    if let Err(cause) = database
        .unlink_template_provider(&key, &provider_type, expected, id(&request))
        .await
    {
        return write_error(&request, cause);
    }
    get_template(State(state), Path(key), request).await
}

pub(in crate::app::admin) async fn unlink_agent_template(
    State(state): State<AppState>,
    Path((agent_key, template_key)): Path<(String, String)>,
    request: Request,
) -> Response {
    let expected = match expected(request.headers()) {
        Ok(value) => value,
        Err(code) => return error(&request, StatusCode::BAD_REQUEST, code),
    };
    let database = match database(&state) {
        Ok(value) => value,
        Err(response) => return response,
    };
    if let Err(cause) = database
        .unlink_agent_template(&agent_key, &template_key, expected, id(&request))
        .await
    {
        return write_error(&request, cause);
    }
    match get_agent_by(database, &agent_key).await {
        Ok(agent) => Json(agent).into_response(),
        Err(error_value) => sql_error(&request, &error_value),
    }
}
