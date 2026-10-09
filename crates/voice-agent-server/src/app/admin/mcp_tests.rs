//! HTTP adapters for manual MCP observations.
use super::*;
use crate::tools::external_mcp::diagnostic::{McpDiagnosticRunner, McpProbeConfig, McpProbeSource};
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DraftRequest {
    server: McpProbeConfig,
}
async fn run(
    state: AppState,
    request: Request,
    source: McpProbeSource,
    discover: bool,
) -> Response {
    let database = match database(&state) {
        Ok(v) => v,
        Err(e) => return e,
    };
    let Some(manager) = state.external_mcp.as_ref() else {
        return error(&request, StatusCode::SERVICE_UNAVAILABLE, "mcp_unavailable");
    };
    let runner = McpDiagnosticRunner {
        database,
        manager: manager.clone(),
        secrets: state.secret_resolver.clone(),
        network: &state.config.mcp.external.network,
    };
    match runner.probe(source, discover).await {
        Ok(v) => Json(v).into_response(),
        Err(e) => error(&request, e.status, e.code),
    }
}
pub(super) async fn draft_connection(State(state): State<AppState>, request: Request) -> Response {
    let (request, body): (_, DraftRequest) = match json(request).await {
        Ok(v) => v,
        Err(e) => return e,
    };
    run(state, request, McpProbeSource::Draft(body.server), false).await
}
pub(super) async fn draft_discover(State(state): State<AppState>, request: Request) -> Response {
    let (request, body): (_, DraftRequest) = match json(request).await {
        Ok(v) => v,
        Err(e) => return e,
    };
    run(state, request, McpProbeSource::Draft(body.server), true).await
}
pub(super) async fn saved_connection(
    State(state): State<AppState>,
    Path(key): Path<String>,
    request: Request,
) -> Response {
    let (request, body): (_, serde_json::Map<String, Value>) = match json(request).await {
        Ok(v) => v,
        Err(e) => return e,
    };
    if !body.is_empty() {
        return error(&request, StatusCode::BAD_REQUEST, "invalid_test_input");
    }
    run(state, request, McpProbeSource::Saved(key), false).await
}
pub(super) async fn saved_discover(
    State(state): State<AppState>,
    Path(key): Path<String>,
    request: Request,
) -> Response {
    let (request, body): (_, serde_json::Map<String, Value>) = match json(request).await {
        Ok(v) => v,
        Err(e) => return e,
    };
    if !body.is_empty() {
        return error(&request, StatusCode::BAD_REQUEST, "invalid_test_input");
    }
    run(state, request, McpProbeSource::Saved(key), true).await
}
