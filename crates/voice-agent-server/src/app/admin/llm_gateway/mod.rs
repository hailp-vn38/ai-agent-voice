//! Admin HTTP endpoints for the Internal LLM Gateway (`/api/admin/llm-gateway/*`).

#[cfg(test)]
mod tests;

use std::sync::{
    Mutex as StdMutex, OnceLock,
    atomic::{AtomicBool, Ordering},
};

use super::*;
use crate::services::llm_gateway::{
    CreatePoolModelRequest, GatewayError, GatewayVendor, OAuthCallbackSubmission, OAuthPollStatus,
    StartOAuthLoginRequest, UpdateGatewayAccountPatch, UpdatePoolModelPatch,
};

#[derive(Debug, Default, Deserialize)]
pub(super) struct ListAccountsQuery {
    #[serde(default)]
    vendor: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
pub(super) struct ListQuotasQuery {
    #[serde(default)]
    vendor: Option<String>,
    #[serde(default)]
    force: Option<bool>,
}

#[derive(Debug, Default, Deserialize)]
pub(super) struct GetQuotaQuery {
    #[serde(default)]
    force: Option<bool>,
}

#[derive(Debug, Default, Deserialize)]
pub(super) struct OAuthCallbackQuery {
    #[serde(default)]
    code: Option<String>,
    #[serde(default)]
    state: Option<String>,
    #[serde(default)]
    error: Option<String>,
    #[serde(default)]
    error_description: Option<String>,
}

pub(super) async fn list_vendors(State(state): State<AppState>) -> Response {
    let vendors = state.llm_gateway_hub.list_vendors_detailed().await;
    Json(serde_json::json!({ "vendors": vendors })).into_response()
}

pub(super) async fn get_pool_detail(
    State(state): State<AppState>,
    Path(vendor_raw): Path<String>,
    request: Request,
) -> Response {
    let Some(vendor) = GatewayVendor::parse(&vendor_raw) else {
        return error(&request, StatusCode::BAD_REQUEST, "invalid_vendor");
    };
    match state.llm_gateway_hub.get_pool_detail(vendor).await {
        Ok(pool) => Json(serde_json::json!({ "pool": pool })).into_response(),
        Err(err) => gateway_error(&request, err),
    }
}

pub(super) async fn create_pool_model(
    State(state): State<AppState>,
    Path(vendor_raw): Path<String>,
    request: Request,
) -> Response {
    let Some(vendor) = GatewayVendor::parse(&vendor_raw) else {
        return error(&request, StatusCode::BAD_REQUEST, "invalid_vendor");
    };
    let (request, body): (_, CreatePoolModelRequest) = match json(request).await {
        Ok(v) => v,
        Err(e) => return e,
    };
    match state
        .llm_gateway_hub
        .create_pool_model(vendor, body, id(&request))
        .await
    {
        Ok(model) => {
            let _ = crate::services::task_flow::sync_orchestrator_from_state(&state).await;
            (
                StatusCode::CREATED,
                Json(serde_json::json!({ "model": model })),
            )
                .into_response()
        }
        Err(err) => gateway_error(&request, err),
    }
}

pub(super) async fn patch_pool_model(
    State(state): State<AppState>,
    Path((vendor_raw, provider_key)): Path<(String, String)>,
    request: Request,
) -> Response {
    let Some(vendor) = GatewayVendor::parse(&vendor_raw) else {
        return error(&request, StatusCode::BAD_REQUEST, "invalid_vendor");
    };
    let (request, body): (_, UpdatePoolModelPatch) = match json(request).await {
        Ok(v) => v,
        Err(e) => return e,
    };
    match state
        .llm_gateway_hub
        .update_pool_model(vendor, &provider_key, body, id(&request))
        .await
    {
        Ok(model) => {
            if let Some(prewarm) = &state.provider_prewarm {
                prewarm.provider(model.id, model.revision).await;
            }
            let _ = crate::services::task_flow::sync_orchestrator_from_state(&state).await;
            Json(serde_json::json!({ "model": model })).into_response()
        }
        Err(err) => gateway_error(&request, err),
    }
}

pub(super) async fn delete_pool_model(
    State(state): State<AppState>,
    Path((vendor_raw, provider_key)): Path<(String, String)>,
    request: Request,
) -> Response {
    let Some(vendor) = GatewayVendor::parse(&vendor_raw) else {
        return error(&request, StatusCode::BAD_REQUEST, "invalid_vendor");
    };
    match state
        .llm_gateway_hub
        .delete_pool_model(vendor, &provider_key, id(&request))
        .await
    {
        Ok(deleted) => {
            let _ = crate::services::task_flow::sync_orchestrator_from_state(&state).await;
            Json(serde_json::json!({ "deleted": deleted })).into_response()
        }
        Err(err) => gateway_error(&request, err),
    }
}

pub(super) async fn list_accounts(
    State(state): State<AppState>,
    Query(query): Query<ListAccountsQuery>,
    request: Request,
) -> Response {
    let vendor_filter = match query
        .vendor
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        Some(raw) => match GatewayVendor::parse(raw) {
            Some(vendor) => Some(vendor),
            None => return error(&request, StatusCode::BAD_REQUEST, "invalid_vendor"),
        },
        None => None,
    };

