//! Conditional hard-delete endpoints for desired configuration.

use super::*;

struct DeleteSpec {
    resource: &'static str,
    select_sql: &'static str,
    dependency_sql: &'static str,
    dependency_binds: usize,
    delete_sql: &'static str,
    in_use_code: &'static str,
}

const AGENT: DeleteSpec = DeleteSpec {
    resource: "agent",
    select_sql: "SELECT id,revision FROM agents WHERE key=?",
    dependency_sql: "SELECT EXISTS(SELECT 1 FROM devices WHERE agent_id=? UNION ALL SELECT 1 FROM agent_mcp_bindings WHERE agent_id=? UNION ALL SELECT 1 FROM history_messages WHERE agent_id=?)",
    dependency_binds: 3,
    // The assignment table cascades on Agent deletion. This removes only the relationship; the
    // globally reusable Template itself stays intact and may remain linked to other Agents.
    delete_sql: "DELETE FROM agents WHERE id=? AND revision=? AND NOT EXISTS(SELECT 1 FROM devices WHERE agent_id=?) AND NOT EXISTS(SELECT 1 FROM agent_mcp_bindings WHERE agent_id=?) AND NOT EXISTS(SELECT 1 FROM history_messages WHERE agent_id=?)",
    in_use_code: "agent_in_use",
};

const DEVICE: DeleteSpec = DeleteSpec {
    resource: "device",
    select_sql: "SELECT id,revision FROM devices WHERE device_id=?",
    dependency_sql: "SELECT EXISTS(SELECT 1 FROM history_messages WHERE device_id=?)",
    dependency_binds: 1,
    delete_sql: "DELETE FROM devices WHERE id=? AND revision=? AND NOT EXISTS(SELECT 1 FROM history_messages WHERE device_id=?)",
    in_use_code: "device_in_use",
};

const TEMPLATE: DeleteSpec = DeleteSpec {
    resource: "template",
    select_sql: "SELECT id,revision FROM agent_templates WHERE key=?",
    dependency_sql: "SELECT EXISTS(SELECT 1 FROM agent_template_assignments WHERE template_id=? AND enabled=1 UNION ALL SELECT 1 FROM template_provider_bindings WHERE template_id=? UNION ALL SELECT 1 FROM devices WHERE template_id=? UNION ALL SELECT 1 FROM history_messages WHERE template_id=?)",
    dependency_binds: 4,
    delete_sql: "DELETE FROM agent_templates WHERE id=? AND revision=? AND NOT EXISTS(SELECT 1 FROM agent_template_assignments WHERE template_id=? AND enabled=1) AND NOT EXISTS(SELECT 1 FROM template_provider_bindings WHERE template_id=?) AND NOT EXISTS(SELECT 1 FROM devices WHERE template_id=?) AND NOT EXISTS(SELECT 1 FROM history_messages WHERE template_id=?)",
    in_use_code: "template_in_use",
};

const PROVIDER: DeleteSpec = DeleteSpec {
    resource: "provider",
    select_sql: "SELECT id,revision FROM providers WHERE key=?",
    dependency_sql: "SELECT EXISTS(SELECT 1 FROM template_provider_bindings WHERE provider_id=?)",
    dependency_binds: 1,
    delete_sql: "DELETE FROM providers WHERE id=? AND revision=? AND NOT EXISTS(SELECT 1 FROM template_provider_bindings WHERE provider_id=?)",
    in_use_code: "provider_in_use",
};

const MCP_SERVER: DeleteSpec = DeleteSpec {
    resource: "mcp_server",
    select_sql: "SELECT id,revision FROM mcp_servers WHERE key=?",
    dependency_sql: "SELECT EXISTS(SELECT 1 FROM agent_mcp_bindings WHERE mcp_server_id=?)",
    dependency_binds: 1,
    delete_sql: "DELETE FROM mcp_servers WHERE id=? AND revision=? AND NOT EXISTS(SELECT 1 FROM agent_mcp_bindings WHERE mcp_server_id=?)",
    in_use_code: "mcp_server_in_use",
};

