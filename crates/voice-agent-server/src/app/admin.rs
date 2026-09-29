//! Bounded, separately authenticated Admin HTTP surface.  It deliberately exposes only
//! Agent and Device desired configuration in this rollout.
use super::AppState;
use crate::database::secrets::SecretRef;
use crate::database::{external_mcp_policy, provider_config};
use axum::{
    Json, Router,
    body::{Body, to_bytes},
    extract::{Path, Query, Request, State},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    middleware::Next,
    response::{IntoResponse, Response},
    routing::{get, put},
};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::Value;
use sqlx::{Executor, FromRow, Sqlite, SqlitePool};
use std::time::{SystemTime, UNIX_EPOCH};
use uuid::Uuid;

const MAX_BODY: usize = 256 * 1024;
const PAGE_DEFAULT: u32 = 50;
const PAGE_MAX: u32 = 200;

pub(super) fn router(state: AppState) -> Router<AppState> {
    Router::new()
        .route("/agents", get(list_agents).post(create_agent))
        .route("/agents/{key}", get(get_agent).patch(patch_agent))
        .route(
            "/agents/{key}/default-template/{template_key}",
            put(set_default_template),
        )
        .route(
            "/agents/{key}/templates/{template_key}",
            put(assign_template),
        )
        .route("/templates", get(list_templates).post(create_template))
        .route("/templates/{key}", get(get_template).patch(patch_template))
        .route(
            "/templates/{key}/providers/{provider_type}",
            put(bind_template_provider),
        )
        .route("/providers", get(list_providers).post(create_provider))
        .route("/providers/{key}", get(get_provider).patch(patch_provider))
        .route(
            "/mcp-servers",
            get(list_mcp_servers).post(create_mcp_server),
        )
        .route(
            "/mcp-servers/{key}",
            get(get_mcp_server).patch(patch_mcp_server),
        )
        .route("/agents/{key}/mcp-bindings", get(list_agent_mcp_bindings))
        .route(
            "/agents/{key}/mcp-bindings/{server_key}",
            put(put_agent_mcp_binding),
        )
        .route("/devices", get(list_devices).post(create_device))
        .route("/devices/{device_id}", get(get_device).patch(patch_device))
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            authenticate,
        ))
        .layer(axum::middleware::from_fn(transport))
        .layer(axum::middleware::from_fn(request_id))
        .with_state(state)
}

async fn transport(request: Request, next: Next) -> Response {
    if request
        .headers()
        .get(header::CONTENT_ENCODING)
        .is_some_and(|value| value != "identity")
    {
        return error(
            &request,
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            "unsupported_content_encoding",
        );
    }
    let is_mutation = matches!(
        request.method(),
        &http::Method::POST | &http::Method::PATCH | &http::Method::PUT
    );
    if is_mutation {
        if let Some(length) = request
            .headers()
            .get(header::CONTENT_LENGTH)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.parse::<usize>().ok())
        {
            if length > MAX_BODY {
                return error(&request, StatusCode::PAYLOAD_TOO_LARGE, "request_too_large");
            }
        }
    }
    next.run(request).await
}

async fn request_id(mut request: Request, next: Next) -> Response {
    let id = Uuid::new_v4().to_string();
    request.extensions_mut().insert(RequestId(id.clone()));
    let mut response = next.run(request).await;
    response.headers_mut().insert(
        "x-request-id",
        HeaderValue::from_str(&id).expect("uuid header"),
    );
    response
}

async fn authenticate(State(state): State<AppState>, request: Request, next: Next) -> Response {
    let valid = request
        .headers()
        .get_all(header::AUTHORIZATION)
        .iter()
        .count()
        == 1
        && request
            .headers()
            .get(header::AUTHORIZATION)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.strip_prefix("Bearer "))
            .is_some_and(|token| {
                !token.is_empty()
                    && constant_time_eq(token.as_bytes(), state.config.api.admin_token.as_bytes())
            });
    if valid {
        next.run(request).await
    } else {
        error(&request, StatusCode::UNAUTHORIZED, "unauthorized")
    }
}

fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    let mut diff = left.len() ^ right.len();
    for index in 0..left.len().max(right.len()) {
        diff |= usize::from(*left.get(index).unwrap_or(&0) ^ *right.get(index).unwrap_or(&0));
    }
    diff == 0
}

#[derive(Clone)]
struct RequestId(String);
fn id(request: &Request) -> &str {
    request
        .extensions()
        .get::<RequestId>()
        .map(|v| v.0.as_str())
        .unwrap_or("unknown")
}
fn error(request: &Request, status: StatusCode, code: &'static str) -> Response {
    (
        status,
        Json(serde_json::json!({"error":{"code":code,"request_id":id(request)}})),
    )
        .into_response()
}
fn db(state: &AppState) -> Result<&SqlitePool, Response> {
    state.database.as_ref().map(|db| db.pool()).ok_or_else(|| {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({"error":{"code":"database_unavailable"}})),
        )
            .into_response()
    })
}
fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}
fn sql_error_kind(error: &sqlx::Error) -> &'static str {
    match error {
        sqlx::Error::PoolTimedOut => "database_unavailable",
        sqlx::Error::Database(error)
            if matches!(
                error.code().as_deref(),
                Some("5" | "6" | "SQLITE_BUSY" | "SQLITE_LOCKED")
            ) =>
        {
            "database_busy"
        }
        _ => "database_unavailable",
    }
}
fn sql_error(request: &Request, sql_error_value: &sqlx::Error) -> Response {
    error(
        request,
        StatusCode::SERVICE_UNAVAILABLE,
        sql_error_kind(sql_error_value),
    )
}
fn mutation_sql_error(request: &Request, sql_error_value: &sqlx::Error) -> Response {
    if sql_error_kind(sql_error_value) == "database_busy" {
        sql_error(request, sql_error_value)
    } else {
        error(request, StatusCode::CONFLICT, "resource_conflict")
    }
}

async fn json<T: DeserializeOwned>(request: Request) -> Result<(Request, T), Response> {
    if request
        .headers()
        .get(header::CONTENT_ENCODING)
        .is_some_and(|v| v != "identity")
    {
        return Err(error(
            &request,
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            "unsupported_content_encoding",
        ));
    }
    let content_type = request
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    let mut content_parts = content_type.split(';');
    let media_type_ok = content_parts
        .next()
        .is_some_and(|v| v.trim().eq_ignore_ascii_case("application/json"));
    let parameters_ok = content_parts.all(|parameter| {
        let parameter = parameter.trim();
        !parameter.is_empty()
            && parameter
                .split_once('=')
                .is_some_and(|(name, value)| !name.trim().is_empty() && !value.trim().is_empty())
    });
    if !media_type_ok || !parameters_ok {
        return Err(error(
            &request,
            StatusCode::BAD_REQUEST,
            "invalid_content_type",
        ));
    }
    let (parts, body) = request.into_parts();
    let bytes = match to_bytes(body, MAX_BODY).await {
        Ok(bytes) => bytes,
        Err(_) => {
            let request = Request::from_parts(parts, Body::empty());
            return Err(error(
                &request,
                StatusCode::PAYLOAD_TOO_LARGE,
                "request_too_large",
            ));
        }
    };
    let request = Request::from_parts(parts, Body::empty());
    match serde_json::from_slice(&bytes) {
        Ok(payload) => Ok((request, payload)),
        Err(_) => Err(error(&request, StatusCode::BAD_REQUEST, "invalid_json")),
    }
}

