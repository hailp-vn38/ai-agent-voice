use super::*;
use crate::services::background::plugins::{PluginError, PluginStore};
use axum::extract::Multipart;

pub(super) const MAX_UPLOAD_BODY: usize = 21 * 1024 * 1024;

/// Core is compiled into the binary. Only its activation policy lives in DB.
pub(super) async fn builtin_catalog(State(state): State<AppState>, request: Request) -> Response {
    let db = match database(&state) {
        Ok(db) => db,
        Err(response) => return response,
    };
    let settings = match db.core_plugin_settings().await {
        Ok(rows) => rows,
        Err(error) => return core_policy_error(&request, error),
    };
    match crate::tools::core::bundled_catalog(&settings) {
        Ok(catalog) => Json(catalog).into_response(),
        Err(_) => error(
            &request,
            StatusCode::SERVICE_UNAVAILABLE,
            "database_unavailable",
        ),
    }
}

fn core_policy_error(
    request: &Request,
    cause: crate::database::core_plugins::CorePluginPolicyError,
) -> Response {
    use crate::database::core_plugins::CorePluginPolicyError;
    match cause {
        CorePluginPolicyError::NotFound => error(request, StatusCode::NOT_FOUND, "not_found"),
        CorePluginPolicyError::RevisionConflict => {
            error(request, StatusCode::CONFLICT, "revision_conflict")
        }
        CorePluginPolicyError::Unavailable => error(
            request,
            StatusCode::SERVICE_UNAVAILABLE,
            "database_unavailable",
        ),
        CorePluginPolicyError::Sql(cause) => sql_error(request, &cause),
    }
}

pub(super) async fn builtin_set_enabled(
    State(state): State<AppState>,
    Path(id): Path<String>,
    request: Request,
) -> Response {
    let expected_revision = match expected(request.headers()) {
        Ok(revision) => revision,
        Err(code) => return error(&request, StatusCode::BAD_REQUEST, code),
    };
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Change {
        enabled: bool,
    }
    let (request, change): (_, Change) = match json(request).await {
        Ok(decoded) => decoded,
        Err(response) => return response,
    };
    if !crate::tools::core::known_group(&id) {
        return error(&request, StatusCode::NOT_FOUND, "not_found");
    }
    let db = match database(&state) {
        Ok(db) => db,
        Err(response) => return response,
    };
    let updated = match db
        .set_core_plugin_enabled(&id, expected_revision, change.enabled)
        .await
    {
        Ok(row) => row,
        Err(cause) => return core_policy_error(&request, cause),
    };
    // Response is fully derived from the committed CAS result; a subsequent
    // read failure must not disguise a successful mutation as HTTP 503.
    match crate::tools::core::bundled_item(&updated) {
        Some(item) => Json(item).into_response(),
        None => error(
            &request,
            StatusCode::SERVICE_UNAVAILABLE,
            "database_unavailable",
        ),
    }
}

