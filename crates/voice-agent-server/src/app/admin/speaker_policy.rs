//! Agent speaker policy (`off` / `observe` / `required`) and per-Template grants.
//!
//! Policy has its own revision, independent of the Agent revision that grants CAS against.  An
//! absent policy row is the `off` contract with revision 1, so a fresh Agent reads consistently
//! before any admin has touched it.
use super::agents::get_agent_by;
use super::speakers::with_etag;
use super::*;

const POLICY_MODES: [&str; 3] = ["off", "observe", "required"];
const MAX_GRANT_TEMPLATES: usize = 32;

/// Why Agent `required` cannot be enabled yet.  Ticket 14 supplies calibration qualification; the
/// `required` is only selectable once the Agent has qualified calibration evidence; the fresh-turn
/// gate (ticket 15) then enforces per-turn verification at runtime. Without qualification, fail
/// closed.
fn required_blockers(qualified: bool) -> Vec<&'static str> {
    if qualified {
        Vec::new()
    } else {
        vec!["speaker_calibration_required"]
    }
}

async fn policy_state(pool: &SqlitePool, agent_id: i64) -> Result<(String, i64), sqlx::Error> {
    let row = sqlx::query_as::<_, (String, i64)>(
        "SELECT mode, revision FROM agent_speaker_policies WHERE agent_id = ?",
    )
    .bind(agent_id)
    .fetch_optional(pool)
    .await?;
    Ok(row.unwrap_or_else(|| ("off".to_owned(), 1)))
}

fn policy_body(agent_key: &str, mode: &str, revision: i64, qualified: bool) -> Value {
    let blockers = required_blockers(qualified);
    serde_json::json!({
        "agent_key": agent_key,
        "mode": mode,
        "revision": revision,
        "verification_scope": "every_voice_turn",
        "speaker_change": "reconnect",
        "text_turns": "allowed_without_speaker_authority",
        "required_available": blockers.is_empty(),
        "required_blockers": blockers,
    })
}

fn agent_lookup_error(request: &Request, error_value: &sqlx::Error) -> Response {
    if matches!(error_value, sqlx::Error::RowNotFound) {
        error(request, StatusCode::NOT_FOUND, "not_found")
    } else {
        sql_error(request, error_value)
    }
}

/// `GET /agents/{agent_key}/speaker-policy` — policy with its own revision and ETag.
pub(super) async fn get_policy(
    State(state): State<AppState>,
    Path(key): Path<String>,
    request: Request,
) -> Response {
    let pool = match db(&state) {
        Ok(pool) => pool,
        Err(response) => return response,
    };
    let agent = match get_agent_by(pool, &key).await {
        Ok(agent) => agent,
        Err(error_value) => return agent_lookup_error(&request, &error_value),
    };
    let (mode, revision) = match policy_state(pool, agent.id).await {
        Ok(state) => state,
        Err(error_value) => return sql_error(&request, &error_value),
    };
    let qualified = match speaker_calibration::qualification(pool, agent.id, &agent.key).await {
        Ok(qualification) => qualification.is_qualified(),
        Err(error_value) => return sql_error(&request, &error_value),
    };
    with_etag(
        Json(policy_body(&agent.key, &mode, revision, qualified)).into_response(),
        revision,
    )
}

#[derive(Deserialize)]
struct PolicyPut {
    mode: String,
}

