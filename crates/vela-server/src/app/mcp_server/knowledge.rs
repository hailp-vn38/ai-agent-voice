//! Streamable HTTP MCP transport and Bearer / API-Key authentication for Knowledge (`WikiEngine`).

use super::AppState;
use crate::tools::knowledge_mcp::KnowledgeToolContext;
use axum::{
    Router,
    extract::{Request, State},
    http::{HeaderMap, StatusCode, header},
    middleware::{self, Next},
    response::{IntoResponse, Response},
};
use rmcp::{
    ErrorData, RoleServer, ServerHandler,
    model::*,
    service::RequestContext,
    transport::streamable_http_server::{
        StreamableHttpServerConfig, StreamableHttpService, session::local::LocalSessionManager,
    },
};
use std::{sync::Arc, time::Duration};

fn extract_credential(headers: &HeaderMap) -> Option<&str> {
    if let Some(auth_val) = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
    {
        let trimmed = auth_val.trim();
        if let Some(rest) = trimmed
            .strip_prefix("Bearer ")
            .or_else(|| trimmed.strip_prefix("bearer "))
        {
            let token = rest.trim();
            if !token.is_empty() {
                return Some(token);
            }
        }
    }
    headers
        .get("x-api-key")
        .and_then(|v| v.to_str().ok())
        .map(str::trim)
        .filter(|s| !s.is_empty())
}

#[derive(Clone)]
struct KnowledgeAuthState {
    state: AppState,
}

pub(super) fn router(state: &AppState) -> Router<AppState> {
    let mcp_cfg = &state.config.wiki.mcp;
    let config = StreamableHttpServerConfig::default()
        .with_legacy_session_mode(false)
        .with_json_response(true)
        .with_max_request_body_bytes(256 * 1024)
        .with_cancellation_token(state.lifecycle.stopping().clone());

    let adapter_state = state.clone();
    let service = StreamableHttpService::new(
        move || {
            Ok(KnowledgeAdapter {
                state: adapter_state.clone(),
            })
        },
        Arc::new(LocalSessionManager::default()),
        config,
    );

    let path = mcp_cfg.path.trim();
    let route_path = if path.is_empty() {
        "/mcp/knowledge"
    } else {
        path
    };

    Router::new()
        .route_service(route_path, service)
        .route_layer(middleware::from_fn_with_state(
            KnowledgeAuthState {
                state: state.clone(),
            },
            authenticate,
        ))
}

async fn authenticate(
    State(auth): State<KnowledgeAuthState>,
    mut request: Request,
    next: Next,
) -> Response {
    if !auth.state.admission_gate().is_open() {
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    }
    let Some(core) = auth.state.knowledge_mcp.get() else {
        return StatusCode::NOT_FOUND.into_response();
    };
    if !core.is_enabled() {
        return StatusCode::NOT_FOUND.into_response();
    }
    let Some(_permit) = core.try_acquire_permit() else {
        return StatusCode::TOO_MANY_REQUESTS.into_response();
    };
    let ctx = extract_credential(request.headers()).and_then(|token| core.verify_token(token));
    let Some(ctx) = ctx else {
        return StatusCode::UNAUTHORIZED.into_response();
    };
    let request_timeout = ctx
        .deadline
        .saturating_duration_since(tokio::time::Instant::now())
        + Duration::from_secs(5);
    request.extensions_mut().insert(ctx);
    match tokio::time::timeout(request_timeout, next.run(request)).await {
        Ok(response) => response,
        Err(_) => StatusCode::GATEWAY_TIMEOUT.into_response(),
    }
}

#[derive(Clone)]
struct KnowledgeAdapter {
    state: AppState,
}

fn context(ctx: &RequestContext<RoleServer>) -> Result<KnowledgeToolContext, ErrorData> {
    ctx.extensions
        .get::<http::request::Parts>()
        .and_then(|p| p.extensions.get::<KnowledgeToolContext>())
        .cloned()
        .ok_or_else(|| ErrorData::invalid_request("unauthorized", None))
}

impl ServerHandler for KnowledgeAdapter {
    fn get_info(&self) -> ServerConfig {
        let mut info = ServerConfig::default();
        info.capabilities = ServerCapabilities::builder().enable_tools().build();
        info
    }

    async fn list_tools(
        &self,
        request: Option<PaginatedRequestParams>,
        ctx: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, ErrorData> {
        if request.is_some_and(|p| p.cursor.is_some()) {
            return Err(ErrorData::invalid_params("invalid_cursor", None));
        }
        let context = context(&ctx)?;
        let Some(core) = self.state.knowledge_mcp.get() else {
            return Err(ErrorData::invalid_request("unavailable", None));
        };
        Ok(ListToolsResult {
            tools: core
                .list_tools(&context)
                .into_iter()
                .map(|t| {
                    Tool::new(
                        t.name,
                        t.description,
                        t.parameters.as_object().cloned().unwrap_or_default(),
                    )
                })
                .collect(),
            ..Default::default()
        })
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        ctx: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, ErrorData> {
        let mut context = context(&ctx)?;
        context.cancellation = ctx.ct.clone();
        let Some(core) = self.state.knowledge_mcp.get() else {
            return Err(ErrorData::invalid_request("unavailable", None));
        };
        let result = match core
            .call_tool(
                &context,
                &request.name,
                serde_json::Value::Object(request.arguments.unwrap_or_default()),
            )
            .await
        {
            Ok(value) => CallToolResult::structured(value),
            Err(error) => CallToolResult::error(vec![ContentBlock::text(error.to_string())]),
        };
        Ok(result.into())
    }
}
