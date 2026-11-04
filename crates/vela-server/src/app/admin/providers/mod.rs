//! Admin providers resources.

pub(super) mod adapters;
pub(super) mod diagnostics;
pub(super) mod draft_tests;
mod response;

pub(super) use adapters::{
    discover_provider_capabilities, get_provider_adapter, get_provider_capabilities,
    list_provider_adapters,
};
pub(super) use diagnostics::{
    prepare_provider, test_asr_provider, test_llm_provider, test_tts_provider, test_vad_provider,
};

use self::response::{adapter_matches_kind, generate_provider_key, managed_provider_response};
use super::*;
use crate::database::providers::ProviderFilters;

/// A create request carries business data only. The provider key is server-owned and immutable, so
/// a client-chosen `key` is a contract error rather than input to ignore; silently dropping it
/// would leave the client believing its own key was adopted.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CreateProvider {
    name: String,
    #[serde(rename = "type")]
    kind: String,
    adapter: String,
    config_json: Value,
    #[serde(default)]
    api_key: Option<crate::database::secrets::SecretValue>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PatchProvider {
    #[serde(default)]
    api_key: Patch<crate::database::secrets::SecretValue>,
    #[serde(default)]
    key: Patch<String>,
    #[serde(default)]
    name: Patch<String>,
    #[serde(default)]
    adapter: Patch<String>,
    #[serde(default)]
    config_json: Patch<Value>,
    #[serde(default)]
    enabled: Patch<bool>,
}

pub(super) async fn create_provider(State(state): State<AppState>, request: Request) -> Response {
    let (request, body): (_, CreateProvider) = match json(request).await {
        Ok(v) => v,
        Err(e) => return e,
    };
    if !valid_text(&body.name, 128, false)
        || !matches!(body.kind.as_str(), "vad" | "asr" | "llm" | "tts")
        || !adapter_matches_kind(&body.kind, &body.adapter)
    {
        return error(&request, StatusCode::BAD_REQUEST, "validation_failed");
    }
    let config = match provider_config::validate(&body.adapter, &body.config_json) {
        Ok(v) => v,
        Err(_) => return error(&request, StatusCode::BAD_REQUEST, "provider_config_invalid"),
    };
    let provider_key = generate_provider_key(&body.kind);
    let has_gateway_binding = body.adapter == "llm_gateway"
        || body
            .config_json
            .get("gateway_binding")
            .and_then(Value::as_str)
            .is_some_and(|s| !s.trim().is_empty());
    let credential = if has_gateway_binding {
        if body.api_key.is_some() {
            return error(&request, StatusCode::BAD_REQUEST, "credential_invalid");
        }
        None
    } else {
        match body.api_key {
            Some(value) => {
                if !crate::database::secrets::adapter_supports_credential(&body.adapter)
                    || !crate::database::credentials::valid_input(&value)
                {
                    return error(&request, StatusCode::BAD_REQUEST, "credential_invalid");
                }
                match state
                    .secret_resolver
                    .seal(&value, &format!("provider:{provider_key}"))
                {
                    Ok(record) => Some(record),
                    Err(_) => {
                        return error(
                            &request,
                            StatusCode::SERVICE_UNAVAILABLE,
                            "credential_storage_unavailable",
                        );
                    }
                }
            }
            None => None,
        }
    };
    let database = match database(&state) {
        Ok(v) => v,
        Err(e) => return e,
    };
    if let Err(cause) = database
        .create_provider(
            crate::database::providers::NewProvider {
                key: &provider_key,
                name: &body.name,
                kind: &body.kind,
                adapter: &body.adapter,
                config: &config,
                credential: credential.as_deref(),
            },
            id(&request),
        )
        .await
    {
        return write_error(&request, cause);
    }
    if body.kind == "llm" {
        let _ = crate::services::task_flow::sync_orchestrator_from_state(&state).await;
    }
    match crate::database::providers::provider_by(database, &provider_key).await {
        Ok(v) => (
            StatusCode::CREATED,
            Json(managed_provider_response(v, &state)),
        )
            .into_response(),
        Err(e) => sql_error(&request, &e),
    }
}

pub(super) async fn get_provider(
    State(state): State<AppState>,
    Path(key): Path<String>,
    request: Request,
) -> Response {
    let database = match database(&state) {
        Ok(v) => v,
        Err(e) => return e,
    };
    if key.starts_with("config:") {
        return match crate::providers::config_catalog::ProviderRef::parse(&key, None)
            .and_then(|r| r.snapshot(&state.config))
        {
            Ok(snapshot) => Json(managed_provider_response(
                crate::providers::config_catalog::public_provider(&snapshot),
                &state,
            ))
            .into_response(),
            Err(()) => error(&request, StatusCode::NOT_FOUND, "invalid_provider"),
        };
    }
    match crate::database::providers::provider_by(database, &key).await {
        Ok(v) => Json(managed_provider_response(v, &state)).into_response(),
        Err(sqlx::Error::RowNotFound) => error(&request, StatusCode::NOT_FOUND, "not_found"),
        Err(e) => sql_error(&request, &e),
    }
}

