//! Admin agents resources.
use super::*;

#[derive(Serialize, FromRow)]
pub(super) struct Agent {
    pub(super) id: i64,
    pub(super) key: String,
    name: String,
    description: Option<String>,
    pub(super) enabled: i64,
    pub(super) revision: i64,
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
pub(super) async fn get_agent(
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
pub(super) async fn get_agent_by(pool: &SqlitePool, key: &str) -> Result<Agent, sqlx::Error> {
    sqlx::query_as("SELECT id,key,name,description,enabled,revision,created_at,updated_at FROM agents WHERE key=?").bind(key).fetch_one(pool).await
}
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