pub(super) async fn delete_agent(
    State(state): State<AppState>,
    Path(key): Path<String>,
    request: Request,
) -> Response {
    delete_by_key(&state, &key, request, &AGENT).await
}

pub(super) async fn delete_device(
    State(state): State<AppState>,
    Path(device_id): Path<String>,
    request: Request,
) -> Response {
    delete_by_key(&state, &device_id, request, &DEVICE).await
}

pub(super) async fn delete_template(
    State(state): State<AppState>,
    Path(key): Path<String>,
    request: Request,
) -> Response {
    delete_by_key(&state, &key, request, &TEMPLATE).await
}

pub(super) async fn delete_provider(
    State(state): State<AppState>,
    Path(key): Path<String>,
    request: Request,
) -> Response {
    delete_by_key(&state, &key, request, &PROVIDER).await
}

pub(super) async fn delete_mcp_server(
    State(state): State<AppState>,
    Path(key): Path<String>,
    request: Request,
) -> Response {
    delete_by_key(&state, &key, request, &MCP_SERVER).await
}

async fn dependency_exists<'e, E>(
    executor: E,
    resource_id: i64,
    spec: &DeleteSpec,
) -> Result<bool, sqlx::Error>
where
    E: sqlx::Executor<'e, Database = Sqlite>,
{
    let mut query = sqlx::query_scalar::<_, i64>(spec.dependency_sql);
    for _ in 0..spec.dependency_binds {
        query = query.bind(resource_id);
    }
    query.fetch_one(executor).await.map(|value| value != 0)
}

async fn delete_by_key(
    state: &AppState,
    key: &str,
    request: Request,
    spec: &DeleteSpec,
) -> Response {
    let expected = match expected(request.headers()) {
        Ok(value) => value,
        Err(code) => return error(&request, StatusCode::BAD_REQUEST, code),
    };
    let pool = match db(state) {
        Ok(value) => value,
        Err(response) => return response,
    };
    let (resource_id, revision): (i64, i64) = match sqlx::query_as(spec.select_sql)
        .bind(key)
        .fetch_one(pool)
        .await
    {
        Ok(value) => value,
        Err(sqlx::Error::RowNotFound) => {
            return error(&request, StatusCode::NOT_FOUND, "not_found");
        }
        Err(error_value) => return sql_error(&request, &error_value),
    };
    if revision != expected {
        audit_conflict_action(
            pool,
            id(&request).into(),
            spec.resource,
            resource_id,
            expected,
            "delete",
        )
        .await;
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
    let in_use = match dependency_exists(&mut *tx, resource_id, spec).await {
        Ok(value) => value,
        Err(error_value) => {
            let _ = tx.rollback().await;
            return sql_error(&request, &error_value);
        }
    };
    if in_use {
        let _ = tx.rollback().await;
        return error(&request, StatusCode::CONFLICT, spec.in_use_code);
    }
    let mut delete = sqlx::query(spec.delete_sql)
        .bind(resource_id)
        .bind(expected);
    for _ in 0..spec.dependency_binds {
        delete = delete.bind(resource_id);
    }
    let deleted = match delete.execute(&mut *tx).await {
        Ok(result) => result.rows_affected() == 1,
        Err(error_value) => {
            let _ = tx.rollback().await;
            return sql_error(&request, &error_value);
        }
    };
    if !deleted {
        let in_use = match dependency_exists(&mut *tx, resource_id, spec).await {
            Ok(value) => value,
            Err(error_value) => {
                let _ = tx.rollback().await;
                return sql_error(&request, &error_value);
            }
        };
        let _ = tx.rollback().await;
        if in_use {
            return error(&request, StatusCode::CONFLICT, spec.in_use_code);
        }
        audit_conflict_action(
            pool,
            id(&request).into(),
            spec.resource,
            resource_id,
            expected,
            "delete",
        )
        .await;
        return error(&request, StatusCode::CONFLICT, "revision_conflict");
    }
    if audit(
        &mut *tx,
        id(&request),
        spec.resource,
        Some(resource_id),
        "delete",
        Some(expected),
        None,
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
