use super::*;
use crate::app::web_assistant_auth::trusted_origin;

pub(super) async fn grants(State(state): State<AppState>, request: Request) -> Response {
    let db = match database(&state) {
        Ok(db) => db,
        Err(e) => return e,
    };
    match db.system_mcp_grants(state.config.system_mcp.enabled).await {
        Ok(settings)=>Json(serde_json::json!({"enabled":state.config.system_mcp.enabled,"grants":settings.grants,"revision":settings.revision})).into_response(),
        Err(e)=>sql_error(&request,&e),
    }
}
pub(super) async fn save_grants(State(state): State<AppState>, request: Request) -> Response {
    if trusted_origin(&state, request.headers()).is_none() {
        return error(&request, StatusCode::FORBIDDEN, "untrusted_origin");
    }
    let (request, grants): (_, crate::tools::system_mcp::SystemToolGrants) =
        match json(request).await {
            Ok(v) => v,
            Err(e) => return e,
        };
    let revision = match expected(request.headers()) {
        Ok(v) => v,
        Err(c) => return error(&request, StatusCode::BAD_REQUEST, c),
    };
    let db = match database(&state) {
        Ok(db) => db,
        Err(e) => return e,
    };
    match db
        .save_system_mcp_grants(
            state.config.system_mcp.enabled,
            grants,
            revision,
            id(&request),
        )
        .await
    {
        Ok(_) => self::grants(State(state), request).await,
        Err(e) => write_error(&request, e),
    }
}
pub(super) async fn requests(State(state): State<AppState>, request: Request) -> Response {
    let mut headers = request.headers().clone();
    if !headers.contains_key(header::ORIGIN)
        && let Some(origin) = headers
            .get(header::REFERER)
            .and_then(|h| h.to_str().ok())
            .and_then(|h| url::Url::parse(h).ok())
            .map(|url| url.origin().ascii_serialization())
            .and_then(|origin| HeaderValue::from_str(&origin).ok())
    {
        headers.insert(header::ORIGIN, origin);
    }
    if trusted_origin(&state, &headers).is_none() {
        return error(&request, StatusCode::FORBIDDEN, "untrusted_origin");
    }
    match &state.system_mcp {
        Some(core) => Json(core.change_requests()).into_response(),
        None => error(
            &request,
            StatusCode::SERVICE_UNAVAILABLE,
            "system_tool_unavailable",
        ),
    }
}
pub(super) async fn confirm(
    State(state): State<AppState>,
    Path(key): Path<String>,
    request: Request,
) -> Response {
    if trusted_origin(&state, request.headers()).is_none() {
        return error(&request, StatusCode::FORBIDDEN, "untrusted_origin");
    }
    let Some(core) = &state.system_mcp else {
        return error(
            &request,
            StatusCode::SERVICE_UNAVAILABLE,
            "system_tool_unavailable",
        );
    };
    match core.confirm(&key, id(&request)).await {
        Ok(view) => Json(view).into_response(),
        Err(_) => error(&request, StatusCode::CONFLICT, "proposal_unavailable"),
    }
}
pub(super) async fn reject(
    State(state): State<AppState>,
    Path(key): Path<String>,
    request: Request,
) -> Response {
    if trusted_origin(&state, request.headers()).is_none() {
        return error(&request, StatusCode::FORBIDDEN, "untrusted_origin");
    }
    let Some(core) = &state.system_mcp else {
        return error(
            &request,
            StatusCode::SERVICE_UNAVAILABLE,
            "system_tool_unavailable",
        );
    };
    match core.reject(&key) {
        Ok(view) => Json(view).into_response(),
        Err(_) => error(&request, StatusCode::CONFLICT, "proposal_unavailable"),
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Revoke {
    jti: String,
    expires_at: i64,
}
pub(super) async fn revoke_token(State(state): State<AppState>, request: Request) -> Response {
    if trusted_origin(&state, request.headers()).is_none() {
        return error(&request, StatusCode::FORBIDDEN, "untrusted_origin");
    }
    let (request, input): (_, Revoke) = match json(request).await {
        Ok(v) => v,
        Err(e) => return e,
    };
    let now = crate::database::unix_seconds().unwrap_or(i64::MAX);
    if !valid_identity(&input.jti)
        || input.expires_at <= now
        || input.expires_at > now.saturating_add(3600)
    {
        return error(&request, StatusCode::BAD_REQUEST, "validation_failed");
    }
    let db = match database(&state) {
        Ok(db) => db,
        Err(e) => return e,
    };
    match db
        .revoke_system_mcp_token(&input.jti, input.expires_at, id(&request))
        .await
    {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => write_error(&request, e),
    }
}
