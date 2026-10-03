//! Admin mcp servers resources.
use super::agents::get_agent_by;
use super::providers::valid_secret_ref;
use super::*;

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
pub(super) async fn create_mcp_server(State(state): State<AppState>, request: Request) -> Response {
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
        Some(resource_id),
        "create",
        None,
        Some(1),
        AuditOutcome::Success,
        1,
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
pub(super) async fn get_mcp_server(
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
pub(super) async fn list_mcp_servers(
    State(state): State<AppState>,
    Query(query): Query<PageQuery>,
    request: Request,
) -> Response {
    let (page, size) = match page_bounds(query.page, query.page_size) {
        Ok(v) => v,
        Err(c) => return error(&request, StatusCode::BAD_REQUEST, c),
    };
    let pool = match db(&state) {
        Ok(v) => v,
        Err(e) => return e,
    };
    match sqlx::query_as::<_,McpServerRow>("SELECT id,key,name,url,headers_json,auth_type,auth_header_name,secret_ref,connect_timeout_ms,request_timeout_ms,enabled,revision,created_at,updated_at FROM mcp_servers ORDER BY key LIMIT ? OFFSET ?").bind(i64::from(size)).bind(i64::from((page-1)*size)).fetch_all(pool).await {Ok(rows)=>Json(serde_json::json!({"items":rows.into_iter().map(mcp_json).collect::<Vec<_>>(),"page":page,"page_size":size,"max_page_size":PAGE_MAX})).into_response(),Err(e)=>sql_error(&request,&e)}
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
        Some(old.id),
        "update",
        Some(expected),
        Some(expected + 1),
        AuditOutcome::Success,
        1,
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
pub(super) async fn list_agent_mcp_bindings(
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
        Some(agent.id),
        "upsert_mcp_binding",
        Some(expected),
        Some(expected + 1),
        AuditOutcome::Success,
        1,
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

pub(super) async fn unlink_agent_mcp_binding(
    State(state): State<AppState>,
    Path((key, server_key)): Path<(String, String)>,
    request: Request,
) -> Response {
    let expected = match expected(request.headers()) {
        Ok(value) => value,
        Err(code) => return error(&request, StatusCode::BAD_REQUEST, code),
    };
    let pool = match db(&state) {
        Ok(value) => value,
        Err(response) => return response,
    };
    let agent = match get_agent_by(pool, &key).await {
        Ok(value) => value,
        Err(sqlx::Error::RowNotFound) => {
            return error(&request, StatusCode::NOT_FOUND, "not_found");
        }
        Err(error_value) => return sql_error(&request, &error_value),
    };
    if agent.revision != expected {
        audit_conflict(pool, id(&request).into(), "agent", agent.id, expected).await;
        return error(&request, StatusCode::CONFLICT, "revision_conflict");
    }
    let mut tx = match pool.begin().await {
        Ok(value) => value,
        Err(_) => {
            return error(
                &request,
                StatusCode::SERVICE_UNAVAILABLE,
                "database_unavailable",
            );
        }
    };
    let deleted = match sqlx::query(
        "DELETE FROM agent_mcp_bindings WHERE agent_id=? AND mcp_server_id=(SELECT id FROM mcp_servers WHERE key=?)",
    )
    .bind(agent.id)
    .bind(&server_key)
    .execute(&mut *tx)
    .await
    {
        Ok(result) => result.rows_affected() == 1,
        Err(error_value) => {
            let _ = tx.rollback().await;
            return sql_error(&request, &error_value);
        }
    };
    if !deleted {
        let _ = tx.rollback().await;
        return error(&request, StatusCode::NOT_FOUND, "not_found");
    }
    let updated = match sqlx::query(
        "UPDATE agents SET revision=revision+1,updated_at=? WHERE id=? AND revision=?",
    )
    .bind(now())
    .bind(agent.id)
    .bind(expected)
    .execute(&mut *tx)
    .await
    {
        Ok(result) => result.rows_affected() == 1,
        Err(error_value) => {
            let _ = tx.rollback().await;
            return sql_error(&request, &error_value);
        }
    };
    if !updated {
        let _ = tx.rollback().await;
        audit_conflict(pool, id(&request).into(), "agent", agent.id, expected).await;
        return error(&request, StatusCode::CONFLICT, "revision_conflict");
    }
    if audit(
        &mut *tx,
        id(&request),
        "agent",
        Some(agent.id),
        "unlink_mcp_binding",
        Some(expected),
        Some(expected + 1),
        AuditOutcome::Success,
        1,
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
    StatusCode::NO_CONTENT.into_response()
}
