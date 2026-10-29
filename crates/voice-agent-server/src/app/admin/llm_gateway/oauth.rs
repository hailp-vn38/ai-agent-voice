use std::sync::{
    Mutex as StdMutex, OnceLock,
    atomic::{AtomicBool, Ordering},
};

use super::{gateway_error, *};
use crate::services::llm_gateway::{
    GatewayVendor, OAuthCallbackSubmission, OAuthPollStatus, StartOAuthLoginRequest,
};

#[derive(Debug, Default, Deserialize)]
pub(in crate::app::admin) struct OAuthCallbackQuery {
    #[serde(default)]
    code: Option<String>,
    #[serde(default)]
    state: Option<String>,
    #[serde(default)]
    error: Option<String>,
    #[serde(default)]
    error_description: Option<String>,
}

pub(in crate::app::admin) async fn start_oauth(
    State(state): State<AppState>,
    request: Request,
) -> Response {
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

pub(in crate::app::admin) async fn oauth_callback(
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

pub(in crate::app::admin) async fn complete_oauth_session(
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

pub(in crate::app::admin) async fn poll_oauth_session(
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
