//! Bounded, separately authenticated Admin HTTP surface.  It deliberately exposes only
//! Agent and Device desired configuration in this rollout.
use super::AppState;
use axum::{
    Json, Router,
    body::{Body, to_bytes},
    extract::{Path, Query, Request, State},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    middleware::Next,
    response::{IntoResponse, Response},
    routing::get,
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
    if !content_type
        .split(';')
        .next()
        .is_some_and(|v| v.trim().eq_ignore_ascii_case("application/json"))
    {
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
    #[default]
    Absent,
    Clear(Option<T>),
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
        Err(_) => return error(&request, StatusCode::BAD_REQUEST, "resource_conflict"),
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
    if query.page.unwrap_or(1) == 0
        || query.page.unwrap_or(1) > 10_000
        || query
            .sort
            .as_deref()
            .is_some_and(|v| !matches!(v, "key" | "name"))
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
    let page = query.page.unwrap_or(1);
    let enabled = query.enabled.map(i64::from);
    let rows=sqlx::query_as::<_,Agent>("SELECT id,key,name,description,enabled,revision,created_at,updated_at FROM agents WHERE (? IS NULL OR enabled=?) ORDER BY key LIMIT ? OFFSET ?").bind(enabled).bind(enabled).bind(i64::from(PAGE_DEFAULT)).bind(i64::from((page-1)*PAGE_DEFAULT)).fetch_all(pool).await;
    match rows {Ok(items)=>Json(serde_json::json!({"items":items,"page":page,"page_size":PAGE_DEFAULT,"max_page_size":PAGE_MAX})).into_response(),Err(error_value)=>sql_error(&request, &error_value)}
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
        Err(_) => return error(&request, StatusCode::NOT_FOUND, "not_found"),
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
        Err(_) => return error(&request, StatusCode::BAD_REQUEST, "resource_conflict"),
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
    get_device_by(pool, &device_id)
        .await
        .map(|v| Json(v).into_response())
        .unwrap_or_else(|_| error(&request, StatusCode::NOT_FOUND, "not_found"))
}
async fn get_device_by(pool: &SqlitePool, device_id: &str) -> Result<Device, sqlx::Error> {
    sqlx::query_as("SELECT d.id,d.device_id,a.key AS agent_key,d.name,d.description,d.enabled,d.metadata_json,d.revision,d.created_at,d.updated_at FROM devices d JOIN agents a ON a.id=d.agent_id WHERE d.device_id=?").bind(device_id).fetch_one(pool).await
}
async fn list_devices(
    State(state): State<AppState>,
    Query(query): Query<PageQuery>,
    request: Request,
) -> Response {
    if query.page.unwrap_or(1) == 0
        || query.page.unwrap_or(1) > 10_000
        || query
            .sort
            .as_deref()
            .is_some_and(|v| !matches!(v, "device_id" | "name"))
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
    let page = query.page.unwrap_or(1);
    let enabled = query.enabled.map(i64::from);
    let rows=sqlx::query_as::<_,Device>("SELECT d.id,d.device_id,a.key AS agent_key,d.name,d.description,d.enabled,d.metadata_json,d.revision,d.created_at,d.updated_at FROM devices d JOIN agents a ON a.id=d.agent_id WHERE (? IS NULL OR d.enabled=?) ORDER BY d.device_id LIMIT ? OFFSET ?").bind(enabled).bind(enabled).bind(i64::from(PAGE_DEFAULT)).bind(i64::from((page-1)*PAGE_DEFAULT)).fetch_all(pool).await;
    match rows{Ok(items)=>Json(serde_json::json!({"items":items,"page":page,"page_size":PAGE_DEFAULT,"max_page_size":PAGE_MAX})).into_response(),Err(_)=>error(&request,StatusCode::SERVICE_UNAVAILABLE,"database_unavailable")}
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
        Err(_) => return error(&request, StatusCode::NOT_FOUND, "not_found"),
    };
    if old.revision != expected {
        {
            audit_conflict(pool, id(&request).to_owned(), "device", old.id, expected).await;
            return error(&request, StatusCode::CONFLICT, "revision_conflict");
        }
    };
    let agent_key = body
        .agent_key
        .value()
        .flatten()
        .unwrap_or(old.agent_key.clone());
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
    let enabled = body
        .enabled
        .value()
        .flatten()
        .map(i64::from)
        .unwrap_or(old.enabled);
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