    match state.llm_gateway_hub.list_accounts(vendor_filter).await {
        Ok(accounts) => Json(serde_json::json!({ "accounts": accounts })).into_response(),
        Err(err) => gateway_error(&request, err),
    }
}

pub(super) async fn list_quotas(
    State(state): State<AppState>,
    Query(query): Query<ListQuotasQuery>,
    request: Request,
) -> Response {
    let vendor_filter = match query
        .vendor
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        Some(raw) => match GatewayVendor::parse(raw) {
            Some(vendor) => Some(vendor),
            None => return error(&request, StatusCode::BAD_REQUEST, "invalid_vendor"),
        },
        None => None,
    };
    let force = query.force.unwrap_or(false);

    match state
        .llm_gateway_hub
        .list_quotas(vendor_filter, force)
        .await
    {
        Ok(quotas) => Json(serde_json::json!({ "quotas": quotas })).into_response(),
        Err(err) => gateway_error(&request, err),
    }
}

pub(super) async fn get_account_quota(
    State(state): State<AppState>,
    Path(account_key): Path<String>,
    Query(query): Query<GetQuotaQuery>,
    request: Request,
) -> Response {
    let force = query.force.unwrap_or(false);
    match state
        .llm_gateway_hub
        .get_account_quota(&account_key, force)
        .await
    {
        Ok(quota) => Json(serde_json::json!({ "quota": quota })).into_response(),
        Err(err) => gateway_error(&request, err),
    }
}

pub(super) async fn refresh_account_quota(
    State(state): State<AppState>,
    Path(account_key): Path<String>,
    request: Request,
) -> Response {
    match state
        .llm_gateway_hub
        .get_account_quota(&account_key, true)
        .await
    {
        Ok(quota) => Json(serde_json::json!({ "quota": quota })).into_response(),
        Err(err) => gateway_error(&request, err),
    }
}

pub(super) async fn start_oauth(State(state): State<AppState>, request: Request) -> Response {
    let (request, body): (_, StartOAuthLoginRequest) = match json(request).await {
        Ok(v) => v,
        Err(e) => return e,
    };

    match state.llm_gateway_hub.start_oauth_login(body).await {
        Ok(challenge) => {
            if challenge.vendor == GatewayVendor::OpenaiCodex {
                ensure_codex_loopback_listener(state.clone());
            }
            Json(challenge).into_response()
        }
        Err(err) => gateway_error(&request, err),
    }
}

pub(super) async fn oauth_callback(
    State(state): State<AppState>,
    Query(query): Query<OAuthCallbackQuery>,
) -> Response {
    if let Some(err_code) = query.error.as_deref().filter(|s| !s.trim().is_empty()) {
        let desc = query
            .error_description
            .as_deref()
            .unwrap_or("Authorization was denied or failed.");
        if let Some(state_param) = query
            .state
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            state
                .llm_gateway_hub
                .record_oauth_failure_by_state(state_param, format!("{err_code}: {desc}"));
        }
        return callback_html_response(
            StatusCode::BAD_REQUEST,
            &serde_json::json!({
                "type": "llm_gateway_oauth_complete",
                "status": "failed",
                "error": err_code,
                "message": desc,
            }),
            false,
            desc,
        );
    }

    let Some(code) = query
        .code
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    else {
        return callback_html_response(
            StatusCode::BAD_REQUEST,
            &serde_json::json!({
                "type": "llm_gateway_oauth_complete",
                "status": "failed",
                "error": "oauth_callback_invalid",
            }),
            false,
            "Missing authorization code in callback query.",
        );
    };

    let Some(state_param) = query
        .state
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    else {
        return callback_html_response(
            StatusCode::BAD_REQUEST,
            &serde_json::json!({
                "type": "llm_gateway_oauth_complete",
                "status": "failed",
                "error": "oauth_callback_invalid",
            }),
            false,
            "Missing OAuth state in callback query.",
        );
    };

    match state
        .llm_gateway_hub
        .complete_oauth_by_state(state_param, code)
        .await
    {
        Ok((session_id, account)) => {
            let _ = crate::services::task_flow::sync_orchestrator_from_state(&state).await;
            callback_html_response(
                StatusCode::OK,
                &serde_json::json!({
                    "type": "llm_gateway_oauth_complete",
                    "status": "completed",
                    "session_id": session_id,
                    "account": account,
                }),
                true,
                "OAuth login completed successfully. You may close this window.",
            )
        }
        Err(err) => {
            let msg = err.to_string();
            callback_html_response(
                StatusCode::BAD_REQUEST,
                &serde_json::json!({
                    "type": "llm_gateway_oauth_complete",
                    "status": "failed",
                    "error": err.code(),
                    "message": msg,
                }),
                false,
                &msg,
            )
        }
    }
}

