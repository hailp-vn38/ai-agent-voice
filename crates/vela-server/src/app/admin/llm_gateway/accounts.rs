use super::{gateway_error, *};
use crate::services::llm_gateway::{GatewayVendor, UpdateGatewayAccountPatch};

#[derive(Debug, Default, Deserialize)]
pub(in crate::app::admin) struct ListAccountsQuery {
    #[serde(default)]
    vendor: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
pub(in crate::app::admin) struct ListQuotasQuery {
    #[serde(default)]
    vendor: Option<String>,
    #[serde(default)]
    force: Option<bool>,
}

#[derive(Debug, Default, Deserialize)]
pub(in crate::app::admin) struct GetQuotaQuery {
    #[serde(default)]
    force: Option<bool>,
}

pub(in crate::app::admin) async fn list_accounts(
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

pub(in crate::app::admin) async fn list_quotas(
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

pub(in crate::app::admin) async fn get_account_quota(
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

pub(in crate::app::admin) async fn refresh_account_quota(
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

pub(in crate::app::admin) async fn refresh_account(
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

pub(in crate::app::admin) async fn clear_account_cooldown(
    State(state): State<AppState>,
    Path(account_key): Path<String>,
    request: Request,
) -> Response {
    match state.llm_gateway_hub.clear_cooldown(&account_key).await {
        Ok(account) => Json(serde_json::json!({ "account": account })).into_response(),
        Err(err) => gateway_error(&request, err),
    }
}

pub(in crate::app::admin) async fn patch_account(
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

pub(in crate::app::admin) async fn delete_account(
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
