//! Admin templates resources.
use super::*;
use crate::database::agents::get_agent_by;

mod relationships;

pub(super) use relationships::{
    list_agent_templates, list_template_agents, list_template_providers, unlink_agent_template,
    unlink_template_provider,
};

use crate::database::templates::{TemplateChanges, TemplateFilters, TemplateInput, template_by};
#[derive(Deserialize)]
struct CreateTemplate {
    key: String,
    name: String,
    #[serde(default)]
    description: Option<String>,
    language: String,
    prompt: String,
}
#[derive(Deserialize)]
struct PatchTemplate {
    #[serde(default)]
    key: Patch<String>,
    #[serde(default)]
    name: Patch<String>,
    #[serde(default)]
    description: Patch<String>,
    #[serde(default)]
    language: Patch<String>,
    #[serde(default)]
    prompt: Patch<String>,
    #[serde(default)]
    enabled: Patch<bool>,
}
#[derive(Deserialize)]
struct ProviderBinding {
    provider_key: String,
}

pub(super) async fn create_template(State(state): State<AppState>, request: Request) -> Response {
    let (request, body): (_, CreateTemplate) = match json(request).await {
        Ok(v) => v,
        Err(e) => return e,
    };
    if !valid_key(&body.key)
        || !valid_text(&body.name, 128, false)
        || !valid_text(&body.language, 32, false)
        || !valid_text(&body.prompt, 64 * 1024, false)
        || body
            .description
            .as_ref()
            .is_some_and(|v| !valid_text(v, 2048, true))
    {
        return error(&request, StatusCode::BAD_REQUEST, "validation_failed");
    }
    let database = match database(&state) {
        Ok(v) => v,
        Err(e) => return e,
    };
    if let Err(cause) = database
        .create_template(
            TemplateInput {
                key: &body.key,
                name: &body.name,
                description: body.description.as_deref(),
                language: &body.language,
                prompt: &body.prompt,
            },
            id(&request),
        )
        .await
    {
        return write_error(&request, cause);
    }
    match template_by(database, &body.key).await {
        Ok(v) => (StatusCode::CREATED, Json(v)).into_response(),
        Err(e) => sql_error(&request, &e),
    }
}
pub(super) async fn get_template(
    State(state): State<AppState>,
    Path(key): Path<String>,
    request: Request,
) -> Response {
    let database = match database(&state) {
        Ok(v) => v,
        Err(e) => return e,
    };
    match template_by(database, &key).await {
        Ok(v) => Json(v).into_response(),
        Err(sqlx::Error::RowNotFound) => error(&request, StatusCode::NOT_FOUND, "not_found"),
        Err(e) => sql_error(&request, &e),
    }
}

pub(super) async fn list_templates(
    State(state): State<AppState>,
    Query(query): Query<TemplateListQuery>,
    request: Request,
) -> Response {
    let (page, size) = match page_bounds(query.page, query.page_size) {
        Ok(v) => v,
        Err(c) => return error(&request, StatusCode::BAD_REQUEST, c),
    };
    let database = match database(&state) {
        Ok(v) => v,
        Err(e) => return e,
    };
    if query
        .sort
        .as_deref()
        .is_some_and(|sort| !matches!(sort, "key" | "-key" | "name" | "-name" | "language"))
    {
        return error(&request, StatusCode::BAD_REQUEST, "invalid_query");
    }
    let filters = match TemplateFilters::new(query.enabled, query.q.clone(), query.language.clone())
    {
        Ok(value) => value,
        Err(()) => return error(&request, StatusCode::BAD_REQUEST, "invalid_query"),
    };
    match database.list_templates(&filters, query.sort.as_deref(), page, size).await {
        Ok((total, items)) => Json(serde_json::json!({"items":items,"page":page,"page_size":size,"max_page_size":PAGE_MAX,"total":total,"total_pages":template_total_pages(total, size)})).into_response(),
        Err(value) => sql_error(&request, &value),
    }
}