#[derive(Serialize, FromRow)]
struct Agent {
    id: i64,
    key: String,
    name: String,
    description: Option<String>,
    enabled: i64,
    revision: i64,
    created_at: i64,
    updated_at: i64,
}
#[derive(Serialize, FromRow)]
struct Device {
    id: i64,
    device_id: String,
    agent_key: String,
    name: Option<String>,
    description: Option<String>,
    enabled: i64,
    metadata_json: Option<String>,
    revision: i64,
    created_at: i64,
    updated_at: i64,
}
#[derive(Deserialize)]
struct CreateAgent {
    key: String,
    name: String,
    #[serde(default)]
    description: Option<String>,
}
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
}
#[derive(Default, Deserialize)]
#[serde(untagged)]
enum Patch<T> {
    Clear(Option<T>),
    #[default]
    Absent,
}
impl<T> Patch<T> {
    fn value(self) -> Option<Option<T>> {
        match self {
            Self::Absent => None,
            Self::Clear(v) => Some(v),
        }
    }
}
#[derive(Deserialize)]
struct PageQuery {
    #[serde(default)]
    page: Option<u32>,
    #[serde(default)]
    enabled: Option<bool>,
    #[serde(default)]
    sort: Option<String>,
    #[serde(default)]
    page_size: Option<u32>,
}

fn page_bounds(query: &PageQuery) -> Result<(u32, u32), &'static str> {
    let page = query.page.unwrap_or(1);
    let page_size = query.page_size.unwrap_or(PAGE_DEFAULT);
    if page == 0 || page > 10_000 || !(1..=PAGE_MAX).contains(&page_size) {
        Err("invalid_query")
    } else {
        Ok((page, page_size))
    }
}

fn valid_key(value: &str) -> bool {
    value.len() <= 64
        && value.as_bytes().first().is_some_and(u8::is_ascii_lowercase)
        && value
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
}
fn valid_text(value: &str, max: usize, allow_empty: bool) -> bool {
    value.len() <= max && (allow_empty || !value.trim().is_empty())
}
fn valid_identity(value: &str) -> bool {
    !value.is_empty() && value.len() <= 128 && value.bytes().all(|b| b > 0x1f && b != 0x7f)
}
fn valid_metadata(value: &Value) -> bool {
    serde_json::to_vec(value).is_ok_and(|v| v.len() <= 16 * 1024) && json_shape(value, 0, &mut 0)
}
fn json_shape(value: &Value, depth: u8, nodes: &mut u16) -> bool {
    if depth > 8 {
        return false;
    };
    match value {
        Value::Array(items) => items.iter().all(|item| {
            *nodes += 1;
            *nodes <= 256 && json_shape(item, depth + 1, nodes)
        }),
        Value::Object(items) => items.values().all(|item| {
            *nodes += 1;
            *nodes <= 256 && json_shape(item, depth + 1, nodes)
        }),
        _ => true,
    }
}
fn expected(headers: &HeaderMap) -> Result<i64, &'static str> {
    let value = headers
        .get(header::IF_MATCH)
        .and_then(|v| v.to_str().ok())
        .ok_or("invalid_if_match")?;
    value
        .strip_prefix('"')
        .and_then(|v| v.strip_suffix('"'))
        .and_then(|v| v.parse().ok())
        .filter(|v: &i64| *v > 0)
        .ok_or("invalid_if_match")
}
async fn audit<'e, E>(
    executor: E,
    request_id: &str,
    resource: &str,
    resource_id: i64,
    action: &str,
    prior: Option<i64>,
    new: Option<i64>,
    outcome: &str,
    error_kind: Option<&str>,
) -> Result<(), sqlx::Error>
where
    E: Executor<'e, Database = Sqlite>,
{
    sqlx::query("INSERT INTO admin_audit_events (created_at,request_id,resource_type,resource_id,action,prior_revision,new_revision,outcome,error_kind,affected_rows) VALUES (?,?,?,?,?,?,?,?,?,?)").bind(now()).bind(request_id).bind(resource).bind(resource_id).bind(action).bind(prior).bind(new).bind(outcome).bind(error_kind).bind(1i64).execute(executor).await.map(|_|())
}

