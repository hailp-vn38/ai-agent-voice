//! Admin mcp servers resources.
use super::*;
use crate::database::agents::get_agent_by;

use crate::database::mcp_servers::{McpChanges, McpInput, McpServerRow, mcp_by};
use crate::tools::external_mcp::diagnostic::{McpAuth, auth_parts};
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CreateMcpServer {
    #[serde(default)]
    api_key: Option<crate::database::secrets::SecretValue>,
    key: String,
    name: String,
    url: String,
    auth: McpAuth,
    #[serde(default = "default_connect_timeout")]
    connect_timeout_ms: u64,
    #[serde(default = "default_request_timeout")]
    request_timeout_ms: u64,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PatchMcpServer {
    #[serde(default)]
    api_key: Patch<crate::database::secrets::SecretValue>,
    #[serde(default)]
    key: Patch<String>,
    #[serde(default)]
    name: Patch<String>,
    #[serde(default)]
    url: Patch<String>,
    #[serde(default)]
    auth: Patch<McpAuth>,
    #[serde(default)]
    connect_timeout_ms: Patch<u64>,
    #[serde(default)]
    request_timeout_ms: Patch<u64>,
    #[serde(default)]
    enabled: Patch<bool>,
}
#[derive(Deserialize)]
struct McpBinding {
    #[serde(default = "default_true")]
    enabled: bool,
    #[serde(default)]
    required: bool,
}
fn default_connect_timeout() -> u64 {
    5_000
}
fn default_request_timeout() -> u64 {
    30_000
}
fn default_true() -> bool {
    true
}

fn valid_mcp_url(url: &str, state: &AppState) -> bool {
    external_mcp_policy::valid_desired_url(url, &state.config.mcp.external.network)
}
fn mcp_json(row: McpServerRow) -> Value {
    let auth = match row.auth_type.as_str() {
        "bearer" => serde_json::json!({"type":"bearer"}),
        "header" => {
            serde_json::json!({"type":"header","header_name":row.auth_header_name})
        }
        _ => serde_json::json!({"type":"none"}),
    };
    let credential_env = crate::database::secrets::mcp_secret_env(&row.key, &row.auth_type);
    serde_json::json!({"key":row.key,"name":row.name,"transport":"streamable_http","url":row.url,"headers":{},"auth":auth,"credential_env":credential_env,"credential":crate::database::credentials::metadata(row.credential_json.as_deref()),"connect_timeout_ms":row.connect_timeout_ms,"request_timeout_ms":row.request_timeout_ms,"enabled":row.enabled != 0,"revision":row.revision,"created_at":row.created_at,"updated_at":row.updated_at})
}
pub(super) async fn create_mcp_server(State(state): State<AppState>, request: Request) -> Response {
    let (request, body): (_, CreateMcpServer) = match json(request).await {
        Ok(v) => v,
        Err(e) => return e,
    };
    let Some((auth_type, auth_header_name)) = auth_parts(body.auth) else {
        return error(&request, StatusCode::BAD_REQUEST, "validation_failed");
    };
    if !valid_key(&body.key)
        || !valid_text(&body.name, 128, false)
        || !valid_mcp_url(&body.url, &state)
        || !(1..=60_000).contains(&body.connect_timeout_ms)
        || !(1..=120_000).contains(&body.request_timeout_ms)
    {
        return error(&request, StatusCode::BAD_REQUEST, "validation_failed");
    }
    let credential = match body.api_key {
        Some(value) => {
            if auth_type == "none" || !crate::database::credentials::valid_input(&value) {
                return error(&request, StatusCode::BAD_REQUEST, "credential_invalid");
            }
            match state
                .secret_resolver
                .seal(&value, &format!("mcp:{}", body.key))
            {
                Ok(record) => Some(record),
                Err(_) => {
                    return error(
                        &request,
                        StatusCode::SERVICE_UNAVAILABLE,
                        "credential_storage_unavailable",
                    );
                }
            }
        }
        None => None,
    };
    let database = match database(&state) {
        Ok(v) => v,
        Err(e) => return e,
    };
    if let Err(cause) = database
        .create_mcp_server(
            McpInput {
                key: &body.key,
                name: &body.name,
                url: &body.url,
                auth_type: &auth_type,
                auth_header_name: auth_header_name.as_deref(),
                credential: credential.as_deref(),
                connect_timeout_ms: body.connect_timeout_ms as i64,
                request_timeout_ms: body.request_timeout_ms as i64,
            },
            id(&request),
        )
        .await
    {
        return write_error(&request, cause);
    }
    match mcp_by(database, &body.key).await {
        Ok(row) => (StatusCode::CREATED, Json(mcp_json(row))).into_response(),
        Err(e) => sql_error(&request, &e),
    }
}
pub(super) async fn get_mcp_server(
    State(state): State<AppState>,
    Path(key): Path<String>,
    request: Request,
) -> Response {
    let database = match database(&state) {
        Ok(v) => v,
        Err(e) => return e,
    };
    match mcp_by(database, &key).await {
        Ok(row) => Json(mcp_json(row)).into_response(),
        Err(sqlx::Error::RowNotFound) => error(&request, StatusCode::NOT_FOUND, "not_found"),
        Err(e) => sql_error(&request, &e),
    }
}
pub(super) async fn list_mcp_servers(
    State(state): State<AppState>,
    Query(query): Query<PageQuery>,
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
    match database.list_mcp_servers(page, size).await {Ok(rows)=>Json(serde_json::json!({"items":rows.into_iter().map(mcp_json).collect::<Vec<_>>(),"page":page,"page_size":size,"max_page_size":PAGE_MAX})).into_response(),Err(e)=>sql_error(&request,&e)}
}
pub(super) async fn patch_mcp_server(
    State(state): State<AppState>,
    Path(key): Path<String>,
    request: Request,
) -> Response {
    let (request, body): (_, PatchMcpServer) = match json(request).await {
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
    let old = match mcp_by(database, &key).await {
        Ok(v) => v,
        Err(sqlx::Error::RowNotFound) => {
            return error(&request, StatusCode::NOT_FOUND, "not_found");
        }
        Err(e) => return sql_error(&request, &e),
    };
    if old.revision != expected {
        database.mcp_conflict(id(&request), old.id, expected).await;
        return error(&request, StatusCode::CONFLICT, "revision_conflict");
    }
    let name = match body.name.value() {
        Some(Some(v)) if valid_text(&v, 128, false) => v,
        Some(_) => return error(&request, StatusCode::BAD_REQUEST, "validation_failed"),
        None => old.name.clone(),
    };
    let url = match body.url.value() {
        Some(Some(v)) if valid_mcp_url(&v, &state) => v,
        Some(_) => return error(&request, StatusCode::BAD_REQUEST, "validation_failed"),
        None => old.url.clone(),
    };
    let (auth_type, auth_header_name) = match body.auth.value() {
        Some(Some(auth)) => match auth_parts(auth) {
            Some(v) => v,
            None => return error(&request, StatusCode::BAD_REQUEST, "validation_failed"),
        },
        Some(None) => return error(&request, StatusCode::BAD_REQUEST, "validation_failed"),
        None => (old.auth_type.clone(), old.auth_header_name.clone()),
    };
    let connect_timeout = match body.connect_timeout_ms.value() {
        Some(Some(v)) if (1..=60_000).contains(&v) => v as i64,
        Some(_) => return error(&request, StatusCode::BAD_REQUEST, "validation_failed"),
        None => old.connect_timeout_ms,
    };
    let request_timeout = match body.request_timeout_ms.value() {
        Some(Some(v)) if (1..=120_000).contains(&v) => v as i64,
        Some(_) => return error(&request, StatusCode::BAD_REQUEST, "validation_failed"),
        None => old.request_timeout_ms,
    };
    let enabled = match body.enabled.value() {
        Some(Some(v)) => i64::from(v),
        Some(None) => return error(&request, StatusCode::BAD_REQUEST, "validation_failed"),
        None => old.enabled,
    };
    let credential = match body.api_key.value() {
        Some(Some(value))
            if auth_type != "none" && crate::database::credentials::valid_input(&value) =>
        {
            match state.secret_resolver.seal(&value, &format!("mcp:{key}")) {
                Ok(record) => Some(record),
                Err(_) => {
                    return error(
                        &request,
                        StatusCode::SERVICE_UNAVAILABLE,
                        "credential_storage_unavailable",
                    );
                }
            }
        }
        Some(_) => return error(&request, StatusCode::BAD_REQUEST, "credential_invalid"),
        None if auth_type == "none" => None,
        None => old.credential_json.clone(),
    };
    if let Err(cause) = database
        .update_mcp_server(
            &old,
            expected,
            McpChanges {
                name: &name,
                url: &url,
                auth_type: &auth_type,
                auth_header_name: auth_header_name.as_deref(),
                credential: credential.as_deref(),
                connect_timeout,
                request_timeout,
                enabled,
            },
            id(&request),
        )
        .await
    {
        return write_error(&request, cause);
    }
    get_mcp_server(State(state), Path(key), request).await
}
pub(super) async fn list_agent_mcp_bindings(
    State(state): State<AppState>,
    Path(key): Path<String>,
    request: Request,
) -> Response {
    let database = match database(&state) {
        Ok(v) => v,
        Err(e) => return e,
    };
    let agent = match get_agent_by(database, &key).await {
        Ok(v) => v,
        Err(sqlx::Error::RowNotFound) => {
            return error(&request, StatusCode::NOT_FOUND, "not_found");
        }
        Err(e) => return sql_error(&request, &e),
    };
    match database.agent_mcp_bindings(agent.id).await { Ok(rows)=>Json(serde_json::json!({"items":rows.into_iter().map(|(server_key,enabled,required)|serde_json::json!({"server_key":server_key,"enabled":enabled!=0,"required":required!=0})).collect::<Vec<_>>() })).into_response(),Err(e)=>sql_error(&request,&e) }
}
pub(super) async fn put_agent_mcp_binding(
    State(state): State<AppState>,
    Path((key, server_key)): Path<(String, String)>,
    request: Request,
) -> Response {
    let (request, body): (_, McpBinding) = match json(request).await {
        Ok(v) => v,
        Err(e) => return e,
    };
    if body.required {
        return error(&request, StatusCode::BAD_REQUEST, "required_unsupported");
    };
    let expected = match expected(request.headers()) {
        Ok(v) => v,
        Err(c) => return error(&request, StatusCode::BAD_REQUEST, c),
    };
    let database = match database(&state) {
        Ok(v) => v,
        Err(e) => return e,
    };
    if let Err(cause) = database
        .put_agent_mcp_binding(&key, &server_key, body.enabled, expected, id(&request))
        .await
    {
        return write_error(&request, cause);
    }
    match get_agent_by(database, &key).await {
        Ok(v) => Json(v).into_response(),
        Err(e) => sql_error(&request, &e),
    }
}

pub(super) async fn unlink_agent_mcp_binding(
    State(state): State<AppState>,
    Path((key, server_key)): Path<(String, String)>,
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
        .unlink_agent_mcp_binding(&key, &server_key, expected, id(&request))
        .await
    {
        return write_error(&request, cause);
    }
    StatusCode::NO_CONTENT.into_response()
}
