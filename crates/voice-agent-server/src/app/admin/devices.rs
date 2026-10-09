//! Admin devices resources.
use super::*;

use crate::database::devices::{DeviceChanges, DeviceInput, get_device_by};
#[derive(Deserialize)]
struct CreateDevice {
    device_id: String,
    agent_key: String,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    metadata_json: Option<Value>,
    #[serde(default)]
    template_key: Option<String>,
}
#[derive(Deserialize)]
struct PatchDevice {
    #[serde(default)]
    device_id: Patch<String>,
    #[serde(default)]
    agent_key: Patch<String>,
    #[serde(default)]
    name: Patch<String>,
    #[serde(default)]
    description: Patch<String>,
    #[serde(default)]
    metadata_json: Patch<Value>,
    #[serde(default)]
    enabled: Patch<bool>,
    #[serde(default)]
    template_key: Patch<String>,
}
pub(super) async fn create_device(State(state): State<AppState>, request: Request) -> Response {
    let (request, body): (_, CreateDevice) = match json(request).await {
        Ok(v) => v,
        Err(e) => return e,
    };
    if !valid_identity(&body.device_id)
        || body
            .name
            .as_ref()
            .is_some_and(|v| !valid_text(v, 128, true))
        || body
            .description
            .as_ref()
            .is_some_and(|v| !valid_text(v, 2048, true))
        || body
            .metadata_json
            .as_ref()
            .is_some_and(|v| !valid_metadata(v))
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
    let metadata = body.metadata_json.map(|v| v.to_string());
    if let Err(cause) = database
        .create_device(
            DeviceInput {
                device_id: &body.device_id,
                agent_key: &body.agent_key,
                template_key: body.template_key.as_deref(),
                name: body.name.as_deref(),
                description: body.description.as_deref(),
                metadata: metadata.as_deref(),
            },
            id(&request),
        )
        .await
    {
        return write_error(&request, cause);
    }
    get_device_by(database, &body.device_id)
        .await
        .map(|v| (StatusCode::CREATED, Json(v)).into_response())
        .unwrap_or_else(|_| {
            error(
                &request,
                StatusCode::SERVICE_UNAVAILABLE,
                "database_unavailable",
            )
        })
}
pub(super) async fn get_device(
    State(state): State<AppState>,
    Path(device_id): Path<String>,
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
    match get_device_by(database, &device_id).await {
        Ok(v) => Json(v).into_response(),
        Err(sqlx::Error::RowNotFound) => error(&request, StatusCode::NOT_FOUND, "not_found"),
        Err(error_value) => sql_error(&request, &error_value),
    }
}
pub(super) async fn list_devices(
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
        .is_some_and(|v| !matches!(v, "device_id" | "name" | "-device_id" | "-name"))
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
        .list_devices(query.enabled, query.sort.as_deref(), page, page_size)
        .await;
    match rows{Ok(items)=>Json(serde_json::json!({"items":items,"page":page,"page_size":page_size,"max_page_size":PAGE_MAX})).into_response(),Err(error_value)=>sql_error(&request, &error_value)}
}
pub(super) async fn patch_device(
    State(state): State<AppState>,
    Path(device_id): Path<String>,
    request: Request,
) -> Response {
    let (request, body): (_, PatchDevice) = match json(request).await {
        Ok(v) => v,
        Err(e) => return e,
    };
    if !matches!(body.device_id, Patch::Absent) {
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
    let old = match get_device_by(database, &device_id).await {
        Ok(v) => v,
        Err(sqlx::Error::RowNotFound) => {
            return error(&request, StatusCode::NOT_FOUND, "not_found");
        }
        Err(error_value) => return sql_error(&request, &error_value),
    };
    if old.revision != expected {
        {
            database
                .device_conflict(id(&request), old.id, expected)
                .await;
            return error(&request, StatusCode::CONFLICT, "revision_conflict");
        }
    };
    let agent_key = match body.agent_key.value() {
        None => old.agent_key.clone(),
        Some(Some(value)) => value,
        Some(None) => return error(&request, StatusCode::BAD_REQUEST, "immutable_field"),
    };
    let requested_template_key = match body.template_key.value() {
        None => old.template_key.clone(),
        Some(Some(value)) => Some(value),
        Some(None) => None,
    };
    let name = body.name.value().unwrap_or(old.name.clone());
    let description = body.description.value().unwrap_or(old.description.clone());
    let metadata = match body.metadata_json.value() {
        Some(Some(v)) if valid_metadata(&v) => Some(v.to_string()),
        Some(Some(_)) => return error(&request, StatusCode::BAD_REQUEST, "validation_failed"),
        Some(None) => None,
        None => old.metadata_json.clone(),
    };
    if name.as_ref().is_some_and(|v| !valid_text(v, 128, true))
        || description
            .as_ref()
            .is_some_and(|v| !valid_text(v, 2048, true))
    {
        return error(&request, StatusCode::BAD_REQUEST, "validation_failed");
    };
    let enabled = match body.enabled.value() {
        None => old.enabled,
        Some(Some(value)) => i64::from(value),
        Some(None) => return error(&request, StatusCode::BAD_REQUEST, "validation_failed"),
    };
    if let Err(cause) = database
        .update_device(
            old.id,
            expected,
            DeviceChanges {
                agent_key: &agent_key,
                template_key: requested_template_key.as_deref(),
                name: name.as_deref(),
                description: description.as_deref(),
                metadata: metadata.as_deref(),
                enabled,
            },
            id(&request),
        )
        .await
    {
        return write_error(&request, cause);
    }
    get_device_by(database, &device_id)
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