pub(super) async fn list_providers(
    State(state): State<AppState>,
    Query(query): Query<ProviderListQuery>,
    request: Request,
) -> Response {
    let (page, size) = match page_bounds(query.page, query.page_size) {
        Ok(v) => v,
        Err(c) => return error(&request, StatusCode::BAD_REQUEST, c),
    };
    let database = match database(&state) {
        Ok(v) => v,
        Err(e) => return e,
    };
    if query
        .sort
        .as_deref()
        .is_some_and(|sort| !matches!(sort, "key" | "-key" | "name" | "-name"))
    {
        return error(&request, StatusCode::BAD_REQUEST, "invalid_query");
    }
    let filters = match ProviderFilters::new(query.enabled, query.q.clone(), query.kind.clone()) {
        Ok(value) => value,
        Err(()) => return error(&request, StatusCode::BAD_REQUEST, "invalid_query"),
    };
    let config_items: Vec<_> =
        crate::providers::config_catalog::deployment_snapshots(&state.config)
            .iter()
            .map(crate::providers::config_catalog::public_provider)
            .collect();
    let config_json = serde_json::to_string(&config_items).expect("public catalog is serializable");
    match database.list_providers(&filters, query.sort.as_deref(), page, size, &config_json).await {
        Ok(result) => Json(serde_json::json!({"items":result.items.into_iter().map(|item| managed_provider_response(item, &state)).collect::<Vec<_>>(),"page":page,"page_size":size,"max_page_size":PAGE_MAX,"total":result.total,"total_pages":provider_total_pages(result.total, size),"facets":result.facets})).into_response(),
        Err(value) => sql_error(&request, &value),
    }
}

#[derive(Deserialize)]
pub(super) struct ProviderListQuery {
    #[serde(default)]
    page: Option<u32>,
    #[serde(default)]
    page_size: Option<u32>,
    #[serde(default)]
    enabled: Option<bool>,
    #[serde(default)]
    q: Option<String>,
    #[serde(rename = "type", default)]
    kind: Option<String>,
    #[serde(default)]
    sort: Option<String>,
}

fn provider_total_pages(total: i64, page_size: u32) -> i64 {
    (total + i64::from(page_size) - 1) / i64::from(page_size)
}

pub(super) async fn list_provider_templates(
    State(state): State<AppState>,
    Path(provider_key): Path<String>,
    Query(query): Query<PageQuery>,
    request: Request,
) -> Response {
    let (page, page_size) = match page_bounds(query.page, query.page_size) {
        Ok(value) if query.enabled.is_none() && query.sort.is_none() => value,
        _ => return error(&request, StatusCode::BAD_REQUEST, "invalid_query"),
    };
    let database = match database(&state) {
        Ok(v) => v,
        Err(e) => return e,
    };
    let (revision, result) = if provider_key.starts_with("config:") {
        let reference =
            match crate::providers::config_catalog::ProviderRef::parse(&provider_key, None) {
                Ok(reference) if reference.snapshot(&state.config).is_ok() => reference,
                _ => return error(&request, StatusCode::NOT_FOUND, "invalid_provider"),
            };
        (
            1,
            database
                .config_provider_templates(&reference.kind, &reference.key, page, page_size)
                .await,
        )
    } else {
        let provider = match crate::database::providers::provider_by(database, &provider_key).await
        {
            Ok(provider) => provider,
            Err(sqlx::Error::RowNotFound) => {
                return error(&request, StatusCode::NOT_FOUND, "not_found");
            }
            Err(cause) => return sql_error(&request, &cause),
        };
        (
            provider.revision,
            database
                .provider_templates(provider.id, page, page_size)
                .await,
        )
    };
    let (total, rows) = match result {
        Ok(value) => value,
        Err(cause) => return sql_error(&request, &cause),
    };
    Json(serde_json::json!({
        "provider_key": provider_key,
        "revision": revision,
        "page": page,
        "page_size": page_size,
        "max_page_size": PAGE_MAX,
        "total": total,
        "items": rows.into_iter().map(|(key, name, provider_type, enabled)| serde_json::json!({
            "key": key,
            "name": name,
            "provider_type": provider_type,
            "enabled": enabled != 0,
        })).collect::<Vec<_>>(),
    }))
    .into_response()
}