/// `PUT /agents/{agent_key}/speaker-policy` — CAS on the policy revision.
pub(super) async fn put_policy(
    State(state): State<AppState>,
    Path(key): Path<String>,
    request: Request,
) -> Response {
    let (request, body): (_, PolicyPut) = match json(request).await {
        Ok(value) => value,
        Err(response) => return response,
    };
    if !POLICY_MODES.contains(&body.mode.as_str()) {
        return error(&request, StatusCode::BAD_REQUEST, "validation_failed");
    }
    let expected = match expected(request.headers()) {
        Ok(value) => value,
        Err(code) => return error(&request, StatusCode::BAD_REQUEST, code),
    };
    let pool = match db(&state) {
        Ok(pool) => pool,
        Err(response) => return response,
    };
    let agent = match get_agent_by(pool, &key).await {
        Ok(agent) => agent,
        Err(error_value) => return agent_lookup_error(&request, &error_value),
    };
    let (previous_mode, current) = match policy_state(pool, agent.id).await {
        Ok(state) => state,
        Err(error_value) => return sql_error(&request, &error_value),
    };
    if expected != current {
        audit_conflict(
            pool,
            id(&request).to_owned(),
            "agent_speaker_policy",
            agent.id,
            expected,
        )
        .await;
        return error(&request, StatusCode::CONFLICT, "revision_conflict");
    }
    if body.mode == "required" {
        let qualified = match speaker_calibration::qualification(pool, agent.id, &agent.key).await {
            Ok(qualification) => qualification.is_qualified(),
            Err(error_value) => return sql_error(&request, &error_value),
        };
        if let Some(blocker) = required_blockers(qualified).first() {
            return error(&request, StatusCode::SERVICE_UNAVAILABLE, blocker);
        }
    }

    let next = current + 1;
    let mut tx = match pool.begin().await {
        Ok(tx) => tx,
        Err(error_value) => return sql_error(&request, &error_value),
    };
    let written = sqlx::query(
        "INSERT INTO agent_speaker_policies (agent_id, mode, revision) VALUES (?, ?, ?) \
         ON CONFLICT(agent_id) DO UPDATE SET mode = excluded.mode, revision = excluded.revision \
         WHERE agent_speaker_policies.revision = ?",
    )
    .bind(agent.id)
    .bind(&body.mode)
    .bind(next)
    .bind(current)
    .execute(&mut *tx)
    .await;
    let written = match written {
        Ok(result) => result,
        Err(error_value) => return sql_error(&request, &error_value),
    };
    if written.rows_affected() == 0 {
        let _ = tx.rollback().await;
        audit_conflict(
            pool,
            id(&request).to_owned(),
            "agent_speaker_policy",
            agent.id,
            expected,
        )
        .await;
        return error(&request, StatusCode::CONFLICT, "revision_conflict");
    }
    let audit_result = audit(
        &mut *tx,
        id(&request),
        "agent_speaker_policy",
        Some(agent.id),
        "put",
        Some(current),
        Some(next),
        AuditOutcome::Success,
        1,
    )
    .await;
    if let Err(error_value) = audit_result {
        return sql_error(&request, &error_value);
    }
    if let Err(error_value) = tx.commit().await {
        return sql_error(&request, &error_value);
    }
    // A policy mode change re-scopes every session on this Agent: drop them so the next voice turn
    // re-admits under the new mode instead of keeping authority from the old snapshot.
    if previous_mode != body.mode
        && let Some(security) = security(&state)
    {
        security.invalidate_agent_speakers(agent.id);
    }
    let qualified = match speaker_calibration::qualification(pool, agent.id, &agent.key).await {
        Ok(qualification) => qualification.is_qualified(),
        Err(error_value) => return sql_error(&request, &error_value),
    };
    with_etag(
        Json(policy_body(&agent.key, &body.mode, next, qualified)).into_response(),
        next,
    )
}

fn binding_body(
    agent_key: &str,
    speaker_key: &str,
    template_keys: &[String],
    revision: i64,
) -> Value {
    serde_json::json!({
        "agent_key": agent_key,
        "speaker_key": speaker_key,
        "template_keys": template_keys,
        "agent_revision": revision,
        "activation": {
            "new_connections": "effective",
            "existing_connections": "reconnect_if_affected",
        },
    })
}

async fn speaker_id_by_key(pool: &SqlitePool, key: &str) -> Result<i64, sqlx::Error> {
    sqlx::query_scalar::<_, i64>("SELECT id FROM speakers WHERE key = ?")
        .bind(key)
        .fetch_one(pool)
        .await
}

