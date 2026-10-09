//! Speaker identification preferences and Agent-scoped candidates.
//! Voice matches personalize a turn; they never grant authority or close a socket.
use super::speakers::with_etag;
use super::*;
use crate::database::agents::get_agent_by;

const POLICY_MODES: [&str; 2] = ["off", "observe"];

fn agent_error(request: &Request, cause: &sqlx::Error) -> Response {
    if matches!(cause, sqlx::Error::RowNotFound) {
        error(request, StatusCode::NOT_FOUND, "not_found")
    } else {
        sql_error(request, cause)
    }
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
    let database = match database(&state) {
        Ok(pool) => pool,
        Err(response) => return response,
    };
    let agent = match get_agent_by(database, &key).await {
        Ok(agent) => agent,
        Err(cause) => return agent_error(&request, &cause),
    };
    let (mode, revision) = match database.policy_state(agent.id).await {
        Ok(value) => value,
        Err(cause) => return sql_error(&request, &cause),
    };
    with_etag(
        Json(policy_body(&agent.key, &mode, revision)).into_response(),
        revision,
    )
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PolicyPut {
    mode: String,
}

/// PUT /agents/{key}/speaker-policy — independent ETag/CAS.
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
    let revision = match expected(request.headers()) {
        Ok(value) => value,
        Err(code) => return error(&request, StatusCode::BAD_REQUEST, code),
    };
    let database = match database(&state) {
        Ok(pool) => pool,
        Err(response) => return response,
    };
    let next = match database
        .put_policy(&key, &body.mode, revision, id(&request))
        .await
    {
        Ok(value) => value,
        Err(cause) => return write_error(&request, cause),
    };
    with_etag(
        Json(policy_body(&key, &body.mode, next)).into_response(),
        next,
    )
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
        Ok(value) => value,
        Err(response) => return response,
    };
    if !body.as_object().is_some_and(|object| object.is_empty()) {
        return error(&request, StatusCode::BAD_REQUEST, "validation_failed");
    }
    let revision = match expected(request.headers()) {
        Ok(value) => value,
        Err(code) => return error(&request, StatusCode::BAD_REQUEST, code),
    };
    let database = match database(&state) {
        Ok(pool) => pool,
        Err(response) => return response,
    };
    let next = match database
        .put_agent_speaker(
            &agent_key,
            &speaker_key,
            revision,
            state.config.speaker_recognition.max_candidates_per_agent,
            id(&request),
        )
        .await
    {
        Ok(value) => value,
        Err(cause) => return write_error(&request, cause),
    };
    with_etag(
        Json(binding_body(&agent_key, &speaker_key, next)).into_response(),
        next,
    )
}

/// DELETE /agents/{key}/speakers/{speaker_key}.
pub(super) async fn delete_agent_speaker(
    State(state): State<AppState>,
    Path((agent_key, speaker_key)): Path<(String, String)>,
    request: Request,
) -> Response {
    let revision = match expected(request.headers()) {
        Ok(value) => value,
        Err(code) => return error(&request, StatusCode::BAD_REQUEST, code),
    };
    let database = match database(&state) {
        Ok(pool) => pool,
        Err(response) => return response,
    };
    let next = match database
        .delete_agent_speaker(&agent_key, &speaker_key, revision, id(&request))
        .await
    {
        Ok(value) => value,
        Err(cause) => return write_error(&request, cause),
    };
    with_etag(
        Json(serde_json::json!({
            "agent_key": agent_key, "speaker_key": speaker_key,
            "unlinked": true, "agent_revision": next
        }))
        .into_response(),
        next,
    )
}

/// GET /agents/{key}/speakers — direct Agent candidates, independent of Template.
pub(super) async fn list_agent_speakers(
    State(state): State<AppState>,
    Path(key): Path<String>,
    Query(query): Query<PageQuery>,
    request: Request,
) -> Response {
    let (page, page_size) = match page_bounds(query.page, query.page_size) {
        Ok(value) => value,
        Err(code) => return error(&request, StatusCode::BAD_REQUEST, code),
    };
    let database = match database(&state) {
        Ok(pool) => pool,
        Err(response) => return response,
    };
    let agent = match get_agent_by(database, &key).await {
        Ok(value) => value,
        Err(cause) => return agent_error(&request, &cause),
    };
    let space = state
        .speaker_runtime
        .as_ref()
        .map(|runtime| runtime.embedding_space_id().to_owned());
    let rows = database
        .agent_speaker_candidates(agent.id, space, page, page_size)
        .await;
    let rows = match rows {
        Ok(value) => value,
        Err(cause) => return sql_error(&request, &cause),
    };
    let total = rows.first().map(|row| row.3).unwrap_or(0);
    let items: Vec<Value> = rows.into_iter().map(|(key, enabled, matched, _)| {
        serde_json::json!({"speaker_key":key,"enabled":enabled != 0,"usable":enabled != 0 && matched != 0})
    }).collect();
    with_etag(
        Json(serde_json::json!({
            "items": items, "page": page, "page_size": page_size,
            "max_page_size": PAGE_MAX, "total": total, "agent_revision": agent.revision
        }))
        .into_response(),
        agent.revision,
    )
}

/// GET /speakers/{key}/bindings — list Agents only.
pub(super) async fn list_speaker_bindings(
    State(state): State<AppState>,
    Path(key): Path<String>,
    Query(query): Query<PageQuery>,
    request: Request,
) -> Response {
    let (page, page_size) = match page_bounds(query.page, query.page_size) {
        Ok(value) => value,
        Err(code) => return error(&request, StatusCode::BAD_REQUEST, code),
    };
    let database = match database(&state) {
        Ok(pool) => pool,
        Err(response) => return response,
    };
    let speaker = match database.speaker_id(&key).await {
        Ok(value) => value,
        Err(cause) => return agent_error(&request, &cause),
    };
    let rows = database.speaker_bindings(speaker, page, page_size).await;
    let rows = match rows {
        Ok(value) => value,
        Err(cause) => return sql_error(&request, &cause),
    };
    let total = rows.first().map(|row| row.1).unwrap_or(0);
    Json(serde_json::json!({
        "items": rows.into_iter().map(|(agent_key, _)| serde_json::json!({"agent_key": agent_key})).collect::<Vec<_>>(),
        "page": page, "page_size": page_size, "max_page_size": PAGE_MAX, "total": total
    })).into_response()
}
