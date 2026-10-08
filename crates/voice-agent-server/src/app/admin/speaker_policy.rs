//! Speaker identification preferences and Agent-scoped candidates.
//! Voice matches personalize a turn; they never grant authority or close a socket.
use super::agents::get_agent_by;
use super::speakers::with_etag;
use super::*;

const POLICY_MODES: [&str; 2] = ["off", "observe"];

fn agent_error(request: &Request, cause: &sqlx::Error) -> Response {
    if matches!(cause, sqlx::Error::RowNotFound) {
        error(request, StatusCode::NOT_FOUND, "not_found")
    } else {
        sql_error(request, cause)
    }
}

async fn policy_state(pool: &SqlitePool, agent_id: i64) -> Result<(String, i64), sqlx::Error> {
    Ok(sqlx::query_as::<_, (String, i64)>(
        "SELECT mode,revision FROM agent_speaker_policies WHERE agent_id=?",
    )
    .bind(agent_id)
    .fetch_optional(pool)
    .await?
    .unwrap_or_else(|| ("off".to_owned(), 1)))
}

fn policy_body(agent_key: &str, mode: &str, revision: i64) -> Value {
    serde_json::json!({
        "agent_key": agent_key,
        "mode": mode,
        "revision": revision,
        "speaker_change": "reconnect"
    })
}