pub(super) async fn complete_oauth_session(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    request: Request,
) -> Response {
    let (request, body): (_, OAuthCallbackSubmission) = match json(request).await {
        Ok(v) => v,
        Err(e) => return e,
    };

    match state
        .llm_gateway_hub
        .complete_oauth_login(&session_id, body)
        .await
    {
        Ok(account) => {
            let _ = crate::services::task_flow::sync_orchestrator_from_state(&state).await;
            Json(serde_json::json!({ "account": account })).into_response()
        }
        Err(err) => gateway_error(&request, err),
    }
}

pub(super) async fn poll_oauth_session(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    request: Request,
) -> Response {
    match state.llm_gateway_hub.poll_oauth_login(&session_id).await {
        Ok(status) => {
            if matches!(status, OAuthPollStatus::Completed { .. }) {
                let _ = crate::services::task_flow::sync_orchestrator_from_state(&state).await;
            }
            Json(status).into_response()
        }
        Err(err) => gateway_error(&request, err),
    }
}

pub(super) async fn refresh_account(
    State(state): State<AppState>,
    Path(account_key): Path<String>,
    request: Request,
) -> Response {
    match state
        .llm_gateway_hub
        .refresh_account_now(&account_key)
        .await
    {
        Ok(account) => Json(serde_json::json!({ "account": account })).into_response(),
        Err(err) => gateway_error(&request, err),
    }
}

pub(super) async fn clear_account_cooldown(
    State(state): State<AppState>,
    Path(account_key): Path<String>,
    request: Request,
) -> Response {
    match state.llm_gateway_hub.clear_cooldown(&account_key).await {
        Ok(account) => Json(serde_json::json!({ "account": account })).into_response(),
        Err(err) => gateway_error(&request, err),
    }
}

pub(super) async fn patch_account(
    State(state): State<AppState>,
    Path(account_key): Path<String>,
    request: Request,
) -> Response {
    let (request, body): (_, UpdateGatewayAccountPatch) = match json(request).await {
        Ok(v) => v,
        Err(e) => return e,
    };

    match state
        .llm_gateway_hub
        .update_account(&account_key, body)
        .await
    {
        Ok(account) => {
            let _ = crate::services::task_flow::sync_orchestrator_from_state(&state).await;
            Json(serde_json::json!({ "account": account })).into_response()
        }
        Err(err) => gateway_error(&request, err),
    }
}

pub(super) async fn delete_account(
    State(state): State<AppState>,
    Path(account_key): Path<String>,
    request: Request,
) -> Response {
    match state.llm_gateway_hub.delete_account(&account_key).await {
        Ok(()) => {
            let _ = crate::services::task_flow::sync_orchestrator_from_state(&state).await;
            Json(serde_json::json!({ "deleted": true })).into_response()
        }
        Err(err) => gateway_error(&request, err),
    }
}

fn gateway_error(request: &Request, err: GatewayError) -> Response {
    let code = err.code();
    match err {
        GatewayError::InvalidBinding
        | GatewayError::InvalidTokenBundle
        | GatewayError::OAuthStateMismatch
        | GatewayError::OAuthCallbackInvalid
        | GatewayError::CredentialInvalid
        | GatewayError::ValidationFailed => error(request, StatusCode::BAD_REQUEST, code),
        GatewayError::OAuthSessionNotFound
        | GatewayError::AccountNotFound
        | GatewayError::PoolModelNotFound => error(request, StatusCode::NOT_FOUND, code),
        GatewayError::OAuthSessionExpired => error(request, StatusCode::GONE, code),
        GatewayError::AccountInUse(providers) => (
            StatusCode::CONFLICT,
            Json(serde_json::json!({
                "error": {
                    "code": code,
                    "providers": providers,
                    "request_id": id(request),
                }
            })),
        )
            .into_response(),
        GatewayError::PoolModelInUse(templates) => (
            StatusCode::CONFLICT,
            Json(serde_json::json!({
                "error": {
                    "code": code,
                    "templates": templates,
                    "request_id": id(request),
                }
            })),
        )
            .into_response(),
        GatewayError::AccountCoolingDown => error(request, StatusCode::TOO_MANY_REQUESTS, code),
        GatewayError::AccountDisabled | GatewayError::AccountReauthRequired => {
            error(request, StatusCode::CONFLICT, code)
        }
        GatewayError::OAuthExchangeFailed(detail) => (
            StatusCode::BAD_GATEWAY,
            Json(serde_json::json!({
                "error": {
                    "code": code,
                    "message": detail,
                    "request_id": id(request),
                }
            })),
        )
            .into_response(),
        GatewayError::PoolExhausted
        | GatewayError::CredentialStorageUnavailable
        | GatewayError::DatabaseUnavailable => {
            error(request, StatusCode::SERVICE_UNAVAILABLE, code)
        }
    }
}

