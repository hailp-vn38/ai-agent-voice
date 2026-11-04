//! Authentication and MCP envelopes only; tool authority and dispatch belong to SystemMcp / KnowledgeMcp.
mod auth;
mod knowledge;
use super::AppState;
use crate::tools::system_mcp::{SystemCaller, SystemMcp, SystemToolContext};
use axum::{
    Router,
    extract::{Request, State},
    http::{StatusCode, header},
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
use std::sync::Arc;

pub(crate) fn validate(
    config: &crate::config::AppConfig,
) -> Result<(), crate::config::ConfigError> {
    if config.system_mcp.http.enabled && auth::Verifier::new(config).is_err() {
        return Err(crate::config::ConfigError::Validation("System MCP HTTP requires dedicated JWT settings, HTTPS resource/origins, and loopback bind".into()));
    }
    Ok(())
}
#[derive(Clone)]
struct AuthState {
    state: AppState,
    verifier: Arc<auth::Verifier>,
    permits: Arc<tokio::sync::Semaphore>,
}
pub(super) fn router(state: &AppState) -> Router<AppState> {
    let sys = if state.config.system_mcp.http.enabled
        && let Ok(verifier) = auth::Verifier::new(&state.config)
    {
        authenticated_router(state, verifier)
    } else {
        Router::new()
    };
    sys.merge(knowledge::router(state))
}
fn authenticated_router(state: &AppState, verifier: auth::Verifier) -> Router<AppState> {
    let Some(core) = &state.system_mcp else {
        return Router::new();
    };
    let core = core.clone();
    let mut config = StreamableHttpServerConfig::default()
        .with_legacy_session_mode(false)
        .with_json_response(true)
        .with_max_request_body_bytes(16 * 1024)
        .with_cancellation_token(state.lifecycle.stopping().clone())
        .enforce_origin_validation();
    let resource =
        url::Url::parse(&state.config.system_mcp.http.audience).expect("validated resource");
    config.allowed_hosts =
        vec![resource[url::Position::BeforeHost..url::Position::AfterPort].to_owned()];
    config.allowed_origins = state.config.system_mcp.http.allowed_origins.clone();
    let service = StreamableHttpService::new(
        move || Ok(Adapter { core: core.clone() }),
        Arc::new(LocalSessionManager::default()),
        config,
    );
    Router::new()
        .route_service("/mcp/system", service)
        .route_layer(middleware::from_fn_with_state(
            AuthState {
                state: state.clone(),
                verifier: Arc::new(verifier),
                permits: Arc::new(tokio::sync::Semaphore::new(16)),
            },
            authenticate,
        ))
}
async fn authenticate(State(auth): State<AuthState>, mut request: Request, next: Next) -> Response {
    if !auth.state.admission_gate().is_open() {
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    }
    let Ok(_permit) = auth.permits.clone().try_acquire_owned() else {
        return StatusCode::TOO_MANY_REQUESTS.into_response();
    };
    let ctx = request
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .and_then(|v| auth.verifier.verify(v).ok());
    let Some(ctx) = ctx else {
        return StatusCode::UNAUTHORIZED.into_response();
    };
    let Some(db) = &auth.state.database else {
        return StatusCode::SERVICE_UNAVAILABLE.into_response();
    };
    let SystemCaller::Remote {
        jti, expires_at, ..
    } = &ctx.caller
    else {
        return StatusCode::UNAUTHORIZED.into_response();
    };
    if !db
        .system_mcp_token_allowed(jti, *expires_at)
        .await
        .unwrap_or(false)
    {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    request.extensions_mut().insert(ctx);
    match tokio::time::timeout(std::time::Duration::from_secs(35), next.run(request)).await {
        Ok(response) => response,
        Err(_) => StatusCode::GATEWAY_TIMEOUT.into_response(),
    }
}
#[derive(Clone)]
struct Adapter {
    core: Arc<SystemMcp>,
}
fn context(ctx: &RequestContext<RoleServer>) -> Result<SystemToolContext, ErrorData> {
    ctx.extensions
        .get::<http::request::Parts>()
        .and_then(|p| p.extensions.get::<SystemToolContext>())
        .cloned()
        .ok_or_else(|| ErrorData::invalid_request("unauthorized", None))
}
impl ServerHandler for Adapter {
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
        Ok(ListToolsResult {
            tools: self
                .core
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
        let result = match self
            .core
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

#[cfg(test)]
mod tests;
