use super::{gateway_error, *};
use crate::services::llm_gateway::{CreatePoolModelRequest, GatewayVendor, UpdatePoolModelPatch};

pub(in crate::app::admin) async fn list_vendors(State(state): State<AppState>) -> Response {
    let vendors = state.llm_gateway_hub.list_vendors_detailed().await;
    Json(serde_json::json!({ "vendors": vendors })).into_response()
}

pub(in crate::app::admin) async fn get_pool_detail(
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

pub(in crate::app::admin) async fn create_pool_model(
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

pub(in crate::app::admin) async fn patch_pool_model(
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

pub(in crate::app::admin) async fn delete_pool_model(
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