async fn create_agent(State(state): State<AppState>, request: Request) -> Response {
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
    let pool = match db(&state) {
        Ok(v) => v,
        Err(_) => {
            return error(
                &request,
                StatusCode::SERVICE_UNAVAILABLE,
                "database_unavailable",
            );
        }
    };
    let mut tx = match pool.begin().await {
        Ok(v) => v,
        Err(_) => {
            return error(
                &request,
                StatusCode::SERVICE_UNAVAILABLE,
                "database_unavailable",
            );
        }
    };
    let time = now();
    let result = sqlx::query(
        "INSERT INTO agents (key,name,description,created_at,updated_at) VALUES (?,?,?,?,?)",
    )
    .bind(&body.key)
    .bind(&body.name)
    .bind(&body.description)
    .bind(time)
    .bind(time)
    .execute(&mut *tx)
    .await;
    let resource_id = match result {
        Ok(v) => v.last_insert_rowid(),
        Err(error_value) => return mutation_sql_error(&request, &error_value),
    };
    if audit(
        &mut *tx,
        id(&request),
        "agent",
        resource_id,
        "create",
        None,
        Some(1),
        "success",
        None,
    )
    .await
    .is_err()
        || tx.commit().await.is_err()
    {
        return error(
            &request,
            StatusCode::SERVICE_UNAVAILABLE,
            "database_unavailable",
        );
    };
    get_agent_by(pool, &body.key)
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
async fn get_agent(
    State(state): State<AppState>,
    Path(key): Path<String>,
    request: Request,
) -> Response {
    let pool = match db(&state) {
        Ok(v) => v,
        Err(_) => {
            return error(
                &request,
                StatusCode::SERVICE_UNAVAILABLE,
                "database_unavailable",
            );
        }
    };
    match get_agent_by(pool, &key).await {
        Ok(v) => Json(v).into_response(),
        Err(sqlx::Error::RowNotFound) => error(&request, StatusCode::NOT_FOUND, "not_found"),
        Err(error_value) => sql_error(&request, &error_value),
    }
}
async fn get_agent_by(pool: &SqlitePool, key: &str) -> Result<Agent, sqlx::Error> {
    sqlx::query_as("SELECT id,key,name,description,enabled,revision,created_at,updated_at FROM agents WHERE key=?").bind(key).fetch_one(pool).await
}
async fn list_agents(
    State(state): State<AppState>,
    Query(query): Query<PageQuery>,
    request: Request,
) -> Response {
    let (page, page_size) = match page_bounds(&query) {
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
    let pool = match db(&state) {
        Ok(v) => v,
        Err(_) => {
            return error(
                &request,
                StatusCode::SERVICE_UNAVAILABLE,
                "database_unavailable",
            );
        }
    };
    let enabled = query.enabled.map(i64::from);
    let order = match query.sort.as_deref().unwrap_or("key") {
        "name" => "name ASC",
        "-name" => "name DESC",
        "-key" => "key DESC",
        _ => "key ASC",
    };
    let sql = format!(
        "SELECT id,key,name,description,enabled,revision,created_at,updated_at FROM agents WHERE (? IS NULL OR enabled=?) ORDER BY {order} LIMIT ? OFFSET ?"
    );
    let rows = sqlx::query_as::<_, Agent>(&sql)
        .bind(enabled)
        .bind(enabled)
        .bind(i64::from(page_size))
        .bind(i64::from((page - 1) * page_size))
        .fetch_all(pool)
        .await;
    match rows {Ok(items)=>Json(serde_json::json!({"items":items,"page":page,"page_size":page_size,"max_page_size":PAGE_MAX})).into_response(),Err(error_value)=>sql_error(&request, &error_value)}
}
#[axum::debug_handler]
async fn patch_agent(
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
    let pool = match db(&state) {
        Ok(v) => v,
        Err(_) => {
            return error(
                &request,
                StatusCode::SERVICE_UNAVAILABLE,
                "database_unavailable",
            );
        }
    };
    let old = match get_agent_by(pool, &key).await {
        Ok(v) => v,
        Err(sqlx::Error::RowNotFound) => {
            return error(&request, StatusCode::NOT_FOUND, "not_found");
        }
        Err(error_value) => return sql_error(&request, &error_value),
    };
    if old.revision != expected {
        {
            audit_conflict(pool, id(&request).to_owned(), "agent", old.id, expected).await;
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
    let mut tx = match pool.begin().await {
        Ok(v) => v,
        Err(_) => {
            return error(
                &request,
                StatusCode::SERVICE_UNAVAILABLE,
                "database_unavailable",
            );
        }
    };
    let update = sqlx::query("UPDATE agents SET name=?,description=?,enabled=?,revision=revision+1,updated_at=? WHERE id=? AND revision=?").bind(name).bind(description).bind(enabled).bind(now()).bind(old.id).bind(expected).execute(&mut *tx).await;
    let update = match update {
        Ok(result) if result.rows_affected() == 1 => Ok(()),
        Ok(_) => Err(()),
        Err(_) => Err(()),
    };
    if update.is_err() {
        let _ = tx.rollback().await;
        audit_conflict(pool, id(&request).to_owned(), "agent", old.id, expected).await;
        return error(&request, StatusCode::CONFLICT, "revision_conflict");
    }
    if audit(
        &mut *tx,
        id(&request),
        "agent",
        old.id,
        "update",
        Some(expected),
        Some(expected + 1),
        "success",
        None,
    )
    .await
    .is_err()
        || tx.commit().await.is_err()
    {
        return error(
            &request,
            StatusCode::SERVICE_UNAVAILABLE,
            "database_unavailable",
        );
    };
    get_agent_by(pool, &key)
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
#[allow(dead_code)]
async fn patch_agent_enabled(
    state: &AppState,
    key: &str,
    request: &Request,
    expected: i64,
    enabled: Option<Option<bool>>,
) -> Response {
    let Some(enabled) = enabled.flatten() else {
        return get_agent(
            State(state.clone()),
            Path(key.to_owned()),
            Request::new(Body::empty()),
        )
        .await;
    };
    let pool = match db(state) {
        Ok(v) => v,
        Err(_) => {
            return error(
                request,
                StatusCode::SERVICE_UNAVAILABLE,
                "database_unavailable",
            );
        }
    };
    let old = match get_agent_by(pool, key).await {
        Ok(v) => v,
        Err(_) => return error(request, StatusCode::NOT_FOUND, "not_found"),
    };
    if old.revision != expected {
        {
            audit_conflict(pool, id(request).to_owned(), "agent", old.id, expected).await;
            return error(request, StatusCode::CONFLICT, "revision_conflict");
        }
    };
    let mut tx = match pool.begin().await {
        Ok(v) => v,
        Err(_) => {
            return error(
                request,
                StatusCode::SERVICE_UNAVAILABLE,
                "database_unavailable",
            );
        }
    };
    if sqlx::query(
        "UPDATE agents SET enabled=?,revision=revision+1,updated_at=? WHERE id=? AND revision=?",
    )
    .bind(i64::from(enabled))
    .bind(now())
    .bind(old.id)
    .bind(expected)
    .execute(&mut *tx)
    .await
    .is_err()
        || audit(
            &mut *tx,
            id(request),
            "agent",
            old.id,
            "update",
            Some(expected),
            Some(expected + 1),
            "success",
            None,
        )
        .await
        .is_err()
        || tx.commit().await.is_err()
    {
        return error(
            request,
            StatusCode::SERVICE_UNAVAILABLE,
            "database_unavailable",
        );
    };
    get_agent_by(pool, key)
        .await
        .map(|v| Json(v).into_response())
        .unwrap_or_else(|_| {
            error(
                request,
                StatusCode::SERVICE_UNAVAILABLE,
                "database_unavailable",
            )
        })
}
async fn audit_conflict(
    pool: &SqlitePool,
    request_id: String,
    resource: &str,
    resource_id: i64,
    expected: i64,
) {
    let _ = audit(
        pool,
        &request_id,
        resource,
        resource_id,
        "update",
        Some(expected),
        None,
        "conflict",
        Some("revision_conflict"),
    )
    .await;
}

async fn create_device(State(state): State<AppState>, request: Request) -> Response {
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
    let pool = match db(&state) {
        Ok(v) => v,
        Err(_) => {
            return error(
                &request,
                StatusCode::SERVICE_UNAVAILABLE,
                "database_unavailable",
            );
        }
    };
    let agent = match get_agent_by(pool, &body.agent_key).await {
        Ok(v) if v.enabled == 1 => v,
        _ => return error(&request, StatusCode::BAD_REQUEST, "invalid_agent"),
    };
    let metadata = body.metadata_json.map(|v| v.to_string());
    let mut tx = match pool.begin().await {
        Ok(v) => v,
        Err(_) => {
            return error(
                &request,
                StatusCode::SERVICE_UNAVAILABLE,
                "database_unavailable",
            );
        }
    };
    let time = now();
    let result=sqlx::query("INSERT INTO devices (device_id,agent_id,name,description,metadata_json,created_at,updated_at) VALUES (?,?,?,?,?,?,?)").bind(&body.device_id).bind(agent.id).bind(&body.name).bind(&body.description).bind(metadata).bind(time).bind(time).execute(&mut *tx).await;
    let device_id = match result {
        Ok(v) => v.last_insert_rowid(),
        Err(error_value) => return mutation_sql_error(&request, &error_value),
    };
    if audit(
        &mut *tx,
        id(&request),
        "device",
        device_id,
        "create",
        None,
        Some(1),
        "success",
        None,
    )
    .await
    .is_err()
        || tx.commit().await.is_err()
    {
        return error(
            &request,
            StatusCode::SERVICE_UNAVAILABLE,
            "database_unavailable",
        );
    };
    get_device_by(pool, &body.device_id)
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
async fn get_device(
    State(state): State<AppState>,
    Path(device_id): Path<String>,
    request: Request,
) -> Response {
    let pool = match db(&state) {
        Ok(v) => v,
        Err(_) => {
            return error(
                &request,
                StatusCode::SERVICE_UNAVAILABLE,
                "database_unavailable",
            );
        }
    };
    match get_device_by(pool, &device_id).await {
        Ok(v) => Json(v).into_response(),
        Err(sqlx::Error::RowNotFound) => error(&request, StatusCode::NOT_FOUND, "not_found"),
        Err(error_value) => sql_error(&request, &error_value),
    }
}
async fn get_device_by(pool: &SqlitePool, device_id: &str) -> Result<Device, sqlx::Error> {
    sqlx::query_as("SELECT d.id,d.device_id,a.key AS agent_key,d.name,d.description,d.enabled,d.metadata_json,d.revision,d.created_at,d.updated_at FROM devices d JOIN agents a ON a.id=d.agent_id WHERE d.device_id=?").bind(device_id).fetch_one(pool).await
}
async fn list_devices(
    State(state): State<AppState>,
    Query(query): Query<PageQuery>,
    request: Request,
) -> Response {
    let (page, page_size) = match page_bounds(&query) {
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
    let pool = match db(&state) {
        Ok(v) => v,
        Err(_) => {
            return error(
                &request,
                StatusCode::SERVICE_UNAVAILABLE,
                "database_unavailable",
            );
        }
    };
    let enabled = query.enabled.map(i64::from);
    let order = match query.sort.as_deref().unwrap_or("device_id") {
        "name" => "d.name ASC",
        "-name" => "d.name DESC",
        "-device_id" => "d.device_id DESC",
        _ => "d.device_id ASC",
    };
    let sql = format!(
        "SELECT d.id,d.device_id,a.key AS agent_key,d.name,d.description,d.enabled,d.metadata_json,d.revision,d.created_at,d.updated_at FROM devices d JOIN agents a ON a.id=d.agent_id WHERE (? IS NULL OR d.enabled=?) ORDER BY {order} LIMIT ? OFFSET ?"
    );
    let rows = sqlx::query_as::<_, Device>(&sql)
        .bind(enabled)
        .bind(enabled)
        .bind(i64::from(page_size))
        .bind(i64::from((page - 1) * page_size))
        .fetch_all(pool)
        .await;
    match rows{Ok(items)=>Json(serde_json::json!({"items":items,"page":page,"page_size":page_size,"max_page_size":PAGE_MAX})).into_response(),Err(error_value)=>sql_error(&request, &error_value)}
}
async fn patch_device(
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
    let pool = match db(&state) {
        Ok(v) => v,
        Err(_) => {
            return error(
                &request,
                StatusCode::SERVICE_UNAVAILABLE,
                "database_unavailable",
            );
        }
    };
    let old = match get_device_by(pool, &device_id).await {
        Ok(v) => v,
        Err(sqlx::Error::RowNotFound) => {
            return error(&request, StatusCode::NOT_FOUND, "not_found");
        }
        Err(error_value) => return sql_error(&request, &error_value),
    };
    if old.revision != expected {
        {
            audit_conflict(pool, id(&request).to_owned(), "device", old.id, expected).await;
            return error(&request, StatusCode::CONFLICT, "revision_conflict");
        }
    };
    let agent_key = match body.agent_key.value() {
        None => old.agent_key.clone(),
        Some(Some(value)) => value,
        Some(None) => return error(&request, StatusCode::BAD_REQUEST, "immutable_field"),
    };
    let agent = match get_agent_by(pool, &agent_key).await {
        Ok(v) if v.enabled == 1 => v,
        _ => return error(&request, StatusCode::BAD_REQUEST, "invalid_agent"),
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
    let mut tx = match pool.begin().await {
        Ok(v) => v,
        Err(_) => {
            return error(
                &request,
                StatusCode::SERVICE_UNAVAILABLE,
                "database_unavailable",
            );
        }
    };
    let update = sqlx::query("UPDATE devices SET agent_id=?,name=?,description=?,metadata_json=?,enabled=?,revision=revision+1,updated_at=? WHERE id=? AND revision=?").bind(agent.id).bind(name).bind(description).bind(metadata).bind(enabled).bind(now()).bind(old.id).bind(expected).execute(&mut *tx).await;
    let update = match update {
        Ok(result) if result.rows_affected() == 1 => Ok(()),
        Ok(_) => Err(()),
        Err(_) => Err(()),
    };
    if update.is_err() {
        let _ = tx.rollback().await;
        audit_conflict(pool, id(&request).to_owned(), "device", old.id, expected).await;
        return error(&request, StatusCode::CONFLICT, "revision_conflict");
    }
    if audit(
        &mut *tx,
        id(&request),
        "device",
        old.id,
        "update",
        Some(expected),
        Some(expected + 1),
        "success",
        None,
    )
    .await
    .is_err()
        || tx.commit().await.is_err()
    {
        return error(
            &request,
            StatusCode::SERVICE_UNAVAILABLE,
            "database_unavailable",
        );
    };
    get_device_by(pool, &device_id)
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

#[derive(Serialize, FromRow)]
struct Template {
    id: i64,
    key: String,
    name: String,
    description: Option<String>,
    language: String,
    prompt: String,
    enabled: i64,
    revision: i64,
    created_at: i64,
    updated_at: i64,
}
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

async fn template_by(pool: &SqlitePool, key: &str) -> Result<Template, sqlx::Error> {
    sqlx::query_as("SELECT id,key,name,description,language,prompt,enabled,revision,created_at,updated_at FROM agent_templates WHERE key=?").bind(key).fetch_one(pool).await
}
async fn create_template(State(state): State<AppState>, request: Request) -> Response {
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
    let pool = match db(&state) {
        Ok(v) => v,
        Err(e) => return e,
    };
    let mut tx = match pool.begin().await {
        Ok(v) => v,
        Err(_) => {
            return error(
                &request,
                StatusCode::SERVICE_UNAVAILABLE,
                "database_unavailable",
            );
        }
    };
    let time = now();
    let result = sqlx::query("INSERT INTO agent_templates (key,name,description,language,prompt,created_at,updated_at) VALUES (?,?,?,?,?,?,?)").bind(&body.key).bind(&body.name).bind(&body.description).bind(&body.language).bind(&body.prompt).bind(time).bind(time).execute(&mut *tx).await;
    let resource_id = match result {
        Ok(v) => v.last_insert_rowid(),
        Err(e) => return mutation_sql_error(&request, &e),
    };
    if audit(
        &mut *tx,
        id(&request),
        "template",
        resource_id,
        "create",
        None,
        Some(1),
        "success",
        None,
    )
    .await
    .is_err()
        || tx.commit().await.is_err()
    {
        return error(
            &request,
            StatusCode::SERVICE_UNAVAILABLE,
            "database_unavailable",
        );
    }
    match template_by(pool, &body.key).await {
        Ok(v) => (StatusCode::CREATED, Json(v)).into_response(),
        Err(e) => sql_error(&request, &e),
    }
}
async fn get_template(
    State(state): State<AppState>,
    Path(key): Path<String>,
    request: Request,
) -> Response {
    let pool = match db(&state) {
        Ok(v) => v,
        Err(e) => return e,
    };
    match template_by(pool, &key).await {
        Ok(v) => Json(v).into_response(),
        Err(sqlx::Error::RowNotFound) => error(&request, StatusCode::NOT_FOUND, "not_found"),
        Err(e) => sql_error(&request, &e),
    }
}
async fn list_templates(
    State(state): State<AppState>,
    Query(query): Query<PageQuery>,
    request: Request,
) -> Response {
    let (page, size) = match page_bounds(&query) {
        Ok(v) => v,
        Err(c) => return error(&request, StatusCode::BAD_REQUEST, c),
    };
    let pool = match db(&state) {
        Ok(v) => v,
        Err(e) => return e,
    };
    match sqlx::query_as::<_, Template>("SELECT id,key,name,description,language,prompt,enabled,revision,created_at,updated_at FROM agent_templates ORDER BY key LIMIT ? OFFSET ?").bind(i64::from(size)).bind(i64::from((page - 1) * size)).fetch_all(pool).await { Ok(items) => Json(serde_json::json!({"items":items,"page":page,"page_size":size,"max_page_size":PAGE_MAX})).into_response(), Err(e) => sql_error(&request, &e) }
}
async fn patch_template(
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
    let pool = match db(&state) {
        Ok(v) => v,
        Err(e) => return e,
    };
    let old = match template_by(pool, &key).await {
        Ok(v) => v,
        Err(sqlx::Error::RowNotFound) => {
            return error(&request, StatusCode::NOT_FOUND, "not_found");
        }
        Err(e) => return sql_error(&request, &e),
    };
    if old.revision != expected {
        audit_conflict(pool, id(&request).into(), "template", old.id, expected).await;
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
    let mut tx = match pool.begin().await {
        Ok(v) => v,
        Err(_) => {
            return error(
                &request,
                StatusCode::SERVICE_UNAVAILABLE,
                "database_unavailable",
            );
        }
    };
    let updated = sqlx::query("UPDATE agent_templates SET name=?,description=?,language=?,prompt=?,enabled=?,revision=revision+1,updated_at=? WHERE id=? AND revision=?").bind(name).bind(description).bind(language).bind(prompt).bind(enabled).bind(now()).bind(old.id).bind(expected).execute(&mut *tx).await.map(|v| v.rows_affected()==1).unwrap_or(false);
    if !updated {
        let _ = tx.rollback().await;
        return error(&request, StatusCode::CONFLICT, "revision_conflict");
    }
    if audit(
        &mut *tx,
        id(&request),
        "template",
        old.id,
        "update",
        Some(expected),
        Some(expected + 1),
        "success",
        None,
    )
    .await
    .is_err()
        || tx.commit().await.is_err()
    {
        return error(
            &request,
            StatusCode::SERVICE_UNAVAILABLE,
            "database_unavailable",
        );
    }
    get_template(State(state), Path(key), request).await
}

async fn bind_template_provider(
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
    let pool = match db(&state) {
        Ok(v) => v,
        Err(e) => return e,
    };
    let template = match template_by(pool, &key).await {
        Ok(v) => v,
        Err(sqlx::Error::RowNotFound) => {
            return error(&request, StatusCode::NOT_FOUND, "not_found");
        }
        Err(e) => return sql_error(&request, &e),
    };
    if template.revision != expected {
        return error(&request, StatusCode::CONFLICT, "revision_conflict");
    };
    let provider: Result<(i64, String, i64), _> =
        sqlx::query_as("SELECT id,type,enabled FROM providers WHERE key=?")
            .bind(&body.provider_key)
            .fetch_one(pool)
            .await;
    let (provider_id, provider_kind, enabled) = match provider {
        Ok(v) => v,
        Err(_) => return error(&request, StatusCode::BAD_REQUEST, "invalid_provider"),
    };
    if provider_kind != provider_type || enabled != 1 {
        return error(&request, StatusCode::BAD_REQUEST, "invalid_provider");
    };
    let mut tx = match pool.begin().await {
        Ok(v) => v,
        Err(_) => {
            return error(
                &request,
                StatusCode::SERVICE_UNAVAILABLE,
                "database_unavailable",
            );
        }
    };
    if sqlx::query("INSERT INTO template_provider_bindings (template_id,provider_type,provider_id,created_at,updated_at) VALUES (?,?,?,?,?) ON CONFLICT(template_id,provider_type) DO UPDATE SET provider_id=excluded.provider_id,updated_at=excluded.updated_at").bind(template.id).bind(&provider_type).bind(provider_id).bind(now()).bind(now()).execute(&mut *tx).await.is_err() || sqlx::query("UPDATE agent_templates SET revision=revision+1,updated_at=? WHERE id=? AND revision=?").bind(now()).bind(template.id).bind(expected).execute(&mut *tx).await.map(|v|v.rows_affected()!=1).unwrap_or(true) || audit(&mut *tx,id(&request),"template",template.id,"bind_provider",Some(expected),Some(expected+1),"success",None).await.is_err() || tx.commit().await.is_err(){return error(&request,StatusCode::SERVICE_UNAVAILABLE,"database_unavailable")};
    match template_by(pool, &key).await {
        Ok(v) => Json(v).into_response(),
        Err(e) => sql_error(&request, &e),
    }
}
async fn set_default_template(
    State(state): State<AppState>,
    Path((agent_key, template_key)): Path<(String, String)>,
    request: Request,
) -> Response {
    let expected = match expected(request.headers()) {
        Ok(v) => v,
        Err(c) => return error(&request, StatusCode::BAD_REQUEST, c),
    };
    let pool = match db(&state) {
        Ok(v) => v,
        Err(e) => return e,
    };
    let agent = match get_agent_by(pool, &agent_key).await {
        Ok(v) => v,
        Err(sqlx::Error::RowNotFound) => {
            return error(&request, StatusCode::NOT_FOUND, "not_found");
        }
        Err(e) => return sql_error(&request, &e),
    };
    if agent.revision != expected {
        return error(&request, StatusCode::CONFLICT, "revision_conflict");
    };
    let template = match template_by(pool, &template_key).await {
        Ok(v) if v.enabled == 1 => v,
        _ => return error(&request, StatusCode::BAD_REQUEST, "invalid_template"),
    };
    let complete:i64=sqlx::query_scalar("SELECT COUNT(*) FROM template_provider_bindings b JOIN providers p ON p.id=b.provider_id WHERE b.template_id=? AND p.enabled=1 AND b.provider_type=p.type").bind(template.id).fetch_one(pool).await.unwrap_or(0);
    if complete != 4 {
        return error(&request, StatusCode::BAD_REQUEST, "template_incomplete");
    };
    let mut tx = match pool.begin().await {
        Ok(v) => v,
        Err(_) => {
            return error(
                &request,
                StatusCode::SERVICE_UNAVAILABLE,
                "database_unavailable",
            );
        }
    };
    let time = now();
    if sqlx::query("UPDATE agent_template_assignments SET is_default=0 WHERE agent_id=? AND enabled=1").bind(agent.id).execute(&mut *tx).await.is_err()||sqlx::query("INSERT INTO agent_template_assignments(agent_id,template_id,is_default,enabled,created_at) VALUES (?,?,1,1,?) ON CONFLICT(agent_id,template_id) DO UPDATE SET is_default=1,enabled=1").bind(agent.id).bind(template.id).bind(time).execute(&mut *tx).await.is_err()||sqlx::query("UPDATE agents SET revision=revision+1,updated_at=? WHERE id=? AND revision=?").bind(time).bind(agent.id).bind(expected).execute(&mut *tx).await.map(|v|v.rows_affected()!=1).unwrap_or(true)||audit(&mut *tx,id(&request),"agent",agent.id,"set_default_template",Some(expected),Some(expected+1),"success",None).await.is_err()||tx.commit().await.is_err(){return error(&request,StatusCode::SERVICE_UNAVAILABLE,"database_unavailable")};
    match get_agent_by(pool, &agent_key).await {
        Ok(v) => Json(v).into_response(),
        Err(e) => sql_error(&request, &e),
    }
}

async fn assign_template(
    State(state): State<AppState>,
    Path((agent_key, template_key)): Path<(String, String)>,
    request: Request,
) -> Response {
    let expected = match expected(request.headers()) {
        Ok(v) => v,
        Err(c) => return error(&request, StatusCode::BAD_REQUEST, c),
    };
    let pool = match db(&state) {
        Ok(v) => v,
        Err(e) => return e,
    };
    let agent = match get_agent_by(pool, &agent_key).await {
        Ok(v) => v,
        Err(sqlx::Error::RowNotFound) => {
            return error(&request, StatusCode::NOT_FOUND, "not_found");
        }
        Err(e) => return sql_error(&request, &e),
    };
    if agent.revision != expected {
        audit_conflict(pool, id(&request).into(), "agent", agent.id, expected).await;
        return error(&request, StatusCode::CONFLICT, "revision_conflict");
    }
    let template = match template_by(pool, &template_key).await {
        Ok(v) if v.enabled == 1 => v,
        _ => return error(&request, StatusCode::BAD_REQUEST, "invalid_template"),
    };
    let mut tx = match pool.begin().await {
        Ok(v) => v,
        Err(_) => {
            return error(
                &request,
                StatusCode::SERVICE_UNAVAILABLE,
                "database_unavailable",
            );
        }
    };
    let mutated = sqlx::query("INSERT INTO agent_template_assignments(agent_id,template_id,is_default,enabled,created_at) VALUES (?,?,0,1,?) ON CONFLICT(agent_id,template_id) DO UPDATE SET enabled=1")
        .bind(agent.id).bind(template.id).bind(now()).execute(&mut *tx).await.is_ok()
        && sqlx::query("UPDATE agents SET revision=revision+1,updated_at=? WHERE id=? AND revision=?").bind(now()).bind(agent.id).bind(expected).execute(&mut *tx).await.map(|v| v.rows_affected() == 1).unwrap_or(false);
    if !mutated {
        let _ = tx.rollback().await;
        audit_conflict(pool, id(&request).into(), "agent", agent.id, expected).await;
        return error(&request, StatusCode::CONFLICT, "revision_conflict");
    }
    if audit(
        &mut *tx,
        id(&request),
        "agent",
        agent.id,
        "assign_template",
        Some(expected),
        Some(expected + 1),
        "success",
        None,
    )
    .await
    .is_err()
        || tx.commit().await.is_err()
    {
        return error(
            &request,
            StatusCode::SERVICE_UNAVAILABLE,
            "database_unavailable",
        );
    }
    match get_agent_by(pool, &agent_key).await {
        Ok(v) => Json(v).into_response(),
        Err(e) => sql_error(&request, &e),
    }
}

#[derive(Serialize, FromRow)]
struct Provider {
    id: i64,
    key: String,
    name: String,
    #[serde(rename = "type")]
    kind: String,
    adapter: String,
    config_json: String,
    enabled: i64,
    revision: i64,
    created_at: i64,
    updated_at: i64,
    has_secret_ref: bool,
}
#[derive(Deserialize)]
struct CreateProvider {
    key: String,
    name: String,
    #[serde(rename = "type")]
    kind: String,
    adapter: String,
    config_json: Value,
    #[serde(default)]
    secret_ref: Option<String>,
}
#[derive(Deserialize)]
struct PatchProvider {
    #[serde(default)]
    key: Patch<String>,
    #[serde(default)]
    name: Patch<String>,
    #[serde(default)]
    adapter: Patch<String>,
    #[serde(default)]
    config_json: Patch<Value>,
    #[serde(default)]
    secret_ref: Patch<String>,
    #[serde(default)]
    enabled: Patch<bool>,
}
fn valid_secret_ref(value: &str) -> bool {
    SecretRef::parse(value.into()).is_ok()
}
fn adapter_matches_kind(kind: &str, adapter: &str) -> bool {
    matches!(
        (kind, adapter),
        ("vad", "silero_onnx")
            | ("asr", "zipformer_sherpa" | "gipformer_sherpa_offline")
            | ("llm", "openai")
            | ("tts", "zerotts_onnx")
    )
}
async fn provider_by(pool: &SqlitePool, key: &str) -> Result<Provider, sqlx::Error> {
    sqlx::query_as("SELECT id,key,name,type AS kind,adapter,config_json,enabled,revision,created_at,updated_at,secret_ref IS NOT NULL AS has_secret_ref FROM providers WHERE key=?").bind(key).fetch_one(pool).await
}
async fn create_provider(State(state): State<AppState>, request: Request) -> Response {
    let (request, body): (_, CreateProvider) = match json(request).await {
        Ok(v) => v,
        Err(e) => return e,
    };
    if !valid_key(&body.key)
        || !valid_text(&body.name, 128, false)
        || !matches!(body.kind.as_str(), "vad" | "asr" | "llm" | "tts")
        || !adapter_matches_kind(&body.kind, &body.adapter)
        || body
            .secret_ref
            .as_ref()
            .is_some_and(|v| !valid_secret_ref(v))
    {
        return error(&request, StatusCode::BAD_REQUEST, "validation_failed");
    };
    let config = match provider_config::validate(&body.adapter, &body.config_json) {
        Ok(v) => v,
        Err(_) => return error(&request, StatusCode::BAD_REQUEST, "provider_config_invalid"),
    };
    let pool = match db(&state) {
        Ok(v) => v,
        Err(e) => return e,
    };
    let mut tx = match pool.begin().await {
        Ok(v) => v,
        Err(_) => {
            return error(
                &request,
                StatusCode::SERVICE_UNAVAILABLE,
                "database_unavailable",
            );
        }
    };
    let r=sqlx::query("INSERT INTO providers(key,name,type,adapter,config_json,secret_ref,created_at,updated_at) VALUES(?,?,?,?,?,?,?,?)").bind(&body.key).bind(&body.name).bind(&body.kind).bind(&body.adapter).bind(config).bind(&body.secret_ref).bind(now()).bind(now()).execute(&mut *tx).await;
    let provider_id = match r {
        Ok(v) => v.last_insert_rowid(),
        Err(e) => return mutation_sql_error(&request, &e),
    };
    if audit(
        &mut *tx,
        id(&request),
        "provider",
        provider_id,
        "create",
        None,
        Some(1),
        "success",
        None,
    )
    .await
    .is_err()
        || tx.commit().await.is_err()
    {
        return error(
            &request,
            StatusCode::SERVICE_UNAVAILABLE,
            "database_unavailable",
        );
    };
    match provider_by(pool, &body.key).await {
        Ok(v) => (StatusCode::CREATED, Json(v)).into_response(),
        Err(e) => sql_error(&request, &e),
    }
}
async fn get_provider(
    State(state): State<AppState>,
    Path(key): Path<String>,
    request: Request,
) -> Response {
    let pool = match db(&state) {
        Ok(v) => v,
        Err(e) => return e,
    };
    match provider_by(pool, &key).await {
        Ok(v) => Json(v).into_response(),
        Err(sqlx::Error::RowNotFound) => error(&request, StatusCode::NOT_FOUND, "not_found"),
        Err(e) => sql_error(&request, &e),
    }
}
async fn list_providers(
    State(state): State<AppState>,
    Query(query): Query<PageQuery>,
    request: Request,
) -> Response {
    let (page, size) = match page_bounds(&query) {
        Ok(v) => v,
        Err(c) => return error(&request, StatusCode::BAD_REQUEST, c),
    };
    let pool = match db(&state) {
        Ok(v) => v,
        Err(e) => return e,
    };
    match sqlx::query_as::<_,Provider>("SELECT id,key,name,type AS kind,adapter,config_json,enabled,revision,created_at,updated_at,secret_ref IS NOT NULL AS has_secret_ref FROM providers ORDER BY key LIMIT ? OFFSET ?").bind(i64::from(size)).bind(i64::from((page-1)*size)).fetch_all(pool).await{Ok(items)=>Json(serde_json::json!({"items":items,"page":page,"page_size":size,"max_page_size":PAGE_MAX})).into_response(),Err(e)=>sql_error(&request,&e)}
}
async fn patch_provider(
    State(state): State<AppState>,
    Path(key): Path<String>,
    request: Request,
) -> Response {
    let (request, body): (_, PatchProvider) = match json(request).await {
        Ok(v) => v,
        Err(e) => return e,
    };
    if !matches!(body.key, Patch::Absent) {
        return error(&request, StatusCode::BAD_REQUEST, "immutable_field");
    }
    let expected = match expected(request.headers()) {
        Ok(v) => v,
        Err(code) => return error(&request, StatusCode::BAD_REQUEST, code),
    };
    let pool = match db(&state) {
        Ok(v) => v,
        Err(e) => return e,
    };
    let old: Result<(i64, String, String, String, String, Option<String>, i64, i64), _> =
        sqlx::query_as("SELECT id,name,type,adapter,config_json,secret_ref,enabled,revision FROM providers WHERE key=?")
            .bind(&key).fetch_one(pool).await;
    let (provider_id, old_name, kind, old_adapter, old_config, old_secret, old_enabled, revision) =
        match old {
            Ok(v) => v,
            Err(sqlx::Error::RowNotFound) => {
                return error(&request, StatusCode::NOT_FOUND, "not_found");
            }
            Err(e) => return sql_error(&request, &e),
        };
    if revision != expected {
        audit_conflict(pool, id(&request).into(), "provider", provider_id, expected).await;
        return error(&request, StatusCode::CONFLICT, "revision_conflict");
    }
    let name = match body.name.value() {
        Some(Some(v)) if valid_text(&v, 128, false) => v,
        Some(_) => return error(&request, StatusCode::BAD_REQUEST, "validation_failed"),
        None => old_name,
    };
    let adapter = match body.adapter.value() {
        Some(Some(v)) if !v.is_empty() => v,
        Some(_) => return error(&request, StatusCode::BAD_REQUEST, "validation_failed"),
        None => old_adapter,
    };
    if !adapter_matches_kind(&kind, &adapter) {
        return error(&request, StatusCode::BAD_REQUEST, "provider_config_invalid");
    }
    let config_value = match body.config_json.value() {
        Some(Some(v)) => v,
        Some(None) => return error(&request, StatusCode::BAD_REQUEST, "provider_config_invalid"),
        None => match serde_json::from_str(&old_config) {
            Ok(v) => v,
            Err(_) => return error(&request, StatusCode::BAD_REQUEST, "provider_config_invalid"),
        },
    };
    let config = match provider_config::validate(&adapter, &config_value) {
        Ok(v) => v,
        Err(_) => return error(&request, StatusCode::BAD_REQUEST, "provider_config_invalid"),
    };
    let secret = match body.secret_ref.value() {
        Some(Some(v)) if valid_secret_ref(&v) => Some(v),
        Some(Some(_)) => return error(&request, StatusCode::BAD_REQUEST, "validation_failed"),
        Some(None) => None,
        None => old_secret,
    };
    let enabled = match body.enabled.value() {
        Some(Some(v)) => i64::from(v),
        Some(None) => return error(&request, StatusCode::BAD_REQUEST, "validation_failed"),
        None => old_enabled,
    };
    let mut tx = match pool.begin().await {
        Ok(v) => v,
        Err(_) => {
            return error(
                &request,
                StatusCode::SERVICE_UNAVAILABLE,
                "database_unavailable",
            );
        }
    };
    let changed = sqlx::query("UPDATE providers SET name=?,adapter=?,config_json=?,secret_ref=?,enabled=?,revision=revision+1,updated_at=? WHERE id=? AND revision=?")
        .bind(name).bind(adapter).bind(config).bind(secret).bind(enabled).bind(now()).bind(provider_id).bind(expected)
        .execute(&mut *tx).await.map(|v| v.rows_affected() == 1).unwrap_or(false);
    if !changed {
        let _ = tx.rollback().await;
        return error(&request, StatusCode::CONFLICT, "revision_conflict");
    }
    if audit(
        &mut *tx,
        id(&request),
        "provider",
        provider_id,
        "update",
        Some(expected),
        Some(expected + 1),
        "success",
        None,
    )
    .await
    .is_err()
        || tx.commit().await.is_err()
    {
        return error(
            &request,
            StatusCode::SERVICE_UNAVAILABLE,
            "database_unavailable",
        );
    }
    let _ = kind; // type is immutable and retained for the provider instance.
    match provider_by(pool, &key).await {
        Ok(v) => Json(v).into_response(),
        Err(e) => sql_error(&request, &e),
    }
}

#[derive(FromRow)]
struct McpServerRow {
    id: i64,
    key: String,
    name: String,
    url: String,
    headers_json: String,
    auth_type: String,
    auth_header_name: Option<String>,
    secret_ref: Option<String>,
    connect_timeout_ms: i64,
    request_timeout_ms: i64,
    enabled: i64,
    revision: i64,
    created_at: i64,
    updated_at: i64,
}
#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum McpAuth {
    None,
    Bearer {
        secret_ref: String,
    },
    Header {
        header_name: String,
        secret_ref: String,
    },
}
#[derive(Deserialize)]
struct CreateMcpServer {
    key: String,
    name: String,
    url: String,
    #[serde(default)]
    headers: serde_json::Map<String, Value>,
    auth: McpAuth,
    #[serde(default = "default_connect_timeout")]
    connect_timeout_ms: u64,
    #[serde(default = "default_request_timeout")]
    request_timeout_ms: u64,
}
#[derive(Deserialize)]
struct PatchMcpServer {
    #[serde(default)]
    key: Patch<String>,
    #[serde(default)]
    name: Patch<String>,
    #[serde(default)]
    url: Patch<String>,
    #[serde(default)]
    headers: Patch<Value>,
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

fn valid_header_name(value: &str) -> bool {
    !value.is_empty()
        && value == value.to_ascii_lowercase()
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-')
}
fn protected_header(value: &str) -> bool {
    matches!(
        value,
        "host"
            | "content-length"
            | "transfer-encoding"
            | "connection"
            | "upgrade"
            | "te"
            | "trailer"
            | "proxy-authorization"
            | "proxy-authenticate"
            | "www-authenticate"
            | "keep-alive"
            | "cookie"
            | "set-cookie"
            | "authorization"
    )
}
fn valid_headers(headers: &serde_json::Map<String, Value>) -> bool {
    serde_json::to_vec(headers).is_ok_and(|raw| raw.len() <= 16 * 1024)
        && headers.iter().all(|(name, value)| {
            valid_header_name(name)
                && !protected_header(name)
                && value
                    .as_str()
                    .is_some_and(|v| v.len() <= 4096 && !v.bytes().any(|b| b < 0x20 || b == 0x7f))
        })
}
fn auth_parts(auth: McpAuth) -> Option<(String, Option<String>, Option<String>)> {
    match auth {
        McpAuth::None => Some(("none".into(), None, None)),
        McpAuth::Bearer { secret_ref } if valid_secret_ref(&secret_ref) => {
            Some(("bearer".into(), None, Some(secret_ref)))
        }
        McpAuth::Header {
            header_name,
            secret_ref,
        } if valid_header_name(&header_name)
            && !protected_header(&header_name)
            && valid_secret_ref(&secret_ref) =>
        {
            Some(("header".into(), Some(header_name), Some(secret_ref)))
        }
        _ => None,
    }
}
fn valid_mcp_url(url: &str, state: &AppState) -> bool {
    external_mcp_policy::valid_desired_url(url, &state.config.mcp.external.network)
}
fn mcp_json(row: McpServerRow) -> Value {
    let headers: Value =
        serde_json::from_str(&row.headers_json).unwrap_or(Value::Object(Default::default()));
    let auth = match row.auth_type.as_str() {
        "bearer" => serde_json::json!({"type":"bearer","has_secret_ref":row.secret_ref.is_some()}),
        "header" => {
            serde_json::json!({"type":"header","header_name":row.auth_header_name,"has_secret_ref":row.secret_ref.is_some()})
        }
        _ => serde_json::json!({"type":"none"}),
    };
    serde_json::json!({"key":row.key,"name":row.name,"transport":"streamable_http","url":row.url,"headers":headers,"auth":auth,"connect_timeout_ms":row.connect_timeout_ms,"request_timeout_ms":row.request_timeout_ms,"enabled":row.enabled != 0,"revision":row.revision,"created_at":row.created_at,"updated_at":row.updated_at})
}
async fn mcp_by(pool: &SqlitePool, key: &str) -> Result<McpServerRow, sqlx::Error> {
    sqlx::query_as("SELECT id,key,name,url,headers_json,auth_type,auth_header_name,secret_ref,connect_timeout_ms,request_timeout_ms,enabled,revision,created_at,updated_at FROM mcp_servers WHERE key=?").bind(key).fetch_one(pool).await
}
async fn create_mcp_server(State(state): State<AppState>, request: Request) -> Response {
    let (request, body): (_, CreateMcpServer) = match json(request).await {
        Ok(v) => v,
        Err(e) => return e,
    };
    let Some((auth_type, auth_header_name, secret_ref)) = auth_parts(body.auth) else {
        return error(&request, StatusCode::BAD_REQUEST, "validation_failed");
    };
    if !valid_key(&body.key)
        || !valid_text(&body.name, 128, false)
        || !valid_mcp_url(&body.url, &state)
        || !valid_headers(&body.headers)
        || !(1..=60_000).contains(&body.connect_timeout_ms)
        || !(1..=120_000).contains(&body.request_timeout_ms)
    {
        return error(&request, StatusCode::BAD_REQUEST, "validation_failed");
    }
    let pool = match db(&state) {
        Ok(v) => v,
        Err(e) => return e,
    };
    let mut tx = match pool.begin().await {
        Ok(v) => v,
        Err(_) => {
            return error(
                &request,
                StatusCode::SERVICE_UNAVAILABLE,
                "database_unavailable",
            );
        }
    };
    let headers_json = Value::Object(body.headers).to_string();
    let result = sqlx::query("INSERT INTO mcp_servers(key,name,url,headers_json,auth_type,auth_header_name,secret_ref,connect_timeout_ms,request_timeout_ms,created_at,updated_at) VALUES(?,?,?,?,?,?,?,?,?,?,?)").bind(&body.key).bind(&body.name).bind(&body.url).bind(headers_json).bind(auth_type).bind(auth_header_name).bind(secret_ref).bind(body.connect_timeout_ms as i64).bind(body.request_timeout_ms as i64).bind(now()).bind(now()).execute(&mut *tx).await;
    let resource_id = match result {
        Ok(v) => v.last_insert_rowid(),
        Err(e) => return mutation_sql_error(&request, &e),
    };
    if audit(
        &mut *tx,
        id(&request),
        "mcp_server",
        resource_id,
        "create",
        None,
        Some(1),
        "success",
        None,
    )
    .await
    .is_err()
        || tx.commit().await.is_err()
    {
        return error(
            &request,
            StatusCode::SERVICE_UNAVAILABLE,
            "database_unavailable",
        );
    }
    match mcp_by(pool, &body.key).await {
        Ok(row) => (StatusCode::CREATED, Json(mcp_json(row))).into_response(),
        Err(e) => sql_error(&request, &e),
    }
}
async fn get_mcp_server(
    State(state): State<AppState>,
    Path(key): Path<String>,
    request: Request,
) -> Response {
    let pool = match db(&state) {
        Ok(v) => v,
        Err(e) => return e,
    };
    match mcp_by(pool, &key).await {
        Ok(row) => Json(mcp_json(row)).into_response(),
        Err(sqlx::Error::RowNotFound) => error(&request, StatusCode::NOT_FOUND, "not_found"),
        Err(e) => sql_error(&request, &e),
    }
}
async fn list_mcp_servers(
    State(state): State<AppState>,
    Query(query): Query<PageQuery>,
    request: Request,
) -> Response {
    let (page, size) = match page_bounds(&query) {
        Ok(v) => v,
        Err(c) => return error(&request, StatusCode::BAD_REQUEST, c),
    };
    let pool = match db(&state) {
        Ok(v) => v,
        Err(e) => return e,
    };
    match sqlx::query_as::<_,McpServerRow>("SELECT id,key,name,url,headers_json,auth_type,auth_header_name,secret_ref,connect_timeout_ms,request_timeout_ms,enabled,revision,created_at,updated_at FROM mcp_servers ORDER BY key LIMIT ? OFFSET ?").bind(i64::from(size)).bind(i64::from((page-1)*size)).fetch_all(pool).await {Ok(rows)=>Json(serde_json::json!({"items":rows.into_iter().map(mcp_json).collect::<Vec<_>>(),"page":page,"page_size":size,"max_page_size":PAGE_MAX})).into_response(),Err(e)=>sql_error(&request,&e)}
}
async fn patch_mcp_server(
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
    let pool = match db(&state) {
        Ok(v) => v,
        Err(e) => return e,
    };
    let old = match mcp_by(pool, &key).await {
        Ok(v) => v,
        Err(sqlx::Error::RowNotFound) => {
            return error(&request, StatusCode::NOT_FOUND, "not_found");
        }
        Err(e) => return sql_error(&request, &e),
    };
    if old.revision != expected {
        audit_conflict(pool, id(&request).into(), "mcp_server", old.id, expected).await;
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
    let headers = match body.headers.value() {
        Some(Some(Value::Object(v))) if valid_headers(&v) => v,
        Some(_) => return error(&request, StatusCode::BAD_REQUEST, "validation_failed"),
        None => serde_json::from_str(&old.headers_json).unwrap_or_default(),
    };
    let (auth_type, auth_header_name, secret_ref) = match body.auth.value() {
        Some(Some(auth)) => match auth_parts(auth) {
            Some(v) => v,
            None => return error(&request, StatusCode::BAD_REQUEST, "validation_failed"),
        },
        Some(None) => return error(&request, StatusCode::BAD_REQUEST, "validation_failed"),
        None => (
            old.auth_type.clone(),
            old.auth_header_name.clone(),
            old.secret_ref.clone(),
        ),
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
    let mut tx = match pool.begin().await {
        Ok(v) => v,
        Err(_) => {
            return error(
                &request,
                StatusCode::SERVICE_UNAVAILABLE,
                "database_unavailable",
            );
        }
    };
    let updated=sqlx::query("UPDATE mcp_servers SET name=?,url=?,headers_json=?,auth_type=?,auth_header_name=?,secret_ref=?,connect_timeout_ms=?,request_timeout_ms=?,enabled=?,revision=revision+1,updated_at=? WHERE id=? AND revision=?").bind(name).bind(url).bind(Value::Object(headers).to_string()).bind(auth_type).bind(auth_header_name).bind(secret_ref).bind(connect_timeout).bind(request_timeout).bind(enabled).bind(now()).bind(old.id).bind(expected).execute(&mut *tx).await.map(|v|v.rows_affected()==1).unwrap_or(false);
    if !updated {
        let _ = tx.rollback().await;
        audit_conflict(pool, id(&request).into(), "mcp_server", old.id, expected).await;
        return error(&request, StatusCode::CONFLICT, "revision_conflict");
    };
    if audit(
        &mut *tx,
        id(&request),
        "mcp_server",
        old.id,
        "update",
        Some(expected),
        Some(expected + 1),
        "success",
        None,
    )
    .await
    .is_err()
        || tx.commit().await.is_err()
    {
        return error(
            &request,
            StatusCode::SERVICE_UNAVAILABLE,
            "database_unavailable",
        );
    };
    get_mcp_server(State(state), Path(key), request).await
}
async fn list_agent_mcp_bindings(
    State(state): State<AppState>,
    Path(key): Path<String>,
    request: Request,
) -> Response {
    let pool = match db(&state) {
        Ok(v) => v,
        Err(e) => return e,
    };
    let agent = match get_agent_by(pool, &key).await {
        Ok(v) => v,
        Err(sqlx::Error::RowNotFound) => {
            return error(&request, StatusCode::NOT_FOUND, "not_found");
        }
        Err(e) => return sql_error(&request, &e),
    };
    match sqlx::query_as::<_,(String,i64,i64)>("SELECT m.key,b.enabled,b.required FROM agent_mcp_bindings b JOIN mcp_servers m ON m.id=b.mcp_server_id WHERE b.agent_id=? ORDER BY m.key").bind(agent.id).fetch_all(pool).await { Ok(rows)=>Json(serde_json::json!({"items":rows.into_iter().map(|(server_key,enabled,required)|serde_json::json!({"server_key":server_key,"enabled":enabled!=0,"required":required!=0})).collect::<Vec<_>>() })).into_response(),Err(e)=>sql_error(&request,&e) }
}
async fn put_agent_mcp_binding(
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
    let pool = match db(&state) {
        Ok(v) => v,
        Err(e) => return e,
    };
    let agent = match get_agent_by(pool, &key).await {
        Ok(v) => v,
        Err(sqlx::Error::RowNotFound) => {
            return error(&request, StatusCode::NOT_FOUND, "not_found");
        }
        Err(e) => return sql_error(&request, &e),
    };
    if agent.revision != expected {
        audit_conflict(pool, id(&request).into(), "agent", agent.id, expected).await;
        return error(&request, StatusCode::CONFLICT, "revision_conflict");
    };
    let server = match mcp_by(pool, &server_key).await {
        Ok(v) => v,
        Err(sqlx::Error::RowNotFound) => {
            return error(&request, StatusCode::BAD_REQUEST, "invalid_mcp_server");
        }
        Err(e) => return sql_error(&request, &e),
    };
    let mut tx = match pool.begin().await {
        Ok(v) => v,
        Err(_) => {
            return error(
                &request,
                StatusCode::SERVICE_UNAVAILABLE,
                "database_unavailable",
            );
        }
    };
    let ok=sqlx::query("INSERT INTO agent_mcp_bindings(agent_id,mcp_server_id,enabled,required,created_at) VALUES(?,?,?,?,?) ON CONFLICT(agent_id,mcp_server_id) DO UPDATE SET enabled=excluded.enabled,required=excluded.required").bind(agent.id).bind(server.id).bind(i64::from(body.enabled)).bind(0i64).bind(now()).execute(&mut *tx).await.is_ok()&&sqlx::query("UPDATE agents SET revision=revision+1,updated_at=? WHERE id=? AND revision=?").bind(now()).bind(agent.id).bind(expected).execute(&mut *tx).await.map(|v|v.rows_affected()==1).unwrap_or(false);
    if !ok {
        let _ = tx.rollback().await;
        return error(&request, StatusCode::CONFLICT, "revision_conflict");
    };
    if audit(
        &mut *tx,
        id(&request),
        "agent",
        agent.id,
        "upsert_mcp_binding",
        Some(expected),
        Some(expected + 1),
        "success",
        None,
    )
    .await
    .is_err()
        || tx.commit().await.is_err()
    {
        return error(
            &request,
            StatusCode::SERVICE_UNAVAILABLE,
            "database_unavailable",
        );
    };
    match get_agent_by(pool, &key).await {
        Ok(v) => Json(v).into_response(),
        Err(e) => sql_error(&request, &e),
    }
}
