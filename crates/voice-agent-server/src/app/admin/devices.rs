//! Admin devices resources.
use super::*;

#[derive(Serialize, FromRow)]
pub(super) struct Device {
    id: i64,
    device_id: String,
    agent_key: String,
    template_key: Option<String>,
    name: Option<String>,
    description: Option<String>,
    enabled: i64,
    metadata_json: Option<String>,
    revision: i64,
    created_at: i64,
    updated_at: i64,
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
    let agent: Result<(i64, i64), _> = sqlx::query_as("SELECT id, enabled FROM agents WHERE key=?")
        .bind(&body.agent_key)
        .fetch_one(&mut *tx)
        .await;
    let (agent_id, _) = match agent {
        Ok(value) if value.1 == 1 => value,
        _ => return error(&request, StatusCode::BAD_REQUEST, "invalid_agent"),
    };
    let template_id =
        match template_override_id(&mut tx, agent_id, body.template_key.as_deref()).await {
            Ok(value) => value,
            Err(sqlx::Error::RowNotFound) => {
                return error(
                    &request,
                    StatusCode::BAD_REQUEST,
                    "invalid_template_override",
                );
            }
            Err(error_value) => return sql_error(&request, &error_value),
        };
    let time = now();
    let result=sqlx::query("INSERT INTO devices (device_id,agent_id,template_id,name,description,metadata_json,created_at,updated_at) VALUES (?,?,?,?,?,?,?,?)").bind(&body.device_id).bind(agent_id).bind(template_id).bind(&body.name).bind(&body.description).bind(metadata).bind(time).bind(time).execute(&mut *tx).await;
    let device_id = match result {
        Ok(v) => v.last_insert_rowid(),
        Err(error_value) => return mutation_sql_error(&request, &error_value),
    };
    if let Some(database) = state.database.as_ref()
        && database
            .cancel_pending_enrollment(&mut tx, &body.device_id, time)
            .await
            .is_err()
    {
        return error(
            &request,
            StatusCode::SERVICE_UNAVAILABLE,
            "database_unavailable",
        );
    }
    if audit(
        &mut *tx,
        id(&request),
        "device",
        Some(device_id),
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
pub(super) async fn get_device(
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
pub(super) async fn get_device_by(
    pool: &SqlitePool,
    device_id: &str,
) -> Result<Device, sqlx::Error> {
    sqlx::query_as("SELECT d.id,d.device_id,a.key AS agent_key,t.key AS template_key,d.name,d.description,d.enabled,d.metadata_json,d.revision,d.created_at,d.updated_at FROM devices d JOIN agents a ON a.id=d.agent_id LEFT JOIN agent_templates t ON t.id=d.template_id WHERE d.device_id=?").bind(device_id).fetch_one(pool).await
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
        "SELECT d.id,d.device_id,a.key AS agent_key,t.key AS template_key,d.name,d.description,d.enabled,d.metadata_json,d.revision,d.created_at,d.updated_at FROM devices d JOIN agents a ON a.id=d.agent_id LEFT JOIN agent_templates t ON t.id=d.template_id WHERE (? IS NULL OR d.enabled=?) ORDER BY {order} LIMIT ? OFFSET ?"
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
    let agent: Result<(i64, i64), _> = sqlx::query_as("SELECT id, enabled FROM agents WHERE key=?")
        .bind(&agent_key)
        .fetch_one(&mut *tx)
        .await;
    let (agent_id, _) = match agent {
        Ok(value) if value.1 == 1 => value,
        _ => return error(&request, StatusCode::BAD_REQUEST, "invalid_agent"),
    };
    let template_id =
        match template_override_id(&mut tx, agent_id, requested_template_key.as_deref()).await {
            Ok(value) => value,
            Err(sqlx::Error::RowNotFound) => {
                return error(
                    &request,
                    StatusCode::BAD_REQUEST,
                    "invalid_template_override",
                );
            }
            Err(error_value) => return sql_error(&request, &error_value),
        };
    let update = sqlx::query("UPDATE devices SET agent_id=?,template_id=?,name=?,description=?,metadata_json=?,enabled=?,revision=revision+1,updated_at=? WHERE id=? AND revision=?").bind(agent_id).bind(template_id).bind(name).bind(description).bind(metadata).bind(enabled).bind(now()).bind(old.id).bind(expected).execute(&mut *tx).await;
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

/// Returns the selected Template only when it is enabled and has an enabled assignment to the
/// resulting Device Agent.  This is duplicated at admission as a fail-closed integrity check.
pub(super) async fn template_override_id(
    tx: &mut sqlx::Transaction<'_, Sqlite>,
    agent_id: i64,
    key: Option<&str>,
) -> Result<Option<i64>, sqlx::Error> {
    let Some(key) = key else {
        return Ok(None);
    };
    sqlx::query_scalar("SELECT t.id FROM agent_template_assignments ata JOIN agent_templates t ON t.id=ata.template_id WHERE ata.agent_id=? AND t.key=? AND ata.enabled=1 AND t.enabled=1")
        .bind(agent_id).bind(key).fetch_optional(&mut **tx).await?.ok_or(sqlx::Error::RowNotFound).map(Some)
}
