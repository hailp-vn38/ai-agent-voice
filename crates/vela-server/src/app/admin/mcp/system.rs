use super::super::*;
use crate::app::assistant::trusted_origin;

pub(in crate::app::admin) async fn grants(
    State(state): State<AppState>,
    request: Request,
) -> Response {
    let db = match database(&state) {
        Ok(db) => db,
        Err(e) => return e,
    };
    match db.system_mcp_grants(state.config.system_mcp.enabled).await {
        Ok(settings) => Json(serde_json::json!({
            "enabled": state.config.system_mcp.enabled,
            "grants": settings.grants,
            "revision": settings.revision
        }))
        .into_response(),
        Err(e) => sql_error(&request, &e),
    }
}

pub(in crate::app::admin) async fn save_grants(
    State(state): State<AppState>,
    request: Request,
) -> Response {
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

pub(in crate::app::admin) async fn requests(
    State(state): State<AppState>,
    request: Request,
) -> Response {
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

pub(in crate::app::admin) async fn confirm(
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

pub(in crate::app::admin) async fn reject(
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

pub(in crate::app::admin) async fn revoke_token(
    State(state): State<AppState>,
    request: Request,
) -> Response {
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

fn resolve_base_url(state: &AppState, headers: &HeaderMap, mcp_path: &str) -> String {
    let clean_path = if mcp_path.starts_with('/') {
        mcp_path.to_owned()
    } else {
        format!("/{mcp_path}")
    };
    let proto = headers
        .get("x-forwarded-proto")
        .and_then(|v| v.to_str().ok())
        .map(str::trim)
        .filter(|s| *s == "http" || *s == "https")
        .unwrap_or("http");
    if let Some(host) = headers
        .get("x-forwarded-host")
        .or_else(|| headers.get(header::HOST))
        .and_then(|v| v.to_str().ok())
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        return format!("{proto}://{host}{clean_path}");
    }
    let bind = state.config.server.bind;
    let host_ip = if bind.ip().is_unspecified() {
        "127.0.0.1".to_string()
    } else {
        bind.ip().to_string()
    };
    format!("http://{host_ip}:{}{clean_path}", bind.port())
}

fn attach_base_url(state: &AppState, headers: &HeaderMap, mut snap: Value) -> Value {
    let path = snap
        .get("path")
        .and_then(Value::as_str)
        .unwrap_or("/mcp/knowledge")
        .to_owned();
    let base_url = resolve_base_url(state, headers, &path);
    if let Some(obj) = snap.as_object_mut() {
        obj.insert("base_url".to_string(), Value::String(base_url));
    }
    snap
}

pub(in crate::app::admin) async fn knowledge_settings(
    State(state): State<AppState>,
    request: Request,
) -> Response {
    let Some(core) = state.get_or_init_knowledge_mcp() else {
        return error(
            &request,
            StatusCode::SERVICE_UNAVAILABLE,
            "knowledge_unavailable",
        );
    };
    let snap = attach_base_url(&state, request.headers(), core.admin_snapshot());
    Json(snap).into_response()
}

pub(in crate::app::admin) async fn save_knowledge_settings(
    State(state): State<AppState>,
    request: Request,
) -> Response {
    if trusted_origin(&state, request.headers()).is_none() {
        return error(&request, StatusCode::FORBIDDEN, "untrusted_origin");
    }
    let headers = request.headers().clone();
    let header_revision = if headers.contains_key(header::IF_MATCH) {
        match expected(&headers) {
            Ok(rev) => Some(rev),
            Err(code) => return error(&request, StatusCode::BAD_REQUEST, code),
        }
    } else {
        None
    };
    let (request, input): (_, crate::tools::knowledge_mcp::UpdateKnowledgeMcpInput) =
        match json(request).await {
            Ok(v) => v,
            Err(e) => return e,
        };
    let Some(core) = state.get_or_init_knowledge_mcp() else {
        return error(
            &request,
            StatusCode::SERVICE_UNAVAILABLE,
            "knowledge_unavailable",
        );
    };
    match core.update_settings(input, header_revision) {
        Ok(snap) => Json(attach_base_url(&state, &headers, snap)).into_response(),
        Err(crate::tools::knowledge_mcp::KnowledgeMcpAdminError::RevisionConflict) => {
            error(&request, StatusCode::CONFLICT, "revision_conflict")
        }
        Err(_) => error(&request, StatusCode::BAD_REQUEST, "validation_failed"),
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CreateKnowledgeTokenBody {
    name: String,
    #[serde(default)]
    access: crate::tools::knowledge_mcp::KnowledgeMcpAccess,
    #[serde(default)]
    custom_token: Option<String>,
}

pub(in crate::app::admin) async fn create_knowledge_token(
    State(state): State<AppState>,
    request: Request,
) -> Response {
    if trusted_origin(&state, request.headers()).is_none() {
        return error(&request, StatusCode::FORBIDDEN, "untrusted_origin");
    }
    let headers = request.headers().clone();
    let (request, body): (_, CreateKnowledgeTokenBody) = match json(request).await {
        Ok(v) => v,
        Err(e) => return e,
    };
    let Some(core) = state.get_or_init_knowledge_mcp() else {
        return error(
            &request,
            StatusCode::SERVICE_UNAVAILABLE,
            "knowledge_unavailable",
        );
    };
    match core.create_token(body.name, body.access, body.custom_token) {
        Ok((created, snap)) => {
            let mut full = attach_base_url(&state, &headers, snap);
            if let Some(obj) = full.as_object_mut() {
                obj.insert(
                    "created_token".to_string(),
                    serde_json::to_value(created).unwrap_or(Value::Null),
                );
            }
            (StatusCode::CREATED, Json(full)).into_response()
        }
        Err(_) => error(&request, StatusCode::BAD_REQUEST, "validation_failed"),
    }
}

pub(in crate::app::admin) async fn delete_knowledge_token(
    State(state): State<AppState>,
    Path(id): Path<String>,
    request: Request,
) -> Response {
    if trusted_origin(&state, request.headers()).is_none() {
        return error(&request, StatusCode::FORBIDDEN, "untrusted_origin");
    }
    let Some(core) = state.get_or_init_knowledge_mcp() else {
        return error(
            &request,
            StatusCode::SERVICE_UNAVAILABLE,
            "knowledge_unavailable",
        );
    };
    match core.delete_token(&id) {
        Ok(snap) => Json(attach_base_url(&state, request.headers(), snap)).into_response(),
        Err(_) => error(&request, StatusCode::NOT_FOUND, "not_found"),
    }
}
