//! Review exact observed contracts using observation and approval revisions.
use super::*;
use crate::database::agents::get_agent_by;
use crate::database::tool_allowlist::ToolReview;
pub(super) async fn list(
    State(state): State<AppState>,
    Path(key): Path<String>,
    request: Request,
) -> Response {
    let database = match database(&state) {
        Ok(v) => v,
        Err(e) => return e,
    };
    let agent = match get_agent_by(database, &key).await {
        Ok(v) => v,
        Err(e) => return sql_error(&request, &e),
    };
    match database.external_tool_reviews(agent.id).await {
        Ok(rows) => Json(serde_json::json!({"items":rows.into_iter().map(|(server_key,original_name,description,schema,fingerprint,observed_revision,observed_at,allowed,sensitive,revision,source)| serde_json::json!({"server_key":server_key,"original_name":original_name,"description":description,"input_schema":serde_json::from_str::<Value>(&schema).unwrap_or_default(),"fingerprint":fingerprint,"observed_revision":observed_revision,"observed_at":observed_at,"allowed":allowed!=0,"sensitive":sensitive!=0,"revision":revision,"source":serde_json::from_str::<Value>(&source).unwrap_or_default(),"presence":"observed_only"})).collect::<Vec<_>>()})).into_response(),
        Err(e) => sql_error(&request,&e)
    }
}
pub(super) async fn review(
    State(state): State<AppState>,
    Path(key): Path<String>,
    request: Request,
) -> Response {
    let (request, body): (_, ToolReview) = match json(request).await {
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
    let database = match database(&state) {
        Ok(v) => v,
        Err(e) => return e,
    };
    match database
        .review_external_tool(&key, body, expected, id(&request))
        .await
    {
        Ok(revision) => Json(serde_json::json!({"revision":revision})).into_response(),
        Err(cause) => write_error(&request, cause),
    }
}
