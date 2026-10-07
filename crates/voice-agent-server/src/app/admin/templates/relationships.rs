//! Read and unlink operations for persisted Template relationships.

use super::*;

#[allow(clippy::result_large_err)] // Handler callers return this HTTP response unchanged.
fn relationship_page(query: PageQuery, request: &Request) -> Result<(u32, u32), Response> {
    match page_bounds(query.page, query.page_size) {
        Ok(value) if query.enabled.is_none() && query.sort.is_none() => Ok(value),
        _ => Err(error(request, StatusCode::BAD_REQUEST, "invalid_query")),
    }
}

pub(in crate::app::admin) async fn list_agent_templates(
    State(state): State<AppState>,
    Path(agent_key): Path<String>,
    Query(query): Query<PageQuery>,
    request: Request,
) -> Response {
    let (page, page_size) = match relationship_page(query, &request) {
        Ok(value) => value,
        Err(response) => return response,
    };
    let pool = match db(&state) {
        Ok(value) => value,
        Err(response) => return response,
    };
    let agent = match get_agent_by(pool, &agent_key).await {
        Ok(value) => value,
        Err(sqlx::Error::RowNotFound) => {
            return error(&request, StatusCode::NOT_FOUND, "not_found");
        }
        Err(error_value) => return sql_error(&request, &error_value),
    };
    type AgentTemplateRow = (String, String, String, i64, i64);
    let rows: Result<Vec<AgentTemplateRow>, _> = sqlx::query_as(
        "SELECT t.key,t.name,t.language,t.enabled,a.is_default \
         FROM agent_template_assignments a JOIN agent_templates t ON t.id=a.template_id \
         WHERE a.agent_id=? AND a.enabled=1 ORDER BY t.key LIMIT ? OFFSET ?",
    )
    .bind(agent.id)
    .bind(i64::from(page_size))
    .bind(i64::from((page - 1) * page_size))
    .fetch_all(pool)
    .await;
    match rows {
        Ok(rows) => Json(serde_json::json!({
            "agent_key": agent_key,
            "revision": agent.revision,
            "page": page,
            "page_size": page_size,
            "max_page_size": PAGE_MAX,
            "items": rows.into_iter().map(|(key, name, language, enabled, is_default)| serde_json::json!({
                "key": key,
                "name": name,
                "language": language,
                "enabled": enabled != 0,
                "is_default": is_default != 0,
            })).collect::<Vec<_>>(),
        }))
        .into_response(),
        Err(error_value) => sql_error(&request, &error_value),
    }
}

pub(in crate::app::admin) async fn list_template_agents(
    State(state): State<AppState>,
    Path(template_key): Path<String>,
    Query(query): Query<PageQuery>,
    request: Request,
) -> Response {
    let (page, page_size) = match relationship_page(query, &request) {
        Ok(value) => value,
        Err(response) => return response,
    };
    let pool = match db(&state) {
        Ok(value) => value,
        Err(response) => return response,
    };
    let template = match template_by(pool, &template_key).await {
        Ok(value) => value,
        Err(sqlx::Error::RowNotFound) => {
            return error(&request, StatusCode::NOT_FOUND, "not_found");
        }
        Err(error_value) => return sql_error(&request, &error_value),
    };
    let total: i64 = match sqlx::query_scalar(
        "SELECT COUNT(*) FROM agent_template_assignments WHERE template_id=? AND enabled=1",
    )
    .bind(template.id)
    .fetch_one(pool)
    .await
    {
        Ok(value) => value,
        Err(error_value) => return sql_error(&request, &error_value),
    };
    let rows: Result<Vec<(String, String, i64, i64)>, _> = sqlx::query_as(
        "SELECT a.key,a.name,a.enabled,ata.is_default \
         FROM agent_template_assignments ata JOIN agents a ON a.id=ata.agent_id \
         WHERE ata.template_id=? AND ata.enabled=1 ORDER BY a.key LIMIT ? OFFSET ?",
    )
    .bind(template.id)
    .bind(i64::from(page_size))
    .bind(i64::from((page - 1) * page_size))
    .fetch_all(pool)
    .await;
    match rows {
        Ok(rows) => Json(serde_json::json!({
            "template_key": template_key,
            "revision": template.revision,
            "page": page,
            "page_size": page_size,
            "max_page_size": PAGE_MAX,
            "total": total,
            "items": rows.into_iter().map(|(key, name, enabled, is_default)| serde_json::json!({
                "key": key,
                "name": name,
                "enabled": enabled != 0,
                "is_default": is_default != 0,
            })).collect::<Vec<_>>(),
        }))
        .into_response(),
        Err(error_value) => sql_error(&request, &error_value),
    }
}

