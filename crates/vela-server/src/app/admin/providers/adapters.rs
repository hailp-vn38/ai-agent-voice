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
    api_key: Option<crate::database::secrets::SecretValue>,
    saved_credential: Option<crate::services::test_credentials::SavedCredential>,
}

pub(in crate::app::admin) async fn list_provider_adapters(
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

pub(in crate::app::admin) async fn get_provider_adapter(
    Path(adapter): Path<String>,
    request: Request,
) -> Response {
    match compiled_provider_adapter_registry().get(&adapter) {
        Some(descriptor) => Json(descriptor).into_response(),
        None => error(&request, StatusCode::NOT_FOUND, "not_found"),
    }
}

pub(in crate::app::admin) async fn get_provider_capabilities(
    State(state): State<AppState>,
    Path(key): Path<String>,
    request: Request,
) -> Response {
    if key.starts_with("config:") {
        use crate::{
            providers::config_catalog::ProviderRef,
            services::provider_runtime::{ProviderIdentity, ProviderVersion},
        };
        let reference = match ProviderRef::parse(&key, None) {
            Ok(value) => value,
            Err(()) => return error(&request, StatusCode::BAD_REQUEST, "invalid_provider"),
        };
        let snapshot = match reference.snapshot(&state.config) {
            Ok(value) => value,
            Err(()) => return error(&request, StatusCode::NOT_FOUND, "provider_not_found"),
        };
        let version = ProviderVersion {
            identity: ProviderIdentity::Deployment {
                kind: snapshot.kind,
                key: snapshot.key,
            },
            revision: snapshot.revision,
        };
        let Some(capabilities) = state
            .provider_runtime_manager
            .as_ref()
            .and_then(|manager| manager.ready_capabilities_version(&version))
        else {
            return error(
                &request,
                StatusCode::CONFLICT,
                "provider_runtime_not_loaded",
            );
        };
        return Json(serde_json::json!({ "provider_key": key, "source": "config", "capabilities": capabilities,
            "runtime": { "runtime_status": "loaded", "tested_provider_id": null, "tested_revision": snapshot.revision, "runtime_matches_desired": true, "requires_restart": false }
        })).into_response();
    }
    let database = match database(&state) {
        Ok(database) => database,
        Err(response) => return response,
    };
    let provider = database.provider_capability_identity(&key).await;
    let (provider_id, adapter, kind, revision) = match provider {
        Ok(provider) => provider,
        Err(sqlx::Error::RowNotFound) => {
            return error(&request, StatusCode::NOT_FOUND, "not_found");
        }
        Err(error_value) => return sql_error(&request, &error_value),
    };
    if let Some(manager) = &state.provider_runtime_manager {
        let Some(capabilities) = manager.ready_capabilities(provider_id, revision) else {
            return error(
                &request,
                StatusCode::CONFLICT,
                "provider_runtime_not_loaded",
            );
        };
        return Json(serde_json::json!({
            "provider_key": key, "capabilities": capabilities,
            "runtime": { "runtime_status":"loaded", "tested_provider_id":provider_id,
                "tested_revision":revision, "runtime_matches_desired":true, "requires_restart":false }
        })).into_response();
    }
    let Some(snapshot) = state.database_runtime_snapshot.as_deref() else {
        return error(
            &request,
            StatusCode::CONFLICT,
            "provider_runtime_not_loaded",
        );
    };
    let runtime = snapshot.runtime_state(&key, revision);
    if !matches!(
        runtime.status,
        crate::providers::DatabaseRuntimeStatus::Loaded
    ) {
        return error(
            &request,
            StatusCode::CONFLICT,
            "provider_runtime_not_loaded",
        );
    }
    let Some(descriptor) = compiled_provider_adapter_registry().get(&adapter) else {
        return error(
            &request,
            StatusCode::CONFLICT,
            "provider_runtime_not_loaded",
        );
    };
    if descriptor.provider_type.as_str() != kind {
        return error(
            &request,
            StatusCode::CONFLICT,
            "provider_runtime_not_loaded",
        );
    }
    Json(serde_json::json!({
        "provider_key": key,
        "capabilities": descriptor.capabilities,
        "runtime": {
            "runtime_status": "loaded",
            "runtime_matches_desired": runtime.desired_revision == revision,
            "requires_restart": runtime.desired_revision != revision,
        }
    }))
    .into_response()
}

pub(in crate::app::admin) async fn discover_provider_capabilities(
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
    if body.api_key.is_some() || body.saved_credential.is_some() {
        return error(&request, StatusCode::BAD_REQUEST, "credential_invalid");
    }
    match registry.discover(&adapter, &body.selection) {
        Ok(capabilities) => Json(capabilities).into_response(),
        Err(ProviderInspectError::InvalidSelection) => error(
            &request,
            StatusCode::BAD_REQUEST,
            "capability_discovery_invalid",
        ),
        Err(ProviderInspectError::RemoteUnavailable) => error(
            &request,
            StatusCode::BAD_GATEWAY,
            "capability_discovery_unavailable",
        ),
        Err(ProviderInspectError::Unsupported) => error(
            &request,
            StatusCode::CONFLICT,
            "capability_discovery_unsupported",
        ),
    }
}