#[derive(Deserialize)]
pub(super) struct TemplateListQuery {
    #[serde(default)]
    page: Option<u32>,
    #[serde(default)]
    page_size: Option<u32>,
    #[serde(default)]
    enabled: Option<bool>,
    #[serde(default)]
    q: Option<String>,
    #[serde(default)]
    language: Option<String>,
    #[serde(default)]
    sort: Option<String>,
}
fn template_total_pages(total: i64, page_size: u32) -> i64 {
    (total + i64::from(page_size) - 1) / i64::from(page_size)
}
pub(super) async fn patch_template(
    State(state): State<AppState>,
    Path(key): Path<String>,
    request: Request,
) -> Response {
    let (request, body): (_, PatchTemplate) = match json(request).await {
        Ok(v) => v,
        Err(e) => return e,
    };
    if !matches!(body.key, Patch::Absent) {
        return error(&request, StatusCode::BAD_REQUEST, "immutable_field");
    }
    let expected = match expected(request.headers()) {
        Ok(v) => v,
        Err(c) => return error(&request, StatusCode::BAD_REQUEST, c),
    };
    let database = match database(&state) {
        Ok(v) => v,
        Err(e) => return e,
    };
    let old = match template_by(database, &key).await {
        Ok(v) => v,
        Err(sqlx::Error::RowNotFound) => {
            return error(&request, StatusCode::NOT_FOUND, "not_found");
        }
        Err(e) => return sql_error(&request, &e),
    };
    if old.revision != expected {
        database
            .template_conflict(id(&request), old.id, expected)
            .await;
        return error(&request, StatusCode::CONFLICT, "revision_conflict");
    }
    let name = match body.name.value() {
        Some(Some(v)) if valid_text(&v, 128, false) => v,
        Some(_) => return error(&request, StatusCode::BAD_REQUEST, "validation_failed"),
        None => old.name,
    };
    let description = body.description.value().unwrap_or(old.description);
    let language = match body.language.value() {
        Some(Some(v)) if valid_text(&v, 32, false) => v,
        Some(_) => return error(&request, StatusCode::BAD_REQUEST, "validation_failed"),
        None => old.language,
    };
    let prompt = match body.prompt.value() {
        Some(Some(v)) if valid_text(&v, 64 * 1024, false) => v,
        Some(_) => return error(&request, StatusCode::BAD_REQUEST, "validation_failed"),
        None => old.prompt,
    };
    let enabled = match body.enabled.value() {
        Some(Some(v)) => i64::from(v),
        Some(None) => return error(&request, StatusCode::BAD_REQUEST, "validation_failed"),
        None => old.enabled,
    };
    if description
        .as_ref()
        .is_some_and(|v| !valid_text(v, 2048, true))
    {
        return error(&request, StatusCode::BAD_REQUEST, "validation_failed");
    }
    if let Err(cause) = database
        .update_template(
            old.id,
            expected,
            TemplateChanges {
                name: &name,
                description: description.as_deref(),
                language: &language,
                prompt: &prompt,
                enabled,
            },
            id(&request),
        )
        .await
    {
        return write_error(&request, cause);
    }
    // The Template revision is part of the session's frozen snapshot; drop sessions pinned to it
    // so the next admission observes the new prompt/language/enabled state instead of hot-reloading.
    if let Some(security) = security(&state) {
        security.invalidate_template_speakers(old.id);
    }
    get_template(State(state), Path(key), request).await
}

pub(super) async fn bind_template_provider(
    State(state): State<AppState>,
    Path((key, provider_type)): Path<(String, String)>,
    request: Request,
) -> Response {
    let (request, body): (_, ProviderBinding) = match json(request).await {
        Ok(v) => v,
        Err(e) => return e,
    };
    if !matches!(provider_type.as_str(), "vad" | "asr" | "llm" | "tts") {
        return error(&request, StatusCode::BAD_REQUEST, "validation_failed");
    }
    let expected = match expected(request.headers()) {
        Ok(v) => v,
        Err(c) => return error(&request, StatusCode::BAD_REQUEST, c),
    };
    let database = match database(&state) {
        Ok(v) => v,
        Err(e) => return e,
    };
    let template_id = match database
        .bind_template_provider(
            &key,
            &provider_type,
            &body.provider_key,
            expected,
            id(&request),
        )
        .await
    {
        Ok(value) => value,
        Err(cause) => return write_error(&request, cause),
    };
    if let Some(prewarm) = &state.provider_prewarm {
        prewarm.template(template_id).await;
    }
    match template_by(database, &key).await {
        Ok(v) => Json(v).into_response(),
        Err(e) => sql_error(&request, &e),
    }
}

pub(super) async fn set_default_template(
    State(state): State<AppState>,
    Path((agent_key, template_key)): Path<(String, String)>,
    request: Request,
) -> Response {
    let expected = match expected(request.headers()) {
        Ok(v) => v,
        Err(c) => return error(&request, StatusCode::BAD_REQUEST, c),
    };
    let database = match database(&state) {
        Ok(v) => v,
        Err(e) => return e,
    };
    let template_id = match database
        .set_default_template(&agent_key, &template_key, expected, id(&request))
        .await
    {
        Ok(value) => value,
        Err(cause) => return write_error(&request, cause),
    };
    if let Some(prewarm) = &state.provider_prewarm {
        prewarm.template(template_id).await;
    }
    match get_agent_by(database, &agent_key).await {
        Ok(v) => Json(v).into_response(),
        Err(e) => sql_error(&request, &e),
    }
}

pub(super) async fn assign_template(
    State(state): State<AppState>,
    Path((agent_key, template_key)): Path<(String, String)>,
    request: Request,
) -> Response {
    let expected = match expected(request.headers()) {
        Ok(v) => v,
        Err(c) => return error(&request, StatusCode::BAD_REQUEST, c),
    };
    let database = match database(&state) {
        Ok(v) => v,
        Err(e) => return e,
    };
    match database
        .assign_template(&agent_key, &template_key, expected, id(&request))
        .await
    {
        Ok(value) => value,
        Err(cause) => return write_error(&request, cause),
    };
    match get_agent_by(database, &agent_key).await {
        Ok(v) => Json(v).into_response(),
        Err(e) => sql_error(&request, &e),
    }
}
