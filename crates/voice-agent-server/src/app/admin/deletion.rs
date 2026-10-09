//! Conditional hard-delete HTTP endpoints.
use super::*;
use crate::database::deletion::DeleteResource;

pub(super) async fn delete_agent(
    State(state): State<AppState>,
    Path(key): Path<String>,
    request: Request,
) -> Response {
    delete_by_key(&state, &key, request, DeleteResource::Agent).await
}

pub(super) async fn delete_device(
    State(state): State<AppState>,
    Path(device_id): Path<String>,
    request: Request,
) -> Response {
    delete_by_key(&state, &device_id, request, DeleteResource::Device).await
}

pub(super) async fn delete_template(
    State(state): State<AppState>,
    Path(key): Path<String>,
    request: Request,
) -> Response {
    delete_by_key(&state, &key, request, DeleteResource::Template).await
}

pub(super) async fn delete_provider(
    State(state): State<AppState>,
    Path(key): Path<String>,
    request: Request,
) -> Response {
    delete_by_key(&state, &key, request, DeleteResource::Provider).await
}

pub(super) async fn delete_mcp_server(
    State(state): State<AppState>,
    Path(key): Path<String>,
    request: Request,
) -> Response {
    delete_by_key(&state, &key, request, DeleteResource::McpServer).await
}

async fn delete_by_key(
    state: &AppState,
    key: &str,
    request: Request,
    resource: DeleteResource,
) -> Response {
    let expected = match expected(request.headers()) {
        Ok(value) => value,
        Err(code) => return error(&request, StatusCode::BAD_REQUEST, code),
    };
    let database = match database(state) {
        Ok(value) => value,
        Err(response) => return response,
    };
    match database
        .delete_resource(resource, key, expected, id(&request))
        .await
    {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(cause) => write_error(&request, cause),
    }
}
