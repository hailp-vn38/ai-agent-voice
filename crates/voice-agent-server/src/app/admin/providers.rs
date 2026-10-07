//! Admin providers resources.
use super::*;
use sqlx::QueryBuilder;

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
    let mut response = serde_json::to_value(provider).expect("Provider is serializable");
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
    crate::providers::admin_provider_adapter_matches_kind(kind, adapter)
}
async fn provider_by(pool: &SqlitePool, key: &str) -> Result<Provider, sqlx::Error> {
    sqlx::query_as("SELECT id,key,name,type AS kind,adapter,config_json,enabled,revision,created_at,updated_at,secret_ref IS NOT NULL AS has_secret_ref FROM providers WHERE key=?").bind(key).fetch_one(pool).await
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
        || !matches!(
            body.kind.as_str(),
            "vad" | "asr" | "llm" | "tts" | "speaker"
        )
        || !adapter_matches_kind(&body.kind, &body.adapter)
        || body
            .secret_ref
            .as_ref()
            .is_some_and(|v| !valid_secret_ref(v))
    {
        return error(&request, StatusCode::BAD_REQUEST, "validation_failed");
    }
    if body.kind == "speaker" && body.secret_ref.is_some() {
        return error(&request, StatusCode::BAD_REQUEST, "provider_config_invalid");
    }
    let provider_key = generate_provider_key(&body.kind);
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
    let r=sqlx::query("INSERT INTO providers(key,name,type,adapter,config_json,secret_ref,created_at,updated_at) VALUES(?,?,?,?,?,?,?,?)").bind(&provider_key).bind(&body.name).bind(&body.kind).bind(&body.adapter).bind(config).bind(&body.secret_ref).bind(now()).bind(now()).execute(&mut *tx).await;
    let provider_id = match r {
        Ok(v) => v.last_insert_rowid(),
        Err(e) => return mutation_sql_error(&request, &e),
    };
    if audit(
        &mut *tx,
        id(&request),
        "provider",
        Some(provider_id),
        "create",
        None,
        Some(1),
        AuditOutcome::Success,
        1,
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
    match provider_by(pool, &provider_key).await {
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
    let pool = match db(&state) {
        Ok(v) => v,
        Err(e) => return e,
    };
    match provider_by(pool, &key).await {
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
    let pool = match db(&state) {
        Ok(v) => v,
        Err(e) => return e,
    };
    let order = match query.sort.as_deref().unwrap_or("key") {
        "key" => "key ASC",
        "-key" => "key DESC",
        "name" => "name ASC, key ASC",
        "-name" => "name DESC, key ASC",
        _ => return error(&request, StatusCode::BAD_REQUEST, "invalid_query"),
    };
    let filters = match ProviderFilters::from_query(&query) {
        Ok(value) => value,
        Err(()) => return error(&request, StatusCode::BAD_REQUEST, "invalid_query"),
    };
    let total = match provider_count(pool, &filters, true).await {
        Ok(value) => value,
        Err(value) => return sql_error(&request, &value),
    };
    let facets = match provider_facets(pool, &filters).await {
        Ok(value) => value,
        Err(value) => return sql_error(&request, &value),
    };
    let mut builder = QueryBuilder::<Sqlite>::new(
        "SELECT id,key,name,type AS kind,adapter,config_json,enabled,revision,created_at,updated_at,secret_ref IS NOT NULL AS has_secret_ref FROM providers",
    );
    append_provider_filters(&mut builder, &filters, true);
    builder
        .push(" ORDER BY ")
        .push(order)
        .push(" LIMIT ")
        .push_bind(i64::from(size))
        .push(" OFFSET ")
        .push_bind(i64::from((page - 1) * size));
    match builder.build_query_as::<Provider>().fetch_all(pool).await { Ok(items) => Json(serde_json::json!({"items":items.into_iter().map(|item| managed_provider_response(item, &state)).collect::<Vec<_>>(),"page":page,"page_size":size,"max_page_size":PAGE_MAX,"total":total,"total_pages":provider_total_pages(total, size),"facets":facets})).into_response(), Err(value) => sql_error(&request, &value) }
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
struct ProviderFilters {
    enabled: Option<bool>,
    q: Option<String>,
    kind: Option<String>,
}
impl ProviderFilters {
    fn from_query(query: &ProviderListQuery) -> Result<Self, ()> {
        let kind = query.kind.clone();
        if kind
            .as_deref()
            .is_some_and(|value| !matches!(value, "vad" | "asr" | "llm" | "tts" | "speaker"))
        {
            return Err(());
        }
        let q = query.q.clone().filter(|value| !value.is_empty());
        if q.as_ref().is_some_and(|value| value.len() > 128) {
            return Err(());
        }
        Ok(Self {
            enabled: query.enabled,
            q,
            kind,
        })
    }
}
fn append_provider_filters(
    builder: &mut QueryBuilder<Sqlite>,
    filters: &ProviderFilters,
    include_kind: bool,
) {
    let mut first = true;
    let mut clause = |builder: &mut QueryBuilder<Sqlite>| {
        builder.push(if first { " WHERE " } else { " AND " });
        first = false;
    };
    if let Some(enabled) = filters.enabled {
        clause(builder);
        builder.push("enabled=").push_bind(i64::from(enabled));
    }
    if include_kind && let Some(kind) = &filters.kind {
        clause(builder);
        builder.push("type=").push_bind(kind.clone());
    }
    if let Some(q) = &filters.q {
        clause(builder);
        let q = format!(
            "%{}%",
            q.replace('\\', "\\\\")
                .replace('%', "\\%")
                .replace('_', "\\_")
        );
        builder
            .push("(key LIKE ")
            .push_bind(q.clone())
            .push(" ESCAPE '\\' OR name LIKE ")
            .push_bind(q)
            .push(" ESCAPE '\\')");
    }
}
async fn provider_count(
    pool: &SqlitePool,
    filters: &ProviderFilters,
    include_kind: bool,
) -> Result<i64, sqlx::Error> {
    let mut builder = QueryBuilder::<Sqlite>::new("SELECT COUNT(*) FROM providers");
    append_provider_filters(&mut builder, filters, include_kind);
    builder.build_query_scalar().fetch_one(pool).await
}
async fn provider_facets(
    pool: &SqlitePool,
    filters: &ProviderFilters,
) -> Result<Value, sqlx::Error> {
    let mut builder = QueryBuilder::<Sqlite>::new("SELECT type, COUNT(*) FROM providers");
    append_provider_filters(&mut builder, filters, false);
    builder.push(" GROUP BY type");
    let counts: Vec<(String, i64)> = builder.build_query_as().fetch_all(pool).await?;
    let mut facets = serde_json::Map::new();
    for kind in ["vad", "asr", "llm", "tts", "speaker"] {
        facets.insert(
            kind.into(),
            Value::from(
                counts
                    .iter()
                    .find(|(value, _)| value == kind)
                    .map(|(_, count)| *count)
                    .unwrap_or(0),
            ),
        );
    }
    Ok(Value::Object(facets))
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
    let pool = match db(&state) {
        Ok(v) => v,
        Err(e) => return e,
    };
    let provider = match provider_by(pool, &provider_key).await {
        Ok(v) => v,
        Err(sqlx::Error::RowNotFound) => {
            return error(&request, StatusCode::NOT_FOUND, "not_found");
        }
        Err(e) => return sql_error(&request, &e),
    };
    let total: i64 = match sqlx::query_scalar(
        "SELECT COUNT(*) FROM template_provider_bindings WHERE provider_id=?",
    )
    .bind(provider.id)
    .fetch_one(pool)
    .await
    {
        Ok(value) => value,
        Err(e) => return sql_error(&request, &e),
    };
    let rows: Result<Vec<(String, String, String, i64)>, _> = sqlx::query_as(
        "SELECT t.key,t.name,b.provider_type,t.enabled FROM template_provider_bindings b \
         JOIN agent_templates t ON t.id=b.template_id WHERE b.provider_id=? ORDER BY t.key LIMIT ? OFFSET ?",
    )
    .bind(provider.id)
    .bind(i64::from(page_size))
    .bind(i64::from((page - 1) * page_size))
    .fetch_all(pool)
    .await;
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
    let pool = match db(&state) {
        Ok(v) => v,
        Err(e) => return e,
    };
    type ProviderRow = (
        i64,
        String,
        String,
        String,
        String,
        Option<String>,
        i64,
        i64,
    );
    let old: Result<ProviderRow, _> =
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
    if kind == "speaker" && secret.is_some() {
        return error(&request, StatusCode::BAD_REQUEST, "provider_config_invalid");
    }
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
        Some(provider_id),
        "update",
        Some(expected),
        Some(expected + 1),
        AuditOutcome::Success,
        1,
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
    if let Some(prewarm) = &state.provider_prewarm {
        prewarm.provider(provider_id, expected + 1).await;
    }
    let _ = kind; // type is immutable and retained for the provider instance.
    match provider_by(pool, &key).await {
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
