//! Read-only adapter descriptor and metadata-only bootstrap discovery endpoints.

use super::*;
use crate::providers::{
    AdapterSummary, ProviderInspectError, ProviderType, compiled_provider_adapter_registry,
};
use axum::extract::RawQuery;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct BootstrapRequest {
    selection: Value,
}

pub(super) async fn list_provider_adapters(
    RawQuery(query): RawQuery,
    request: Request,
) -> Response {
    let provider_type = match parse_type_filter(query.as_deref()) {
        Ok(provider_type) => provider_type,
        Err(()) => return error(&request, StatusCode::BAD_REQUEST, "validation_failed"),
    };
    let registry = compiled_provider_adapter_registry();
    let items: Vec<AdapterSummary> = registry.list(provider_type).map(Into::into).collect();
    Json(serde_json::json!({"items": items})).into_response()
}

fn parse_type_filter(query: Option<&str>) -> Result<Option<ProviderType>, ()> {
    let Some(query) = query else {
        return Ok(None);
    };
    let mut provider_type = None;
    for (key, value) in url::form_urlencoded::parse(query.as_bytes()) {
        if key != "type" || provider_type.is_some() {
            return Err(());
        }
        provider_type = ProviderType::parse(&value);
        if provider_type.is_none() {
            return Err(());
        }
    }
    Ok(provider_type)
}

pub(super) async fn get_provider_adapter(
    Path(adapter): Path<String>,
    request: Request,
) -> Response {
    match compiled_provider_adapter_registry().get(&adapter) {
        Some(descriptor) => Json(descriptor).into_response(),
        None => error(&request, StatusCode::NOT_FOUND, "not_found"),
    }
}

pub(super) async fn discover_provider_capabilities(
    Path(adapter): Path<String>,
    request: Request,
) -> Response {
    let (request, body): (_, BootstrapRequest) = match json(request).await {
        Ok(value) => value,
        Err(response) => return response,
    };
    let registry = compiled_provider_adapter_registry();
    if registry.get(&adapter).is_none() {
        return error(&request, StatusCode::NOT_FOUND, "not_found");
    }
    match registry.discover(&adapter, &body.selection) {
        Ok(capabilities) => Json(capabilities).into_response(),
        Err(ProviderInspectError::InvalidSelection) => error(
            &request,
            StatusCode::BAD_REQUEST,
            "capability_discovery_invalid",
        ),
        Err(ProviderInspectError::Unsupported) => error(
            &request,
            StatusCode::CONFLICT,
            "capability_discovery_unsupported",
        ),
    }
}
