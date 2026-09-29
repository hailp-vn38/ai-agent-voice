//! Admin providers resources.
use super::*;

#[derive(Serialize, FromRow)]
struct Provider {
    id: i64,
    key: String,
    name: String,
    #[serde(rename = "type")]
    kind: String,
    adapter: String,
    config_json: String,
    enabled: i64,
    revision: i64,
    created_at: i64,
    updated_at: i64,
    has_secret_ref: bool,
}
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
    let mut response = serde_json::to_value(provider).expect("Provider is serializable");
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
    response
}
#[derive(Deserialize)]
struct CreateProvider {
    key: String,
    name: String,
    #[serde(rename = "type")]
    kind: String,
    adapter: String,
    config_json: Value,
    #[serde(default)]
    secret_ref: Option<String>,
}
#[derive(Deserialize)]
struct PatchProvider {
    #[serde(default)]
    key: Patch<String>,
    #[serde(default)]
    name: Patch<String>,
    #[serde(default)]
    adapter: Patch<String>,
    #[serde(default)]
    config_json: Patch<Value>,
    #[serde(default)]
    secret_ref: Patch<String>,
    #[serde(default)]
    enabled: Patch<bool>,
}
pub(super) fn valid_secret_ref(value: &str) -> bool {
    SecretRef::parse(value.into()).is_ok()
}
fn adapter_matches_kind(kind: &str, adapter: &str) -> bool {
    matches!(
        (kind, adapter),
        ("vad", "silero_onnx")
            | ("asr", "zipformer_sherpa" | "gipformer_sherpa_offline")
            | ("llm", "openai")
            | ("tts", "zerotts_onnx")
    )
}
async fn provider_by(pool: &SqlitePool, key: &str) -> Result<Provider, sqlx::Error> {
    sqlx::query_as("SELECT id,key,name,type AS kind,adapter,config_json,enabled,revision,created_at,updated_at,secret_ref IS NOT NULL AS has_secret_ref FROM providers WHERE key=?").bind(key).fetch_one(pool).await
}
pub(super) async fn create_provider(State(state): State<AppState>, request: Request) -> Response {
    let (request, body): (_, CreateProvider) = match json(request).await {
        Ok(v) => v,
        Err(e) => return e,
    };
    if !valid_key(&body.key)
        || !valid_text(&body.name, 128, false)
        || !matches!(body.kind.as_str(), "vad" | "asr" | "llm" | "tts")
        || !adapter_matches_kind(&body.kind, &body.adapter)
        || body
            .secret_ref
            .as_ref()
            .is_some_and(|v| !valid_secret_ref(v))
    {
        return error(&request, StatusCode::BAD_REQUEST, "validation_failed");
    };
    let config = match provider_config::validate(&body.adapter, &body.config_json) {
        Ok(v) => v,
        Err(_) => return error(&request, StatusCode::BAD_REQUEST, "provider_config_invalid"),
    };
    let pool = match db(&state) {
        Ok(v) => v,
        Err(e) => return e,
    };
    let mut tx = match pool.begin().await {
        Ok(v) => v,
        Err(_) => {
            return error(
                &request,
                StatusCode::SERVICE_UNAVAILABLE,
                "database_unavailable",
            );
        }
    };
    let r=sqlx::query("INSERT INTO providers(key,name,type,adapter,config_json,secret_ref,created_at,updated_at) VALUES(?,?,?,?,?,?,?,?)").bind(&body.key).bind(&body.name).bind(&body.kind).bind(&body.adapter).bind(config).bind(&body.secret_ref).bind(now()).bind(now()).execute(&mut *tx).await;
    let provider_id = match r {
        Ok(v) => v.last_insert_rowid(),
        Err(e) => return mutation_sql_error(&request, &e),
    };
    if audit(
        &mut *tx,
        id(&request),
        "provider",
        provider_id,
        "create",
        None,
        Some(1),
        "success",
        None,
    )
    .await
    .is_err()
        || tx.commit().await.is_err()
    {
        return error(
            &request,
            StatusCode::SERVICE_UNAVAILABLE,
            "database_unavailable",
        );
    };
    match provider_by(pool, &body.key).await {
        Ok(v) => (
            StatusCode::CREATED,
            Json(provider_response(
                v,
                state.database_runtime_snapshot.as_deref(),
            )),
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
    let pool = match db(&state) {
        Ok(v) => v,
        Err(e) => return e,
    };
    match provider_by(pool, &key).await {
        Ok(v) => Json(provider_response(
            v,
            state.database_runtime_snapshot.as_deref(),
        ))
        .into_response(),
        Err(sqlx::Error::RowNotFound) => error(&request, StatusCode::NOT_FOUND, "not_found"),
        Err(e) => sql_error(&request, &e),
    }
}
pub(super) async fn list_providers(
    State(state): State<AppState>,
    Query(query): Query<PageQuery>,
    request: Request,
) -> Response {
    let (page, size) = match page_bounds(&query) {
        Ok(v) => v,
        Err(c) => return error(&request, StatusCode::BAD_REQUEST, c),
    };
    let pool = match db(&state) {
        Ok(v) => v,
        Err(e) => return e,
    };
    match sqlx::query_as::<_,Provider>("SELECT id,key,name,type AS kind,adapter,config_json,enabled,revision,created_at,updated_at,secret_ref IS NOT NULL AS has_secret_ref FROM providers ORDER BY key LIMIT ? OFFSET ?").bind(i64::from(size)).bind(i64::from((page-1)*size)).fetch_all(pool).await{Ok(items)=>Json(serde_json::json!({"items":items.into_iter().map(|item| provider_response(item, state.database_runtime_snapshot.as_deref())).collect::<Vec<_>>(),"page":page,"page_size":size,"max_page_size":PAGE_MAX})).into_response(),Err(e)=>sql_error(&request,&e)}
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
    let pool = match db(&state) {
        Ok(v) => v,
        Err(e) => return e,
    };
    let old: Result<(i64, String, String, String, String, Option<String>, i64, i64), _> =
        sqlx::query_as("SELECT id,name,type,adapter,config_json,secret_ref,enabled,revision FROM providers WHERE key=?")
            .bind(&key).fetch_one(pool).await;
    let (provider_id, old_name, kind, old_adapter, old_config, old_secret, old_enabled, revision) =
        match old {
            Ok(v) => v,
            Err(sqlx::Error::RowNotFound) => {
                return error(&request, StatusCode::NOT_FOUND, "not_found");
            }
            Err(e) => return sql_error(&request, &e),
        };
    if revision != expected {
        audit_conflict(pool, id(&request).into(), "provider", provider_id, expected).await;
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
    let secret = match body.secret_ref.value() {
        Some(Some(v)) if valid_secret_ref(&v) => Some(v),
        Some(Some(_)) => return error(&request, StatusCode::BAD_REQUEST, "validation_failed"),
        Some(None) => None,
        None => old_secret,
    };
    let enabled = match body.enabled.value() {
        Some(Some(v)) => i64::from(v),
        Some(None) => return error(&request, StatusCode::BAD_REQUEST, "validation_failed"),
        None => old_enabled,
    };
    let mut tx = match pool.begin().await {
        Ok(v) => v,
        Err(_) => {
            return error(
                &request,
                StatusCode::SERVICE_UNAVAILABLE,
                "database_unavailable",
            );
        }
    };
    let changed = sqlx::query("UPDATE providers SET name=?,adapter=?,config_json=?,secret_ref=?,enabled=?,revision=revision+1,updated_at=? WHERE id=? AND revision=?")
        .bind(name).bind(adapter).bind(config).bind(secret).bind(enabled).bind(now()).bind(provider_id).bind(expected)
        .execute(&mut *tx).await.map(|v| v.rows_affected() == 1).unwrap_or(false);
    if !changed {
        let _ = tx.rollback().await;
        return error(&request, StatusCode::CONFLICT, "revision_conflict");
    }
    if audit(
        &mut *tx,
        id(&request),
        "provider",
        provider_id,
        "update",
        Some(expected),
        Some(expected + 1),
        "success",
        None,
    )
    .await
    .is_err()
        || tx.commit().await.is_err()
    {
        return error(
            &request,
            StatusCode::SERVICE_UNAVAILABLE,
            "database_unavailable",
        );
    }
    let _ = kind; // type is immutable and retained for the provider instance.
    match provider_by(pool, &key).await {
        Ok(v) => Json(provider_response(
            v,
            state.database_runtime_snapshot.as_deref(),
        ))
        .into_response(),
        Err(e) => sql_error(&request, &e),
    }
}

#[cfg(test)]
mod provider_runtime_tests {
    use super::*;
    use crate::providers::{DatabaseRuntimeSnapshot, DatabaseRuntimeState, DatabaseRuntimeStatus};

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
            has_secret_ref: false,
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
