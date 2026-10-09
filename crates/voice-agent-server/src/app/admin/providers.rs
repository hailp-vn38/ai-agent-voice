//! Admin providers resources.
use super::*;

use crate::database::providers::{Provider, ProviderFilters};
fn provider_response(
    provider: Provider,
    runtime_snapshot: Option<&crate::providers::DatabaseRuntimeSnapshot>,
) -> Value {
    let runtime = runtime_snapshot
        .map(|snapshot| snapshot.runtime_state(&provider.key, provider.revision))
        .unwrap_or_else(|| crate::providers::DatabaseRuntimeState {
            provider_id: provider.id,
            desired_revision: provider.revision,
            status: crate::providers::DatabaseRuntimeStatus::NotLoaded,
            failure: None,
        });
    let matches_desired = matches!(
        runtime.status,
        crate::providers::DatabaseRuntimeStatus::Loaded
    ) && runtime.desired_revision == provider.revision;
    let runtime_status = match runtime.status {
        crate::providers::DatabaseRuntimeStatus::NotLoaded => "not_loaded",
        crate::providers::DatabaseRuntimeStatus::Unavailable => "unavailable",
        crate::providers::DatabaseRuntimeStatus::Loaded => "loaded",
    };
    let credential_env =
        crate::database::secrets::provider_secret_env(&provider.key, &provider.adapter);
    let credential = crate::database::credentials::metadata(provider.credential_json.as_deref());
    let mut response = serde_json::to_value(provider).expect("Provider is serializable");
    response["credential"] = credential;
    let object = response
        .as_object_mut()
        .expect("Provider serializes to object");
    object.insert(
        "runtime_status".into(),
        Value::String(runtime_status.into()),
    );
    object.insert(
        "runtime_matches_desired".into(),
        Value::Bool(matches_desired),
    );
    object.insert("requires_restart".into(), Value::Bool(!matches_desired));
    object.insert(
        "credential_env".into(),
        serde_json::to_value(credential_env).unwrap(),
    );
    response
}
fn managed_provider_response(provider: Provider, state: &AppState) -> Value {
    let Some(manager) = &state.provider_runtime_manager else {
        return provider_response(provider, state.database_runtime_snapshot.as_deref());
    };
    let mut runtime = manager.inspect(provider.id, provider.revision);
    runtime.can_prepare &= provider.enabled != 0;
    use crate::services::provider_runtime::RuntimeState;
    let ready = runtime.desired_state == RuntimeState::Ready;
    let status = match runtime.desired_state {
        RuntimeState::Ready => "loaded",
        RuntimeState::Failed | RuntimeState::Quarantined => "unavailable",
        _ => "not_loaded",
    };
    let credential_env =
        crate::database::secrets::provider_secret_env(&provider.key, &provider.adapter);
    let credential = crate::database::credentials::metadata(provider.credential_json.as_deref());
    let mut response = serde_json::to_value(provider).expect("Provider is serializable");
    response["credential"] = credential;
    response["credential_env"] = serde_json::to_value(credential_env).unwrap();
    response["runtime_status"] = Value::String(status.into());
    response["runtime_matches_desired"] = Value::Bool(ready);
    response["requires_restart"] = Value::Bool(false);
    response["runtime"] =
        serde_json::to_value(runtime).expect("runtime inspection is serializable");
    response
}
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
fn adapter_matches_kind(kind: &str, adapter: &str) -> bool {
    crate::providers::admin_provider_adapter_matches_kind(kind, adapter)
}
/// Mints the immutable identity for a new provider as `{provider_type}_{uuid32}`.
///
/// The key never derives from the display name: names repeat, get renamed and carry no slug
/// rules, so a random suffix keeps every create collision-free without a retry loop. The result
/// satisfies the `valid_key` rules a client-supplied key once had to, so existing rows, URLs and
/// template bindings keep working unchanged. `kind` must already be a compiled provider
/// type, which is what keeps the prefix lowercase.
fn generate_provider_key(kind: &str) -> String {
    format!("{kind}_{}", Uuid::new_v4().simple())
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
    let credential = match body.api_key {
        Some(value) => {
            if !matches!(body.adapter.as_str(), "openai" | "chillaudio_ws")
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
    match database.list_providers(&filters, query.sort.as_deref(), page, size).await {
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
    let provider = match crate::database::providers::provider_by(database, &provider_key).await {
        Ok(v) => v,
        Err(sqlx::Error::RowNotFound) => {
            return error(&request, StatusCode::NOT_FOUND, "not_found");
        }
        Err(e) => return sql_error(&request, &e),
    };
    let (total, rows) = match database
        .provider_templates(provider.id, page, page_size)
        .await
    {
        Ok(value) => value,
        Err(cause) => return sql_error(&request, &cause),
    };
    let rows: Result<_, sqlx::Error> = Ok(rows);
    match rows {
        Ok(rows) => Json(serde_json::json!({
            "provider_key": provider_key,
            "revision": provider.revision,
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
        .into_response(),
        Err(e) => sql_error(&request, &e),
    }
}
pub(super) async fn patch_provider(
    State(state): State<AppState>,
    Path(key): Path<String>,
    request: Request,
) -> Response {
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
    let credential = match body.api_key.value() {
        Some(Some(value))
            if matches!(adapter.as_str(), "openai" | "chillaudio_ws")
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
        None if matches!(adapter.as_str(), "openai" | "chillaudio_ws") => old_credential,
        None => None,
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
    let _ = kind; // type is immutable and retained for the provider instance.
    match crate::database::providers::provider_by(database, &key).await {
        Ok(v) => Json(managed_provider_response(v, &state)).into_response(),
        Err(e) => sql_error(&request, &e),
    }
}

#[cfg(test)]
mod provider_runtime_tests {
    use super::*;
    use crate::providers::{DatabaseRuntimeSnapshot, DatabaseRuntimeState, DatabaseRuntimeStatus};

    #[test]
    fn a_generated_key_is_prefixed_by_type_and_never_repeats() {
        let key = generate_provider_key("llm");
        assert!(key.starts_with("llm_"), "{key}");
        assert!(valid_key(&key), "{key}");
        assert_eq!(key.len(), "llm_".len() + 32, "{key}");
        assert_ne!(key, generate_provider_key("llm"));
        assert_ne!(key, generate_provider_key("tts"));
    }

    #[test]
    fn loaded_runtime_becomes_stale_after_desired_revision_changes() {
        let provider = Provider {
            id: 11,
            key: "llm-main".into(),
            name: "LLM".into(),
            kind: "llm".into(),
            adapter: "openai".into(),
            config_json: "{}".into(),
            enabled: 1,
            revision: 4,
            created_at: 0,
            updated_at: 0,
            credential_json: None,
        };
        let snapshot = DatabaseRuntimeSnapshot::from_states([(
            "llm-main".into(),
            DatabaseRuntimeState {
                provider_id: 11,
                desired_revision: 3,
                status: DatabaseRuntimeStatus::Loaded,
                failure: None,
            },
        )]);
        let response = provider_response(provider, Some(&snapshot));
        assert_eq!(response["runtime_status"], "loaded");
        assert_eq!(response["runtime_matches_desired"], false);
        assert_eq!(response["requires_restart"], true);
    }
}