pub(in crate::app::admin) async fn list_template_providers(
    State(state): State<AppState>,
    Path(template_key): Path<String>,
    request: Request,
) -> Response {
    let pool = match db(&state) {
        Ok(value) => value,
        Err(response) => return response,
    };
    let template = match template_by(pool, &template_key).await {
        Ok(value) => value,
        Err(sqlx::Error::RowNotFound) => {
            return error(&request, StatusCode::NOT_FOUND, "not_found");
        }
        Err(error_value) => return sql_error(&request, &error_value),
    };
    let rows: Result<Vec<(String, String, i64)>, _> = sqlx::query_as(
        "SELECT b.provider_type,p.key,p.enabled FROM template_provider_bindings b \
         JOIN providers p ON p.id=b.provider_id WHERE b.template_id=? ORDER BY b.provider_type",
    )
    .bind(template.id)
    .fetch_all(pool)
    .await;
    match rows {
        Ok(rows) => Json(serde_json::json!({
            "template_key": template_key,
            "revision": template.revision,
            "bindings": rows.into_iter().map(|(provider_type, provider_key, enabled)| (
                provider_type,
                serde_json::json!({"provider_key": provider_key, "enabled": enabled != 0}),
            )).collect::<serde_json::Map<String, Value>>(),
        }))
        .into_response(),
        Err(error_value) => sql_error(&request, &error_value),
    }
}

pub(in crate::app::admin) async fn unlink_template_provider(
    State(state): State<AppState>,
    Path((key, provider_type)): Path<(String, String)>,
    request: Request,
) -> Response {
    if !matches!(
        provider_type.as_str(),
        "vad" | "asr" | "llm" | "tts" | "speaker"
    ) {
        return error(&request, StatusCode::BAD_REQUEST, "validation_failed");
    }
    let expected = match expected(request.headers()) {
        Ok(value) => value,
        Err(code) => return error(&request, StatusCode::BAD_REQUEST, code),
    };
    let pool = match db(&state) {
        Ok(value) => value,
        Err(response) => return response,
    };
    let template = match template_by(pool, &key).await {
        Ok(value) => value,
        Err(sqlx::Error::RowNotFound) => {
            return error(&request, StatusCode::NOT_FOUND, "not_found");
        }
        Err(error_value) => return sql_error(&request, &error_value),
    };
    if template.revision != expected {
        audit_conflict(pool, id(&request).into(), "template", template.id, expected).await;
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
        "DELETE FROM template_provider_bindings WHERE template_id=? AND provider_type=?",
    )
    .bind(template.id)
    .bind(&provider_type)
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
        "UPDATE agent_templates SET revision=revision+1,updated_at=? WHERE id=? AND revision=?",
    )
    .bind(now())
    .bind(template.id)
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
        audit_conflict(pool, id(&request).into(), "template", template.id, expected).await;
        return error(&request, StatusCode::CONFLICT, "revision_conflict");
    }
    if audit(
        &mut *tx,
        id(&request),
        "template",
        Some(template.id),
        "unlink_provider",
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
    get_template(State(state), Path(key), request).await
}

pub(in crate::app::admin) async fn unlink_agent_template(
    State(state): State<AppState>,
    Path((agent_key, template_key)): Path<(String, String)>,
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
    let agent = match get_agent_by(pool, &agent_key).await {
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
    let assignment: Result<(i64,), _> = sqlx::query_as(
        "SELECT ata.id FROM agent_template_assignments ata \
         JOIN agent_templates t ON t.id=ata.template_id \
         WHERE ata.agent_id=? AND t.key=? AND ata.enabled=1",
    )
    .bind(agent.id)
    .bind(&template_key)
    .fetch_one(pool)
    .await;
    let (assignment_id,) = match assignment {
        Ok(value) => value,
        Err(sqlx::Error::RowNotFound) => {
            return error(&request, StatusCode::NOT_FOUND, "not_found");
        }
        Err(error_value) => return sql_error(&request, &error_value),
    };
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
    let unlinked = match sqlx::query(
        "UPDATE agent_template_assignments SET enabled=0,is_default=0 WHERE id=? AND enabled=1",
    )
    .bind(assignment_id)
    .execute(&mut *tx)
    .await
    {
        Ok(result) => result.rows_affected() == 1,
        Err(error_value) => {
            let _ = tx.rollback().await;
            return sql_error(&request, &error_value);
        }
    };
    let updated = if unlinked {
        match sqlx::query(
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
        }
    } else {
        false
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
        "unlink_template",
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
    match get_agent_by(pool, &agent_key).await {
        Ok(agent) => Json(agent).into_response(),
        Err(error_value) => sql_error(&request, &error_value),
    }
}
