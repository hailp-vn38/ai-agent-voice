//! Admin HTTP endpoints for the Internal LLM Gateway (`/api/admin/llm-gateway/*`).

mod accounts;
mod oauth;
mod pools;
#[cfg(test)]
mod tests;

pub(super) use accounts::{
    clear_account_cooldown, delete_account, get_account_quota, list_accounts, list_quotas,
    patch_account, refresh_account, refresh_account_quota,
};
pub(super) use oauth::{
    complete_oauth_session, oauth_callback, poll_oauth_session, start_oauth,
};
pub(super) use pools::{
    create_pool_model, delete_pool_model, get_pool_detail, list_vendors, patch_pool_model,
};

use super::*;
use crate::services::llm_gateway::GatewayError;

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