/// GET /agents/{key}/speaker-policy
pub(super) async fn get_policy(
    State(state): State<AppState>,
    Path(key): Path<String>,
    request: Request,
) -> Response {
    let pool = match db(&state) { Ok(pool) => pool, Err(response) => return response };
    let agent = match get_agent_by(pool, &key).await {
        Ok(agent) => agent,
        Err(cause) => return agent_error(&request, &cause),
    };
    let (mode, revision) = match policy_state(pool, agent.id).await {
        Ok(value) => value,
        Err(cause) => return sql_error(&request, &cause),
    };
    with_etag(Json(policy_body(&agent.key, &mode, revision)).into_response(), revision)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PolicyPut { mode: String }

/// PUT /agents/{key}/speaker-policy — independent ETag/CAS.
pub(super) async fn put_policy(
    State(state): State<AppState>,
    Path(key): Path<String>,
    request: Request,
) -> Response {
    let (request, body): (_, PolicyPut) = match json(request).await {
        Ok(value) => value, Err(response) => return response,
    };
    if !POLICY_MODES.contains(&body.mode.as_str()) {
        return error(&request, StatusCode::BAD_REQUEST, "validation_failed");
    }
    let revision = match expected(request.headers()) {
        Ok(value) => value, Err(code) => return error(&request, StatusCode::BAD_REQUEST, code),
    };
    let pool = match db(&state) { Ok(pool) => pool, Err(response) => return response };
    let agent = match get_agent_by(pool, &key).await {
        Ok(agent) => agent,
        Err(cause) => return agent_error(&request, &cause),
    };
    let (_, current) = match policy_state(pool, agent.id).await {
        Ok(value) => value, Err(cause) => return sql_error(&request, &cause),
    };
    if revision != current {
        audit_conflict(pool, id(&request).to_owned(), "agent_speaker_policy", agent.id, revision).await;
        return error(&request, StatusCode::CONFLICT, "revision_conflict");
    }
    let next = current + 1;
    let mut tx = match pool.begin().await {
        Ok(tx) => tx, Err(cause) => return sql_error(&request, &cause),
    };
    let result = sqlx::query(
        "INSERT INTO agent_speaker_policies (agent_id,mode,revision) VALUES (?,?,?) \
         ON CONFLICT(agent_id) DO UPDATE SET mode=excluded.mode,revision=excluded.revision \
         WHERE agent_speaker_policies.revision=?",
    )
    .bind(agent.id).bind(&body.mode).bind(next).bind(current)
    .execute(&mut *tx).await;
    match result {
        Ok(result) if result.rows_affected() == 1 => {}
        Ok(_) => return error(&request, StatusCode::CONFLICT, "revision_conflict"),
        Err(cause) => return sql_error(&request, &cause),
    }
    if let Err(cause) = audit(&mut *tx, id(&request), "agent_speaker_policy", Some(agent.id),
        "put", Some(current), Some(next), AuditOutcome::Success, 1).await {
        return sql_error(&request, &cause);
    }
    if let Err(cause) = tx.commit().await { return sql_error(&request, &cause); }
    // Identification preferences take effect on the next connection.
    with_etag(Json(policy_body(&agent.key, &body.mode, next)).into_response(), next)
}

async fn speaker_id(pool: &SqlitePool, key: &str) -> Result<i64, sqlx::Error> {
    sqlx::query_scalar("SELECT id FROM speakers WHERE key=?")
        .bind(key).fetch_one(pool).await
}

fn binding_body(agent: &str, speaker: &str, revision: i64) -> Value {
    serde_json::json!({
        "agent_key": agent,
        "speaker_key": speaker,
        "agent_revision": revision,
        "activation": {"new_connections": "effective", "existing_connections": "reconnect"}
    })
}

/// PUT /agents/{key}/speakers/{speaker_key} — body is {}, no Template grants.
pub(super) async fn put_agent_speaker(
    State(state): State<AppState>,
    Path((agent_key, speaker_key)): Path<(String, String)>,
    request: Request,
) -> Response {
    let (request, body): (_, Value) = match json(request).await {
        Ok(value) => value, Err(response) => return response,
    };
    if !body.as_object().is_some_and(|object| object.is_empty()) {
        return error(&request, StatusCode::BAD_REQUEST, "validation_failed");
    }
    let revision = match expected(request.headers()) {
        Ok(value) => value, Err(code) => return error(&request, StatusCode::BAD_REQUEST, code),
    };
    let pool = match db(&state) { Ok(pool) => pool, Err(response) => return response };
    let agent = match get_agent_by(pool, &agent_key).await {
        Ok(agent) => agent, Err(cause) => return agent_error(&request, &cause),
    };
    let speaker = match speaker_id(pool, &speaker_key).await {
        Ok(value) => value, Err(cause) => return agent_error(&request, &cause),
    };
    if agent.revision != revision {
        return error(&request, StatusCode::CONFLICT, "revision_conflict");
    }
    let current: i64 = match sqlx::query_scalar(
        "SELECT COUNT(*) FROM agent_speaker_candidates WHERE agent_id=?",
    ).bind(agent.id).fetch_one(pool).await {
        Ok(value) => value, Err(cause) => return sql_error(&request, &cause),
    };
    let exists: i64 = match sqlx::query_scalar(
        "SELECT COUNT(*) FROM agent_speaker_candidates WHERE agent_id=? AND speaker_id=?",
    ).bind(agent.id).bind(speaker).fetch_one(pool).await {
        Ok(value) => value, Err(cause) => return sql_error(&request, &cause),
    };
    if exists == 0 && current >= state.config.speaker_recognition.max_candidates_per_agent as i64 {
        return error(&request, StatusCode::CONFLICT, "speaker_candidate_limit");
    }
    let next = agent.revision + 1;
    let mut tx = match pool.begin().await {
        Ok(tx) => tx, Err(cause) => return sql_error(&request, &cause),
    };
    let inserted = sqlx::query(
        "INSERT INTO agent_speaker_candidates(agent_id,speaker_id,created_at) VALUES (?,?,?) \
         ON CONFLICT(agent_id,speaker_id) DO NOTHING",
    )
    .bind(agent.id).bind(speaker).bind(now()).execute(&mut *tx).await;
    if let Err(cause) = inserted { return sql_error(&request, &cause); }
    let bumped = sqlx::query(
        "UPDATE agents SET revision=?,updated_at=? WHERE id=? AND revision=?",
    )
    .bind(next).bind(now()).bind(agent.id).bind(revision).execute(&mut *tx).await;
    if !matches!(bumped, Ok(ref value) if value.rows_affected() == 1) {
        return error(&request, StatusCode::CONFLICT, "revision_conflict");
    }
    if let Err(cause) = audit(&mut *tx, id(&request), "agent_speaker", Some(agent.id),
        "put", Some(revision), Some(next), AuditOutcome::Success, 1).await {
        return sql_error(&request, &cause);
    }
    if let Err(cause) = tx.commit().await { return sql_error(&request, &cause); }
    with_etag(Json(binding_body(&agent.key, &speaker_key, next)).into_response(), next)
}

/// DELETE /agents/{key}/speakers/{speaker_key}.
pub(super) async fn delete_agent_speaker(
    State(state): State<AppState>,
    Path((agent_key, speaker_key)): Path<(String, String)>,
    request: Request,
) -> Response {
    let revision = match expected(request.headers()) {
        Ok(value) => value, Err(code) => return error(&request, StatusCode::BAD_REQUEST, code),
    };
    let pool = match db(&state) { Ok(pool) => pool, Err(response) => return response };
    let agent = match get_agent_by(pool, &agent_key).await {
        Ok(value) => value, Err(cause) => return agent_error(&request, &cause),
    };
    let speaker = match speaker_id(pool, &speaker_key).await {
        Ok(value) => value, Err(cause) => return agent_error(&request, &cause),
    };
    if revision != agent.revision {
        return error(&request, StatusCode::CONFLICT, "revision_conflict");
    }
    let mut tx = match pool.begin().await {
        Ok(value) => value, Err(cause) => return sql_error(&request, &cause),
    };
    let deleted = sqlx::query(
        "DELETE FROM agent_speaker_candidates WHERE agent_id=? AND speaker_id=?",
    ).bind(agent.id).bind(speaker).execute(&mut *tx).await;
    if !matches!(deleted, Ok(ref value) if value.rows_affected() == 1) {
        return error(&request, StatusCode::NOT_FOUND, "not_found");
    }
    let next = revision + 1;
    let bumped = sqlx::query(
        "UPDATE agents SET revision=?,updated_at=? WHERE id=? AND revision=?",
    )
    .bind(next).bind(now()).bind(agent.id).bind(revision).execute(&mut *tx).await;
    if !matches!(bumped, Ok(ref value) if value.rows_affected() == 1) {
        return error(&request, StatusCode::CONFLICT, "revision_conflict");
    }
    if let Err(cause) = audit(&mut *tx, id(&request), "agent_speaker", Some(agent.id),
        "delete", Some(revision), Some(next), AuditOutcome::Success, 1).await {
        return sql_error(&request, &cause);
    }
    if let Err(cause) = tx.commit().await { return sql_error(&request, &cause); }
    with_etag(Json(serde_json::json!({
        "agent_key": agent.key, "speaker_key": speaker_key,
        "unlinked": true, "agent_revision": next
    })).into_response(), next)
}

/// GET /agents/{key}/speakers — direct Agent candidates, independent of Template.
pub(super) async fn list_agent_speakers(
    State(state): State<AppState>,
    Path(key): Path<String>,
    Query(query): Query<PageQuery>,
    request: Request,
) -> Response {
    let (page, page_size) = match page_bounds(query.page, query.page_size) {
        Ok(value) => value, Err(code) => return error(&request, StatusCode::BAD_REQUEST, code),
    };
    let pool = match db(&state) { Ok(pool) => pool, Err(response) => return response };
    let agent = match get_agent_by(pool, &key).await {
        Ok(value) => value, Err(cause) => return agent_error(&request, &cause),
    };
    let space = state.speaker_runtime.as_ref().map(|runtime| runtime.embedding_space_id().to_owned());
    let rows = sqlx::query_as::<_, (String, i64, i64, i64)>(
        "SELECT s.key,s.enabled,EXISTS(SELECT 1 FROM speaker_voiceprints v \
            WHERE v.speaker_id=s.id AND v.embedding_space=?),COUNT(*) OVER() \
         FROM agent_speaker_candidates c JOIN speakers s ON s.id=c.speaker_id \
         WHERE c.agent_id=? ORDER BY s.key LIMIT ? OFFSET ?",
    ).bind(space).bind(agent.id).bind(page_size as i64)
        .bind(((page - 1) * page_size) as i64).fetch_all(pool).await;
    let rows = match rows { Ok(value) => value, Err(cause) => return sql_error(&request, &cause) };
    let total = rows.first().map(|row| row.3).unwrap_or(0);
    let items: Vec<Value> = rows.into_iter().map(|(key, enabled, matched, _)| {
        serde_json::json!({"speaker_key":key,"enabled":enabled != 0,"usable":enabled != 0 && matched != 0})
    }).collect();
    with_etag(Json(serde_json::json!({
        "items": items, "page": page, "page_size": page_size,
        "max_page_size": PAGE_MAX, "total": total, "agent_revision": agent.revision
    })).into_response(), agent.revision)
}

/// GET /speakers/{key}/bindings — list Agents only.
pub(super) async fn list_speaker_bindings(
    State(state): State<AppState>,
    Path(key): Path<String>,
    Query(query): Query<PageQuery>,
    request: Request,
) -> Response {
    let (page, page_size) = match page_bounds(query.page, query.page_size) {
        Ok(value) => value, Err(code) => return error(&request, StatusCode::BAD_REQUEST, code),
    };
    let pool = match db(&state) { Ok(pool) => pool, Err(response) => return response };
    let speaker = match speaker_id(pool, &key).await {
        Ok(value) => value, Err(cause) => return agent_error(&request, &cause),
    };
    let rows = sqlx::query_as::<_, (String, i64)>(
        "SELECT a.key,COUNT(*) OVER() FROM agent_speaker_candidates c \
         JOIN agents a ON a.id=c.agent_id WHERE c.speaker_id=? \
         ORDER BY a.key LIMIT ? OFFSET ?",
    ).bind(speaker).bind(page_size as i64)
        .bind(((page - 1) * page_size) as i64).fetch_all(pool).await;
    let rows = match rows { Ok(value) => value, Err(cause) => return sql_error(&request, &cause) };
    let total = rows.first().map(|row| row.1).unwrap_or(0);
    Json(serde_json::json!({
        "items": rows.into_iter().map(|(agent_key, _)| serde_json::json!({"agent_key": agent_key})).collect::<Vec<_>>(),
        "page": page, "page_size": page_size, "max_page_size": PAGE_MAX, "total": total
    })).into_response()
}