/// Resolve one requested Template key to an id only when it is assigned to the Agent and both the
/// Template and its assignment are enabled.  `Ok(None)` means "not a grantable Template".
async fn grantable_template_id(
    pool: &SqlitePool,
    agent_id: i64,
    key: &str,
) -> Result<Option<i64>, sqlx::Error> {
    sqlx::query_scalar::<_, i64>(
        "SELECT t.id FROM agent_templates t \
         JOIN agent_template_assignments a ON a.template_id = t.id \
         WHERE a.agent_id = ? AND a.enabled = 1 AND t.enabled = 1 AND t.key = ?",
    )
    .bind(agent_id)
    .bind(key)
    .fetch_optional(pool)
    .await
}

/// `PUT /agents/{agent_key}/speakers/{speaker_key}` — replace-all explicit Template grants.
pub(super) async fn put_agent_speaker(
    State(state): State<AppState>,
    Path((agent_key, speaker_key)): Path<(String, String)>,
    request: Request,
) -> Response {
    let (request, body): (_, TemplateKeys) = match json(request).await {
        Ok(value) => value,
        Err(response) => return response,
    };
    let mut seen = std::collections::HashSet::new();
    if body.template_keys.is_empty()
        || body.template_keys.len() > MAX_GRANT_TEMPLATES
        || body
            .template_keys
            .iter()
            .any(|key| key == "*" || !valid_key(key) || !seen.insert(key.clone()))
    {
        return error(&request, StatusCode::BAD_REQUEST, "validation_failed");
    }
    let expected = match expected(request.headers()) {
        Ok(value) => value,
        Err(code) => return error(&request, StatusCode::BAD_REQUEST, code),
    };
    let pool = match db(&state) {
        Ok(pool) => pool,
        Err(response) => return response,
    };
    let agent = match get_agent_by(pool, &agent_key).await {
        Ok(agent) => agent,
        Err(error_value) => return agent_lookup_error(&request, &error_value),
    };
    let speaker_id = match speaker_id_by_key(pool, &speaker_key).await {
        Ok(id) => id,
        Err(sqlx::Error::RowNotFound) => {
            return error(&request, StatusCode::NOT_FOUND, "not_found");
        }
        Err(error_value) => return sql_error(&request, &error_value),
    };
    if agent.revision != expected {
        audit_conflict(pool, id(&request).to_owned(), "agent", agent.id, expected).await;
        return error(&request, StatusCode::CONFLICT, "revision_conflict");
    }

    let mut template_ids = Vec::with_capacity(body.template_keys.len());
    for key in &body.template_keys {
        match grantable_template_id(pool, agent.id, key).await {
            Ok(Some(id)) => template_ids.push(id),
            Ok(None) => return error(&request, StatusCode::BAD_REQUEST, "invalid_template"),
            Err(error_value) => return sql_error(&request, &error_value),
        }
    }
    // Old grants, to tell a reduction (which revokes sessions) from an addition (which does not).
    let previous_templates = match sqlx::query_scalar::<_, i64>(
        "SELECT template_id FROM agent_speaker_template_grants WHERE agent_id = ? AND speaker_id = ?",
    )
    .bind(agent.id)
    .bind(speaker_id)
    .fetch_all(pool)
    .await
    {
        Ok(ids) => ids,
        Err(error_value) => return sql_error(&request, &error_value),
    };

    let already_bound: bool = match sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM agent_speaker_candidates WHERE agent_id = ? AND speaker_id = ?",
    )
    .bind(agent.id)
    .bind(speaker_id)
    .fetch_one(pool)
    .await
    {
        Ok(count) => count > 0,
        Err(error_value) => return sql_error(&request, &error_value),
    };
    if !already_bound {
        let bound: i64 = match sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM agent_speaker_candidates WHERE agent_id = ?",
        )
        .bind(agent.id)
        .fetch_one(pool)
        .await
        {
            Ok(count) => count,
            Err(error_value) => return sql_error(&request, &error_value),
        };
        if bound as usize >= state.config.speaker_recognition.max_candidates_per_agent {
            return error(&request, StatusCode::CONFLICT, "speaker_candidate_limit");
        }
    }

    let next = agent.revision + 1;
    let mut tx = match pool.begin().await {
        Ok(tx) => tx,
        Err(error_value) => return sql_error(&request, &error_value),
    };
    let result = async {
        sqlx::query(
            "INSERT INTO agent_speaker_candidates (agent_id, speaker_id, created_at) \
             VALUES (?, ?, ?) ON CONFLICT(agent_id, speaker_id) DO NOTHING",
        )
        .bind(agent.id)
        .bind(speaker_id)
        .bind(now())
        .execute(&mut *tx)
        .await?;
        sqlx::query("DELETE FROM agent_speaker_template_grants WHERE agent_id = ? AND speaker_id = ?")
            .bind(agent.id)
            .bind(speaker_id)
            .execute(&mut *tx)
            .await?;
        for template_id in &template_ids {
            sqlx::query(
                "INSERT INTO agent_speaker_template_grants (agent_id, speaker_id, template_id, created_at) VALUES (?, ?, ?, ?)",
            )
            .bind(agent.id)
            .bind(speaker_id)
            .bind(template_id)
            .bind(now())
            .execute(&mut *tx)
            .await?;
        }
        let bumped = sqlx::query(
            "UPDATE agents SET revision = ?, updated_at = ? WHERE id = ? AND revision = ?",
        )
        .bind(next)
        .bind(now())
        .bind(agent.id)
        .bind(agent.revision)
        .execute(&mut *tx)
        .await?;
        if bumped.rows_affected() == 0 {
            return Err(sqlx::Error::RowNotFound);
        }
        Ok::<_, sqlx::Error>(())
    }
    .await;
    if let Err(error_value) = result {
        let _ = tx.rollback().await;
        if matches!(error_value, sqlx::Error::RowNotFound) {
            audit_conflict(pool, id(&request).to_owned(), "agent", agent.id, expected).await;
            return error(&request, StatusCode::CONFLICT, "revision_conflict");
        }
        return sql_error(&request, &error_value);
    }
    let audit_result = audit(
        &mut *tx,
        id(&request),
        "agent_speaker",
        Some(agent.id),
        "put",
        Some(agent.revision),
        Some(next),
        AuditOutcome::Success,
        template_ids.len() as u64,
    )
    .await;
    if let Err(error_value) = audit_result {
        return sql_error(&request, &error_value);
    }
    if let Err(error_value) = tx.commit().await {
        return sql_error(&request, &error_value);
    }
    // A reduction removes authority an admitted session pinned; an addition only applies to new
    // sessions. Revoke only when a previously granted Template was dropped.
    if previous_templates
        .iter()
        .any(|template| !template_ids.contains(template))
        && let Some(security) = security(&state)
    {
        security.invalidate_speaker(speaker_id);
    }
    with_etag(
        Json(binding_body(
            &agent.key,
            &speaker_key,
            &body.template_keys,
            next,
        ))
        .into_response(),
        next,
    )
}

