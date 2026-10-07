//! Review exact observed contracts using observation and approval revisions.
use super::agents::get_agent_by;
use super::*;
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Review {
    server_key: String,
    original_name: String,
    observed_revision: i64,
    fingerprint: String,
    #[serde(default)]
    allowed: bool,
    #[serde(default)]
    sensitive: bool,
}
pub(super) async fn list(
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
        Err(e) => return sql_error(&request, &e),
    };
    match sqlx::query_as::<_, (String,String,String,String,String,i64,i64,i64,i64,i64,String)>("SELECT m.key,o.original_name,o.description,o.input_schema,o.fingerprint,o.revision,o.observed_at,CASE WHEN a.fingerprint=o.fingerprint THEN COALESCE(a.allowed,0) ELSE 0 END,COALESCE(a.sensitive,0),COALESCE(a.revision,1),json_object('endpoint',m.url,'transport','streamable_http','auth_type',m.auth_type,'auth_header',m.auth_header_name,'auth_reference',m.secret_ref) FROM agent_mcp_bindings b JOIN mcp_servers m ON m.id=b.mcp_server_id JOIN external_tool_observations o ON o.server_id=m.id AND o.server_revision=m.revision AND o.blocked=0 LEFT JOIN agent_external_tool_allowlist a ON a.agent_id=b.agent_id AND a.server_id=m.id AND a.original_name=o.original_name WHERE b.agent_id=? ORDER BY m.key,o.original_name").bind(agent.id).fetch_all(pool).await {
        Ok(rows) => Json(serde_json::json!({"items":rows.into_iter().map(|(server_key,original_name,description,schema,fingerprint,observed_revision,observed_at,allowed,sensitive,revision,source)| serde_json::json!({"server_key":server_key,"original_name":original_name,"description":description,"input_schema":serde_json::from_str::<Value>(&schema).unwrap_or_default(),"fingerprint":fingerprint,"observed_revision":observed_revision,"observed_at":observed_at,"allowed":allowed!=0,"sensitive":sensitive!=0,"revision":revision,"source":serde_json::from_str::<Value>(&source).unwrap_or_default(),"presence":"observed_only"})).collect::<Vec<_>>()})).into_response(),
        Err(e) => sql_error(&request,&e)
    }
}
pub(super) async fn review(
    State(state): State<AppState>,
    Path(key): Path<String>,
    request: Request,
) -> Response {
    let (request, body): (_, Review) = match json(request).await {
        Ok(v) => v,
        Err(e) => return e,
    };
    let expected = match expected(request.headers()) {
        Ok(v) => v,
        Err(code) => return error(&request, StatusCode::BAD_REQUEST, code),
    };
    if !valid_key(&body.server_key)
        || !valid_text(&body.original_name, 256, false)
        || body.fingerprint.len() != 64
        || body.observed_revision < 1
    {
        return error(&request, StatusCode::BAD_REQUEST, "validation_failed");
    }
    let pool = match db(&state) {
        Ok(v) => v,
        Err(e) => return e,
    };
    let agent = match get_agent_by(pool, &key).await {
        Ok(v) => v,
        Err(e) => return sql_error(&request, &e),
    };
    let security = &state.database.as_ref().unwrap().tool_security;
    let _publication = security.publication.write().await;
    let mut tx = match pool.begin().await {
        Ok(v) => v,
        Err(e) => return sql_error(&request, &e),
    };
    let observed=sqlx::query_scalar::<_,i64>("SELECT o.server_id FROM external_tool_observations o JOIN mcp_servers m ON m.id=o.server_id JOIN agent_mcp_bindings b ON b.mcp_server_id=m.id WHERE b.agent_id=? AND m.key=? AND o.original_name=? AND o.revision=? AND o.fingerprint=? AND o.server_revision=m.revision AND o.blocked=0").bind(agent.id).bind(&body.server_key).bind(&body.original_name).bind(body.observed_revision).bind(&body.fingerprint).fetch_optional(&mut *tx).await;
    let server = match observed {
        Ok(Some(v)) => v,
        Ok(None) => return error(&request, StatusCode::CONFLICT, "contract_conflict"),
        Err(e) => return sql_error(&request, &e),
    };
    let old=sqlx::query_as::<_,(i64,i64,i64,String)>("SELECT revision,allowed,sensitive,fingerprint FROM agent_external_tool_allowlist WHERE agent_id=? AND server_id=? AND original_name=?").bind(agent.id).bind(server).bind(&body.original_name).fetch_optional(&mut *tx).await;
    let old = match old {
        Ok(v) => v,
        Err(e) => return sql_error(&request, &e),
    };
    if old.as_ref().map(|v| v.0).unwrap_or(1) != expected {
        return error(&request, StatusCode::CONFLICT, "revision_conflict");
    }
    let result=sqlx::query("INSERT INTO agent_external_tool_allowlist(agent_id,server_id,original_name,fingerprint,allowed,sensitive,revision) VALUES(?,?,?,?,?,?,2) ON CONFLICT(agent_id,server_id,original_name) DO UPDATE SET fingerprint=excluded.fingerprint,allowed=excluded.allowed,sensitive=excluded.sensitive,revision=revision+1")
        .bind(agent.id).bind(server).bind(&body.original_name).bind(&body.fingerprint).bind(i64::from(body.allowed)).bind(i64::from(body.sensitive)).execute(&mut *tx).await;
    if let Err(e) = result {
        return sql_error(&request, &e);
    }
    if let Err(e) = audit(
        &mut *tx,
        id(&request),
        "agent",
        Some(agent.id),
        "review_external_tool",
        Some(expected),
        Some(expected + 1),
        AuditOutcome::Success,
        1,
    )
    .await
    {
        return sql_error(&request, &e);
    }
    if let Err(e) = tx.commit().await {
        return sql_error(&request, &e);
    }
    if old.is_some_and(|(_, allowed, sensitive, fingerprint)| {
        allowed != 0
            && sensitive == 0
            && (!body.allowed || body.sensitive || fingerprint != body.fingerprint)
    }) {
        state
            .database
            .as_ref()
            .unwrap()
            .tool_security
            .invalidate_agent(agent.id);
    }
    Json(serde_json::json!({"revision":expected+1})).into_response()
}