fn callback_html_response(
    status: StatusCode,
    payload: &serde_json::Value,
    auto_close: bool,
    visible_message: &str,
) -> Response {
    let payload_json = serde_json::to_string(payload)
        .unwrap_or_else(|_| "{}".to_owned())
        .replace('&', "\\u0026")
        .replace('<', "\\u003c")
        .replace('>', "\\u003e");
    let escaped_msg = html_escape(visible_message);
    let close_js = if auto_close {
        "setTimeout(function() { window.close(); }, 400);"
    } else {
        ""
    };
    let html = format!(
        r#"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="utf-8" />
<title>OAuth Callback</title>
<style>
  body {{ font-family: system-ui, -apple-system, sans-serif; padding: 2rem; color: #1f2937; background: #f9fafb; }}
  .card {{ max-width: 28rem; margin: 2rem auto; padding: 1.5rem; border-radius: 0.75rem; background: #ffffff; box-shadow: 0 1px 3px rgba(0,0,0,0.1); }}
  h1 {{ font-size: 1.125rem; margin: 0 0 0.5rem; }}
  p {{ font-size: 0.875rem; color: #4b5563; word-break: break-word; }}
</style>
</head>
<body>
<div class="card">
  <h1>LLM Gateway OAuth</h1>
  <p>{escaped_msg}</p>
</div>
<script>
(function() {{
  var payload = {payload_json};
  try {{
    if (window.opener && !window.opener.closed) {{
      window.opener.postMessage(payload, window.location.origin);
    }}
  }} catch (_) {{}}
  {close_js}
}})();
</script>
</body>
</html>"#
    );

    let mut response = (status, html).into_response();
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/html; charset=utf-8"),
    );
    response
}

fn html_escape(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for ch in input.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(ch),
        }
    }
    out
}

static CODEX_PROXY_ACTIVE: AtomicBool = AtomicBool::new(false);
static CODEX_PROXY_DEADLINE: OnceLock<StdMutex<std::time::Instant>> = OnceLock::new();

fn codex_deadline_cell() -> &'static StdMutex<std::time::Instant> {
    CODEX_PROXY_DEADLINE.get_or_init(|| StdMutex::new(std::time::Instant::now()))
}

/// Spawns a temporary loopback HTTP server on `127.0.0.1:1455` (matching 9router's `startCodexProxy`)
/// so OpenAI's fixed `http://localhost:1455/auth/callback` redirect URI is automatically captured
/// and exchanged when the admin server runs on the same machine as the browser.
fn ensure_codex_loopback_listener(state: AppState) {
    let next_deadline = std::time::Instant::now() + std::time::Duration::from_secs(600);
    if let Ok(mut guard) = codex_deadline_cell().lock() {
        *guard = next_deadline;
    }
    if CODEX_PROXY_ACTIVE.swap(true, Ordering::SeqCst) {
        return;
    }
    tokio::spawn(async move {
        let bind_addr = std::net::SocketAddr::from(([127, 0, 0, 1], 1455));
        let Ok(listener) = tokio::net::TcpListener::bind(bind_addr).await else {
            CODEX_PROXY_ACTIVE.store(false, Ordering::SeqCst);
            return;
        };
        let app = axum::Router::new()
            .route("/auth/callback", axum::routing::get(oauth_callback))
            .route("/callback", axum::routing::get(oauth_callback))
            .with_state(state);

        let shutdown_signal = async {
            loop {
                tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                let expired = codex_deadline_cell()
                    .lock()
                    .map(|deadline| std::time::Instant::now() >= *deadline)
                    .unwrap_or(true);
                if expired {
                    break;
                }
            }
        };

        let _ = axum::serve(listener, app)
            .with_graceful_shutdown(shutdown_signal)
            .await;
        CODEX_PROXY_ACTIVE.store(false, Ordering::SeqCst);
    });
}