/// `DELETE /agents/{agent_key}/speakers/{speaker_key}` — unlink and invalidate live sessions.
pub(super) async fn delete_agent_speaker(
    State(state): State<AppState>,
    Path((agent_key, speaker_key)): Path<(String, String)>,
    request: Request,
) -> Response {
    let expected = match expected(request.headers()) {
        Ok(value) => value,
        Err(code) => return error(&request, StatusCode::BAD_REQUEST, code),
    };
    let pool = match db(&state) {
        Ok(pool) => pool,
        Err(response) => return response,
    };
    let agent = match get_agent_by(pool, &agent_key).await {
        Ok(agent) => agent,
        Err(error_value) => return agent_lookup_error(&request, &error_value),
    };
    let speaker_id = match speaker_id_by_key(pool, &speaker_key).await {
        Ok(id) => id,
        Err(sqlx::Error::RowNotFound) => {
            return error(&request, StatusCode::NOT_FOUND, "not_found");
        }
        Err(error_value) => return sql_error(&request, &error_value),
    };
    if agent.revision != expected {
        audit_conflict(pool, id(&request).to_owned(), "agent", agent.id, expected).await;
        return error(&request, StatusCode::CONFLICT, "revision_conflict");
    }

    let next = agent.revision + 1;
    let mut tx = match pool.begin().await {
        Ok(tx) => tx,
        Err(error_value) => return sql_error(&request, &error_value),
    };
    let result = async {
        sqlx::query(
            "DELETE FROM agent_speaker_template_grants WHERE agent_id = ? AND speaker_id = ?",
        )
        .bind(agent.id)
        .bind(speaker_id)
        .execute(&mut *tx)
        .await?;
        let deleted = sqlx::query(
            "DELETE FROM agent_speaker_candidates WHERE agent_id = ? AND speaker_id = ?",
        )
        .bind(agent.id)
        .bind(speaker_id)
        .execute(&mut *tx)
        .await?;
        if deleted.rows_affected() == 0 {
            return Err(sqlx::Error::RowNotFound);
        }
        let bumped = sqlx::query(
            "UPDATE agents SET revision = ?, updated_at = ? WHERE id = ? AND revision = ?",
        )
        .bind(next)
        .bind(now())
        .bind(agent.id)
        .bind(agent.revision)
        .execute(&mut *tx)
        .await?;
        if bumped.rows_affected() == 0 {
            return Err(sqlx::Error::RowNotFound);
        }
        Ok::<_, sqlx::Error>(())
    }
    .await;
    if let Err(error_value) = result {
        let _ = tx.rollback().await;
        if matches!(error_value, sqlx::Error::RowNotFound) {
            return error(&request, StatusCode::NOT_FOUND, "not_found");
        }
        return sql_error(&request, &error_value);
    }
    let audit_result = audit(
        &mut *tx,
        id(&request),
        "agent_speaker",
        Some(agent.id),
        "delete",
        Some(agent.revision),
        Some(next),
        AuditOutcome::Success,
        1,
    )
    .await;
    if let Err(error_value) = audit_result {
        return sql_error(&request, &error_value);
    }
    if let Err(error_value) = tx.commit().await {
        return sql_error(&request, &error_value);
    }
    // Removing a grant changes the authority of already-admitted sessions; drop them so the next
    // voice turn re-admits under the reduced grant set.
    if let Some(security) = security(&state) {
        security.invalidate_agent(agent.id);
        security.invalidate_agent_speakers(agent.id);
    }
    with_etag(
        Json(serde_json::json!({
            "agent_key": agent.key,
            "speaker_key": speaker_key,
            "unlinked": true,
            "agent_revision": next,
        }))
        .into_response(),
        next,
    )
}