pub(super) async fn patch_provider(
    State(state): State<AppState>,
    Path(key): Path<String>,
    request: Request,
) -> Response {
    if key.starts_with("config:") {
        return error(&request, StatusCode::CONFLICT, "config_provider_readonly");
    }
    let (request, body): (_, PatchProvider) = match json(request).await {
        Ok(v) => v,
        Err(e) => return e,
    };
    if !matches!(body.key, Patch::Absent) {
        return error(&request, StatusCode::BAD_REQUEST, "immutable_field");
    }
    let expected = match expected(request.headers()) {
        Ok(v) => v,
        Err(code) => return error(&request, StatusCode::BAD_REQUEST, code),
    };
    let database = match database(&state) {
        Ok(v) => v,
        Err(e) => return e,
    };
    let old = crate::database::providers::provider_by(database, &key).await;
    let (
        provider_id,
        old_name,
        kind,
        old_adapter,
        old_config,
        old_enabled,
        revision,
        old_credential,
    ) = match old {
        Ok(v) => (
            v.id,
            v.name,
            v.kind,
            v.adapter,
            v.config_json,
            v.enabled,
            v.revision,
            v.credential_json,
        ),
        Err(sqlx::Error::RowNotFound) => {
            return error(&request, StatusCode::NOT_FOUND, "not_found");
        }
        Err(e) => return sql_error(&request, &e),
    };
    if revision != expected {
        database
            .provider_conflict(id(&request), provider_id, expected)
            .await;
        return error(&request, StatusCode::CONFLICT, "revision_conflict");
    }
    let name = match body.name.value() {
        Some(Some(v)) if valid_text(&v, 128, false) => v,
        Some(_) => return error(&request, StatusCode::BAD_REQUEST, "validation_failed"),
        None => old_name,
    };
    let adapter = match body.adapter.value() {
        Some(Some(v)) if !v.is_empty() => v,
        Some(_) => return error(&request, StatusCode::BAD_REQUEST, "validation_failed"),
        None => old_adapter,
    };
    if !adapter_matches_kind(&kind, &adapter) {
        return error(&request, StatusCode::BAD_REQUEST, "provider_config_invalid");
    }
    let config_value = match body.config_json.value() {
        Some(Some(v)) => v,
        Some(None) => return error(&request, StatusCode::BAD_REQUEST, "provider_config_invalid"),
        None => match serde_json::from_str(&old_config) {
            Ok(v) => v,
            Err(_) => return error(&request, StatusCode::BAD_REQUEST, "provider_config_invalid"),
        },
    };
    let config = match provider_config::validate(&adapter, &config_value) {
        Ok(v) => v,
        Err(_) => return error(&request, StatusCode::BAD_REQUEST, "provider_config_invalid"),
    };
    let has_gateway_binding = adapter == "llm_gateway"
        || config_value
            .get("gateway_binding")
            .and_then(Value::as_str)
            .is_some_and(|s| !s.trim().is_empty());
    let credential = if has_gateway_binding {
        match body.api_key.value() {
            None | Some(None) => None,
            Some(Some(_)) => return error(&request, StatusCode::BAD_REQUEST, "credential_invalid"),
        }
    } else {
        match body.api_key.value() {
            Some(Some(value))
                if crate::database::secrets::adapter_supports_credential(&adapter)
                    && crate::database::credentials::valid_input(&value) =>
            {
                match state
                    .secret_resolver
                    .seal(&value, &format!("provider:{key}"))
                {
                    Ok(record) => Some(record),
                    Err(_) => {
                        return error(
                            &request,
                            StatusCode::SERVICE_UNAVAILABLE,
                            "credential_storage_unavailable",
                        );
                    }
                }
            }
            Some(_) => return error(&request, StatusCode::BAD_REQUEST, "credential_invalid"),
            None if crate::database::secrets::adapter_supports_credential(&adapter) => {
                old_credential
            }
            None => None,
        }
    };
    let enabled = match body.enabled.value() {
        Some(Some(v)) => i64::from(v),
        Some(None) => return error(&request, StatusCode::BAD_REQUEST, "validation_failed"),
        None => old_enabled,
    };
    if let Err(cause) = database
        .update_provider(
            provider_id,
            expected,
            crate::database::providers::ProviderChanges {
                name: &name,
                adapter: &adapter,
                config: &config,
                credential: credential.as_deref(),
                enabled,
            },
            id(&request),
        )
        .await
    {
        return write_error(&request, cause);
    }
    if let Some(prewarm) = &state.provider_prewarm {
        prewarm.provider(provider_id, expected + 1).await;
    }
    if kind == "llm" {
        let _ = crate::services::task_flow::sync_orchestrator_from_state(&state).await;
    }
    let _ = kind; // type is immutable and retained for the provider instance.
    match crate::database::providers::provider_by(database, &key).await {
        Ok(v) => Json(managed_provider_response(v, &state)).into_response(),
        Err(e) => sql_error(&request, &e),
    }
}