fn store(state: &AppState) -> Option<&PluginStore> {
    state
        .background
        .as_ref()
        .map(|runtime| &runtime.engine.plugins)
}
fn failure(cause: PluginError) -> Response {
    let status = match cause {
        PluginError::InvalidPackage | PluginError::PermissionRequired => StatusCode::BAD_REQUEST,
        PluginError::RevisionConflict | PluginError::InUse => StatusCode::CONFLICT,
        PluginError::NotFound => StatusCode::NOT_FOUND,
        PluginError::BuildTimeout => StatusCode::REQUEST_TIMEOUT,
        _ => StatusCode::SERVICE_UNAVAILABLE,
    };
    (
        status,
        Json(serde_json::json!({"error":{"code":cause.code()}})),
    )
        .into_response()
}
fn unavailable() -> Response {
    failure(PluginError::SandboxUnavailable)
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct PluginQuery {
    page: Option<u32>,
    page_size: Option<u32>,
    status: Option<String>,
    q: Option<String>,
}
pub(super) async fn list(
    State(state): State<AppState>,
    Query(query): Query<PluginQuery>,
) -> Response {
    let Some(store) = store(&state) else {
        return unavailable();
    };
    let page = query.page.unwrap_or(1);
    let size = query.page_size.unwrap_or(30);
    match store
        .list(page, size, query.status.as_deref(), query.q.as_deref())
        .await
    {
        Ok((items, total)) => {
            Json(serde_json::json!({"items":items,"total":total,"page":page,"page_size":size}))
                .into_response()
        }
        Err(e) => failure(e),
    }
}
pub(super) async fn get(State(state): State<AppState>, Path(id): Path<String>) -> Response {
    let Some(store) = store(&state) else {
        return unavailable();
    };
    match store.get(&id).await {
        Ok(view) => Json(view).into_response(),
        Err(e) => failure(e),
    }
}
pub(super) async fn install(State(state): State<AppState>, mut multipart: Multipart) -> Response {
    let Some(store) = store(&state) else {
        return unavailable();
    };
    let field = match multipart.next_field().await {
        Ok(Some(field)) => field,
        _ => return failure(PluginError::InvalidPackage),
    };
    if field.name() != Some("package")
        || !field
            .file_name()
            .is_some_and(|name| name.to_ascii_lowercase().ends_with(".zip"))
    {
        return failure(PluginError::InvalidPackage);
    }
    let bytes = match field.bytes().await {
        Ok(bytes) if bytes.len() <= 20 * 1024 * 1024 => bytes,
        _ => return failure(PluginError::InvalidPackage),
    };
    if !matches!(multipart.next_field().await, Ok(None)) {
        return failure(PluginError::InvalidPackage);
    }
    match store.install(&bytes).await {
        Ok(view) => (StatusCode::CREATED, Json(view)).into_response(),
        Err(e) => failure(e),
    }
}
pub(super) async fn install_curated(
    State(state): State<AppState>,
    Path(id): Path<String>,
    request: Request,
) -> Response {
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Empty {}
    let (request, _): (_, Empty) = match json(request).await {
        Ok(v) => v,
        Err(e) => return e,
    };
    let Some(store) = store(&state) else {
        return error(
            &request,
            StatusCode::SERVICE_UNAVAILABLE,
            "sandbox_unavailable",
        );
    };
    match store.install_curated(&id).await {
        Ok(view) => (StatusCode::CREATED, Json(view)).into_response(),
        Err(e) => failure(e),
    }
}
#[allow(clippy::result_large_err)] // Same request-ID-preserving HTTP boundary as json().
async fn empty_request(request: Request) -> Result<(Request, i64), Response> {
    let revision =
        expected(request.headers()).map_err(|e| error(&request, StatusCode::BAD_REQUEST, e))?;
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Empty {}
    let (request, _): (_, Empty) = json(request).await?;
    Ok((request, revision))
}
pub(super) async fn test(
    State(state): State<AppState>,
    Path(id): Path<String>,
    request: Request,
) -> Response {
    let (_, revision) = match empty_request(request).await {
        Ok(v) => v,
        Err(e) => return e,
    };
    let Some(store) = store(&state) else {
        return unavailable();
    };
    match store.test(&id, revision).await {
        Ok(()) => Json(serde_json::json!({"ok":true})).into_response(),
        Err(e) => Json(serde_json::json!({"ok":false,"code":e.code()})).into_response(),
    }
}
pub(super) async fn activate(
    State(state): State<AppState>,
    Path(id): Path<String>,
    request: Request,
) -> Response {
    let (_, revision) = match empty_request(request).await {
        Ok(v) => v,
        Err(e) => return e,
    };
    let Some(store) = store(&state) else {
        return unavailable();
    };
    match store.activate(&id, revision).await {
        Ok(view) => Json(view).into_response(),
        Err(e) => failure(e),
    }
}
pub(super) async fn disable(
    State(state): State<AppState>,
    Path(id): Path<String>,
    request: Request,
) -> Response {
    let (_, revision) = match empty_request(request).await {
        Ok(v) => v,
        Err(e) => return e,
    };
    let Some(store) = store(&state) else {
        return unavailable();
    };
    match store.disable(&id, revision).await {
        Ok(view) => Json(view).into_response(),
        Err(e) => failure(e),
    }
}
pub(super) async fn uninstall(
    State(state): State<AppState>,
    Path(id): Path<String>,
    request: Request,
) -> Response {
    let revision = match expected(request.headers()) {
        Ok(v) => v,
        Err(e) => return error(&request, StatusCode::BAD_REQUEST, e),
    };
    let Some(store) = store(&state) else {
        return unavailable();
    };
    match store.uninstall(&id, revision).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => failure(e),
    }
}