#[derive(Deserialize)]
struct TemplateKeys {
    template_keys: Vec<String>,
}

/// `GET /agents/{agent_key}/speakers` — bindings with their grant dependencies.
pub(super) async fn list_agent_speakers(
    State(state): State<AppState>,
    Path(key): Path<String>,
    Query(query): Query<PageQuery>,
    request: Request,
) -> Response {
    let (page, page_size) = match page_bounds(query.page, query.page_size) {
        Ok(bounds) => bounds,
        Err(code) => return error(&request, StatusCode::BAD_REQUEST, code),
    };
    let pool = match db(&state) {
        Ok(pool) => pool,
        Err(response) => return response,
    };
    let agent = match get_agent_by(pool, &key).await {
        Ok(agent) => agent,
        Err(error_value) => return agent_lookup_error(&request, &error_value),
    };
    let rows = match sqlx::query_as::<_, (String, i64, i64)>(
        "SELECT s.key, s.enabled, COUNT(*) OVER() FROM agent_speaker_candidates c \
         JOIN speakers s ON s.id = c.speaker_id WHERE c.agent_id = ? ORDER BY s.key \
         LIMIT ? OFFSET ?",
    )
    .bind(agent.id)
    .bind(page_size as i64)
    .bind(((page - 1) * page_size) as i64)
    .fetch_all(pool)
    .await
    {
        Ok(rows) => rows,
        Err(error_value) => return sql_error(&request, &error_value),
    };
    let total = rows.first().map(|row| row.2).unwrap_or(0);
    let mut items = Vec::with_capacity(rows.len());
    for (speaker_key, enabled, _) in rows {
        let grants = match sqlx::query_as::<_, (String, i64, i64, i64)>(
            "SELECT t.key, t.enabled, a.enabled, g.template_id FROM agent_speaker_template_grants g \
             JOIN agent_templates t ON t.id = g.template_id \
             JOIN agent_speaker_candidates c ON c.agent_id = g.agent_id AND c.speaker_id = g.speaker_id \
             LEFT JOIN agent_template_assignments a ON a.template_id = t.id AND a.agent_id = g.agent_id \
             WHERE g.agent_id = ? AND g.speaker_id = (SELECT id FROM speakers WHERE key = ?) \
             ORDER BY t.key",
        )
        .bind(agent.id)
        .bind(&speaker_key)
        .fetch_all(pool)
        .await
        {
            Ok(grants) => grants,
            Err(error_value) => return sql_error(&request, &error_value),
        };
        let template_keys: Vec<String> = grants.iter().map(|grant| grant.0.clone()).collect();
        let usable = enabled == 1 && grants.iter().all(|grant| grant.1 == 1 && grant.2 == 1);
        items.push(serde_json::json!({
            "speaker_key": speaker_key,
            "template_keys": template_keys,
            "enabled": enabled == 1,
            "usable": usable,
        }));
    }
    with_etag(
        Json(serde_json::json!({
            "items": items,
            "page": page,
            "page_size": page_size,
            "max_page_size": PAGE_MAX,
            "total": total,
            "agent_revision": agent.revision,
        }))
        .into_response(),
        agent.revision,
    )
}

