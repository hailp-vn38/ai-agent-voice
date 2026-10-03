//! Admin-side, authenticated consumption of an activation code.
use super::*;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ClaimEnrollment {
    code: String,
    agent_key: String,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    template_key: Option<String>,
}

pub(super) async fn claim(State(state): State<AppState>, request: Request) -> Response {
    if !state.config.database.devices.enrollment.enabled {
        return error(&request, StatusCode::NOT_FOUND, "not_found");
    }
    let (request, body): (_, ClaimEnrollment) = match json(request).await {
        Ok(value) => value,
        Err(response) => return response,
    };
    if !valid_code(&body.code)
        || !valid_key(&body.agent_key)
        || body
            .template_key
            .as_ref()
            .is_some_and(|key| !valid_key(key))
        || body
            .name
            .as_ref()
            .is_some_and(|name| !valid_text(name, 128, true))
    {
        return error(&request, StatusCode::BAD_REQUEST, "validation_failed");
    }
    if !state.admission_gate().is_open() {
        return error(
            &request,
            StatusCode::SERVICE_UNAVAILABLE,
            "server_shutting_down",
        );
    }
    let pool = match db(&state) {
        Ok(pool) => pool,
        Err(_) => {
            return error(
                &request,
                StatusCode::SERVICE_UNAVAILABLE,
                "database_unavailable",
            );
        }
    };
    let mut tx = match pool.begin_with("BEGIN IMMEDIATE").await {
        Ok(tx) => tx,
        Err(error_value) => return sql_error(&request, &error_value),
    };
    let row: Option<(String, String, i64, String)> = match sqlx::query_as(
        "SELECT status,device_id,expires_at,metadata_json FROM device_enrollments WHERE code=?",
    )
    .bind(&body.code)
    .fetch_optional(&mut *tx)
    .await
    {
        Ok(row) => row,
        Err(error_value) => return sql_error(&request, &error_value),
    };
    let Some((status, device_identity, expires_at, metadata_json)) = row else {
        return error(&request, StatusCode::NOT_FOUND, "enrollment_code_invalid");
    };
    match status.as_str() {
        "claimed" => return error(&request, StatusCode::CONFLICT, "enrollment_already_claimed"),
        "cancelled" => return error(&request, StatusCode::CONFLICT, "enrollment_cancelled"),
        "expired" => return error(&request, StatusCode::GONE, "enrollment_code_expired"),
        "pending" if now() >= expires_at => {
            return error(&request, StatusCode::GONE, "enrollment_code_expired");
        }
        "pending" => {}
        _ => {
            return error(
                &request,
                StatusCode::SERVICE_UNAVAILABLE,
                "database_unavailable",
            );
        }
    }
    let agent: Result<(i64, i64), _> = sqlx::query_as("SELECT id,enabled FROM agents WHERE key=?")
        .bind(&body.agent_key)
        .fetch_one(&mut *tx)
        .await;
    let (agent_id, enabled) = match agent {
        Ok(value) => value,
        Err(sqlx::Error::RowNotFound) => {
            return error(&request, StatusCode::BAD_REQUEST, "invalid_agent");
        }
        Err(error_value) => return sql_error(&request, &error_value),
    };
    if enabled != 1 {
        return error(&request, StatusCode::BAD_REQUEST, "invalid_agent");
    }
    let template_id =
        match super::devices::template_override_id(&mut tx, agent_id, body.template_key.as_deref())
            .await
        {
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
    let existing: Result<Option<(i64,)>, _> =
        sqlx::query_as("SELECT id FROM devices WHERE device_id=?")
            .bind(&device_identity)
            .fetch_optional(&mut *tx)
            .await;
    match existing {
        Ok(None) => {}
        Ok(Some(_)) => return error(&request, StatusCode::CONFLICT, "device_already_registered"),
        Err(error_value) => return sql_error(&request, &error_value),
    }
    let timestamp = now();
    let device_row = match sqlx::query("INSERT INTO devices (device_id,agent_id,template_id,name,metadata_json,created_at,updated_at) VALUES (?,?,?,?,?,?,?)")
        .bind(&device_identity).bind(agent_id).bind(template_id).bind(&body.name).bind(metadata_json).bind(timestamp).bind(timestamp).execute(&mut *tx).await {
        Ok(result) => result.last_insert_rowid(),
        Err(error_value) if sql_error_kind(&error_value) == "database_busy" => return sql_error(&request, &error_value),
        Err(error_value) if is_unique_constraint(&error_value) => {
            return error(&request, StatusCode::CONFLICT, "device_already_registered");
        }
        Err(error_value) => return sql_error(&request, &error_value),
    };
    let claimed = match sqlx::query("UPDATE device_enrollments SET status='claimed',claimed_device_id=?,terminal_at=? WHERE code=? AND status='pending' AND expires_at>?")
        .bind(device_row).bind(timestamp).bind(&body.code).bind(timestamp).execute(&mut *tx).await {
        Ok(result) => result.rows_affected(), Err(error_value) => return sql_error(&request, &error_value),
    };
    if claimed != 1 {
        return error(&request, StatusCode::CONFLICT, "enrollment_already_claimed");
    }
    if audit(
        &mut *tx,
        id(&request),
        "device",
        Some(device_row),
        "create",
        None,
        Some(1),
        AuditOutcome::Success,
        1,
    )
    .await
    .is_err()
        || audit(
            &mut *tx,
            id(&request),
            "device_enrollment",
            None,
            "claim",
            None,
            None,
            AuditOutcome::Success,
            1,
        )
        .await
        .is_err()
    {
        return error(
            &request,
            StatusCode::SERVICE_UNAVAILABLE,
            "database_unavailable",
        );
    }
    let device: super::devices::Device = match sqlx::query_as("SELECT d.id,d.device_id,a.key AS agent_key,t.key AS template_key,d.name,d.description,d.enabled,d.metadata_json,d.revision,d.created_at,d.updated_at FROM devices d JOIN agents a ON a.id=d.agent_id LEFT JOIN agent_templates t ON t.id=d.template_id WHERE d.id=?")
        .bind(device_row).fetch_one(&mut *tx).await { Ok(device) => device, Err(error_value) => return sql_error(&request, &error_value) };
    if tx.commit().await.is_err() {
        return error(
            &request,
            StatusCode::SERVICE_UNAVAILABLE,
            "database_unavailable",
        );
    }
    (StatusCode::CREATED, Json(device)).into_response()
}

fn valid_code(value: &str) -> bool {
    value.len() == 6 && value.bytes().all(|byte| byte.is_ascii_digit())
}

fn is_unique_constraint(error: &sqlx::Error) -> bool {
    matches!(error, sqlx::Error::Database(database_error) if matches!(database_error.code().as_deref(), Some("1555" | "2067" | "SQLITE_CONSTRAINT_UNIQUE")))
}