/// `GET /speakers/{speaker_key}/bindings` — Agents and Templates this Speaker is granted to.
pub(super) async fn list_speaker_bindings(
    State(state): State<AppState>,
    Path(key): Path<String>,
    Query(query): Query<PageQuery>,
    request: Request,
) -> Response {
    let (page, page_size) = match page_bounds(query.page, query.page_size) {
        Ok(bounds) => bounds,
        Err(code) => return error(&request, StatusCode::BAD_REQUEST, code),
    };
    let pool = match db(&state) {
        Ok(pool) => pool,
        Err(response) => return response,
    };
    let speaker_id = match speaker_id_by_key(pool, &key).await {
        Ok(id) => id,
        Err(sqlx::Error::RowNotFound) => {
            return error(&request, StatusCode::NOT_FOUND, "not_found");
        }
        Err(error_value) => return sql_error(&request, &error_value),
    };
    let rows = match sqlx::query_as::<_, (String, String, i64)>(
        "SELECT a.key, t.key, COUNT(*) OVER() FROM agent_speaker_template_grants g \
         JOIN agents a ON a.id = g.agent_id \
         JOIN agent_templates t ON t.id = g.template_id \
         WHERE g.speaker_id = ? ORDER BY a.key, t.key LIMIT ? OFFSET ?",
    )
    .bind(speaker_id)
    .bind(page_size as i64)
    .bind(((page - 1) * page_size) as i64)
    .fetch_all(pool)
    .await
    {
        Ok(rows) => rows,
        Err(error_value) => return sql_error(&request, &error_value),
    };
    let total = rows.first().map(|row| row.2).unwrap_or(0);
    let items: Vec<Value> = rows
        .into_iter()
        .map(|(agent_key, template_key, _)| {
            serde_json::json!({ "agent_key": agent_key, "template_key": template_key })
        })
        .collect();
    Json(serde_json::json!({
        "items": items,
        "page": page,
        "page_size": page_size,
        "max_page_size": PAGE_MAX,
        "total": total,
    }))
    .into_response()
}
