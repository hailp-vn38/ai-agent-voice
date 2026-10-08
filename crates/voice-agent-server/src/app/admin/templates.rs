//! Admin templates resources.
use super::agents::get_agent_by;
use super::*;
use sqlx::QueryBuilder;

mod relationships;

pub(super) use relationships::{
    list_agent_templates, list_template_agents, list_template_providers, unlink_agent_template,
    unlink_template_provider,
};

#[derive(Serialize, FromRow)]
struct Template {
    id: i64,
    key: String,
    name: String,
    description: Option<String>,
    language: String,
    prompt: String,
    enabled: i64,
    revision: i64,
    created_at: i64,
    updated_at: i64,
}
#[derive(Deserialize)]
struct CreateTemplate {
    key: String,
    name: String,
    #[serde(default)]
    description: Option<String>,
    language: String,
    prompt: String,
}
#[derive(Deserialize)]
struct PatchTemplate {
    #[serde(default)]
    key: Patch<String>,
    #[serde(default)]
    name: Patch<String>,
    #[serde(default)]
    description: Patch<String>,
    #[serde(default)]
    language: Patch<String>,
    #[serde(default)]
    prompt: Patch<String>,
    #[serde(default)]
    enabled: Patch<bool>,
}
#[derive(Deserialize)]
struct ProviderBinding {
    provider_key: String,
}

async fn template_by(pool: &SqlitePool, key: &str) -> Result<Template, sqlx::Error> {
    sqlx::query_as("SELECT id,key,name,description,language,prompt,enabled,revision,created_at,updated_at FROM agent_templates WHERE key=?").bind(key).fetch_one(pool).await
}
pub(super) async fn create_template(State(state): State<AppState>, request: Request) -> Response {
    let (request, body): (_, CreateTemplate) = match json(request).await {
        Ok(v) => v,
        Err(e) => return e,
    };
    if !valid_key(&body.key)
        || !valid_text(&body.name, 128, false)
        || !valid_text(&body.language, 32, false)
        || !valid_text(&body.prompt, 64 * 1024, false)
        || body
            .description
            .as_ref()
            .is_some_and(|v| !valid_text(v, 2048, true))
    {
        return error(&request, StatusCode::BAD_REQUEST, "validation_failed");
    }
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
    let time = now();
    let result = sqlx::query("INSERT INTO agent_templates (key,name,description,language,prompt,created_at,updated_at) VALUES (?,?,?,?,?,?,?)").bind(&body.key).bind(&body.name).bind(&body.description).bind(&body.language).bind(&body.prompt).bind(time).bind(time).execute(&mut *tx).await;
    let resource_id = match result {
        Ok(v) => v.last_insert_rowid(),
        Err(e) => return mutation_sql_error(&request, &e),
    };
    if audit(
        &mut *tx,
        id(&request),
        "template",
        Some(resource_id),
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
    }
    match template_by(pool, &body.key).await {
        Ok(v) => (StatusCode::CREATED, Json(v)).into_response(),
        Err(e) => sql_error(&request, &e),
    }
}
pub(super) async fn get_template(
    State(state): State<AppState>,
    Path(key): Path<String>,
    request: Request,
) -> Response {
    let pool = match db(&state) {
        Ok(v) => v,
        Err(e) => return e,
    };
    match template_by(pool, &key).await {
        Ok(v) => Json(v).into_response(),
        Err(sqlx::Error::RowNotFound) => error(&request, StatusCode::NOT_FOUND, "not_found"),
        Err(e) => sql_error(&request, &e),
    }
}

pub(super) async fn list_templates(
    State(state): State<AppState>,
    Query(query): Query<TemplateListQuery>,
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
        "language" => "language ASC, key ASC",
        _ => return error(&request, StatusCode::BAD_REQUEST, "invalid_query"),
    };
    let filters = match TemplateFilters::from_query(&query) {
        Ok(value) => value,
        Err(()) => return error(&request, StatusCode::BAD_REQUEST, "invalid_query"),
    };
    let total = match template_count(pool, &filters).await {
        Ok(value) => value,
        Err(value) => return sql_error(&request, &value),
    };
    let mut builder = QueryBuilder::<Sqlite>::new(
        "SELECT id,key,name,description,language,prompt,enabled,revision,created_at,updated_at FROM agent_templates",
    );
    append_template_filters(&mut builder, &filters);
    builder
        .push(" ORDER BY ")
        .push(order)
        .push(" LIMIT ")
        .push_bind(i64::from(size))
        .push(" OFFSET ")
        .push_bind(i64::from((page - 1) * size));
    match builder.build_query_as::<Template>().fetch_all(pool).await { Ok(items) => Json(serde_json::json!({"items":items,"page":page,"page_size":size,"max_page_size":PAGE_MAX,"total":total,"total_pages":template_total_pages(total, size)})).into_response(), Err(value) => sql_error(&request, &value) }
}

#[derive(Deserialize)]
pub(super) struct TemplateListQuery {
    #[serde(default)]
    page: Option<u32>,
    #[serde(default)]
    page_size: Option<u32>,
    #[serde(default)]
    enabled: Option<bool>,
    #[serde(default)]
    q: Option<String>,
    #[serde(default)]
    language: Option<String>,
    #[serde(default)]
    sort: Option<String>,
}
struct TemplateFilters {
    enabled: Option<bool>,
    q: Option<String>,
    language: Option<String>,
}
impl TemplateFilters {
    fn from_query(query: &TemplateListQuery) -> Result<Self, ()> {
        let q = query.q.clone().filter(|value| !value.is_empty());
        let language = query.language.clone().filter(|value| !value.is_empty());
        if q.as_ref().is_some_and(|value| value.len() > 128)
            || language.as_ref().is_some_and(|value| value.len() > 32)
        {
            return Err(());
        }
        Ok(Self {
            enabled: query.enabled,
            q,
            language,
        })
    }
}
fn append_template_filters(builder: &mut QueryBuilder<Sqlite>, filters: &TemplateFilters) {
    let mut first = true;
    let mut clause = |builder: &mut QueryBuilder<Sqlite>| {
        builder.push(if first { " WHERE " } else { " AND " });
        first = false;
    };
    if let Some(enabled) = filters.enabled {
        clause(builder);
        builder.push("enabled=").push_bind(i64::from(enabled));
    }
    if let Some(language) = &filters.language {
        clause(builder);
        builder.push("language=").push_bind(language.clone());
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
            .push_bind(q.clone())
            .push(" ESCAPE '\\' OR description LIKE ")
            .push_bind(q)
            .push(" ESCAPE '\\')");
    }
}
async fn template_count(pool: &SqlitePool, filters: &TemplateFilters) -> Result<i64, sqlx::Error> {
    let mut builder = QueryBuilder::<Sqlite>::new("SELECT COUNT(*) FROM agent_templates");
    append_template_filters(&mut builder, filters);
    builder.build_query_scalar().fetch_one(pool).await
}
fn template_total_pages(total: i64, page_size: u32) -> i64 {
    (total + i64::from(page_size) - 1) / i64::from(page_size)
}
pub(super) async fn patch_template(
    State(state): State<AppState>,
    Path(key): Path<String>,
    request: Request,
) -> Response {
    let (request, body): (_, PatchTemplate) = match json(request).await {
        Ok(v) => v,
        Err(e) => return e,
    };
    if !matches!(body.key, Patch::Absent) {
        return error(&request, StatusCode::BAD_REQUEST, "immutable_field");
    }
    let expected = match expected(request.headers()) {
        Ok(v) => v,
        Err(c) => return error(&request, StatusCode::BAD_REQUEST, c),
    };
    let pool = match db(&state) {
        Ok(v) => v,
        Err(e) => return e,
    };
    let old = match template_by(pool, &key).await {
        Ok(v) => v,
        Err(sqlx::Error::RowNotFound) => {
            return error(&request, StatusCode::NOT_FOUND, "not_found");
        }
        Err(e) => return sql_error(&request, &e),
    };
    if old.revision != expected {
        audit_conflict(pool, id(&request).into(), "template", old.id, expected).await;
        return error(&request, StatusCode::CONFLICT, "revision_conflict");
    }
    let name = match body.name.value() {
        Some(Some(v)) if valid_text(&v, 128, false) => v,
        Some(_) => return error(&request, StatusCode::BAD_REQUEST, "validation_failed"),
        None => old.name,
    };
    let description = body.description.value().unwrap_or(old.description);
    let language = match body.language.value() {
        Some(Some(v)) if valid_text(&v, 32, false) => v,
        Some(_) => return error(&request, StatusCode::BAD_REQUEST, "validation_failed"),
        None => old.language,
    };
    let prompt = match body.prompt.value() {
        Some(Some(v)) if valid_text(&v, 64 * 1024, false) => v,
        Some(_) => return error(&request, StatusCode::BAD_REQUEST, "validation_failed"),
        None => old.prompt,
    };
    let enabled = match body.enabled.value() {
        Some(Some(v)) => i64::from(v),
        Some(None) => return error(&request, StatusCode::BAD_REQUEST, "validation_failed"),
        None => old.enabled,
    };
    if description
        .as_ref()
        .is_some_and(|v| !valid_text(v, 2048, true))
    {
        return error(&request, StatusCode::BAD_REQUEST, "validation_failed");
    }
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
    let updated = sqlx::query("UPDATE agent_templates SET name=?,description=?,language=?,prompt=?,enabled=?,revision=revision+1,updated_at=? WHERE id=? AND revision=?").bind(name).bind(description).bind(language).bind(prompt).bind(enabled).bind(now()).bind(old.id).bind(expected).execute(&mut *tx).await.map(|v| v.rows_affected()==1).unwrap_or(false);
    if !updated {
        let _ = tx.rollback().await;
        return error(&request, StatusCode::CONFLICT, "revision_conflict");
    }
    if audit(
        &mut *tx,
        id(&request),
        "template",
        Some(old.id),
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
    // The Template revision is part of the session's frozen snapshot; drop sessions pinned to it
    // so the next admission observes the new prompt/language/enabled state instead of hot-reloading.
    if let Some(security) = security(&state) {
        security.invalidate_template_speakers(old.id);
    }
    get_template(State(state), Path(key), request).await
}

pub(super) async fn bind_template_provider(
    State(state): State<AppState>,
    Path((key, provider_type)): Path<(String, String)>,
    request: Request,
) -> Response {
    let (request, body): (_, ProviderBinding) = match json(request).await {
        Ok(v) => v,
        Err(e) => return e,
    };
    if !matches!(
        provider_type.as_str(),
        "vad" | "asr" | "llm" | "tts" | "speaker"
    ) {
        return error(&request, StatusCode::BAD_REQUEST, "validation_failed");
    }
    let expected = match expected(request.headers()) {
        Ok(v) => v,
        Err(c) => return error(&request, StatusCode::BAD_REQUEST, c),
    };
    let pool = match db(&state) {
        Ok(v) => v,
        Err(e) => return e,
    };
    let template = match template_by(pool, &key).await {
        Ok(v) => v,
        Err(sqlx::Error::RowNotFound) => {
            return error(&request, StatusCode::NOT_FOUND, "not_found");
        }
        Err(e) => return sql_error(&request, &e),
    };
    if template.revision != expected {
        return error(&request, StatusCode::CONFLICT, "revision_conflict");
    };
    let provider: Result<(i64, String, i64), _> =
        sqlx::query_as("SELECT id,type,enabled FROM providers WHERE key=?")
            .bind(&body.provider_key)
            .fetch_one(pool)
            .await;
    let (provider_id, provider_kind, enabled) = match provider {
        Ok(v) => v,
        Err(_) => return error(&request, StatusCode::BAD_REQUEST, "invalid_provider"),
    };
    if provider_kind != provider_type || enabled != 1 {
        return error(&request, StatusCode::BAD_REQUEST, "invalid_provider");
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
    if sqlx::query("INSERT INTO template_provider_bindings (template_id,provider_type,provider_id,created_at,updated_at) VALUES (?,?,?,?,?) ON CONFLICT(template_id,provider_type) DO UPDATE SET provider_id=excluded.provider_id,updated_at=excluded.updated_at").bind(template.id).bind(&provider_type).bind(provider_id).bind(now()).bind(now()).execute(&mut *tx).await.is_err() || sqlx::query("UPDATE agent_templates SET revision=revision+1,updated_at=? WHERE id=? AND revision=?").bind(now()).bind(template.id).bind(expected).execute(&mut *tx).await.map(|v|v.rows_affected()!=1).unwrap_or(true) || audit(&mut *tx,id(&request),"template",Some(template.id),"bind_provider",Some(expected),Some(expected+1),AuditOutcome::Success, 1).await.is_err() || tx.commit().await.is_err(){return error(&request,StatusCode::SERVICE_UNAVAILABLE,"database_unavailable")};
    if let Some(prewarm) = &state.provider_prewarm {
        prewarm.template(template.id).await;
    }
    match template_by(pool, &key).await {
        Ok(v) => Json(v).into_response(),
        Err(e) => sql_error(&request, &e),
    }
}

pub(super) async fn set_default_template(
    State(state): State<AppState>,
    Path((agent_key, template_key)): Path<(String, String)>,
    request: Request,
) -> Response {
    let expected = match expected(request.headers()) {
        Ok(v) => v,
        Err(c) => return error(&request, StatusCode::BAD_REQUEST, c),
    };
    let pool = match db(&state) {
        Ok(v) => v,
        Err(e) => return e,
    };
    let agent = match get_agent_by(pool, &agent_key).await {
        Ok(v) => v,
        Err(sqlx::Error::RowNotFound) => {
            return error(&request, StatusCode::NOT_FOUND, "not_found");
        }
        Err(e) => return sql_error(&request, &e),
    };
    if agent.revision != expected {
        return error(&request, StatusCode::CONFLICT, "revision_conflict");
    };
    let template = match template_by(pool, &template_key).await {
        Ok(v) if v.enabled == 1 => v,
        _ => return error(&request, StatusCode::BAD_REQUEST, "invalid_template"),
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
    let time = now();
    if sqlx::query("UPDATE agent_template_assignments SET is_default=0 WHERE agent_id=? AND enabled=1").bind(agent.id).execute(&mut *tx).await.is_err()||sqlx::query("INSERT INTO agent_template_assignments(agent_id,template_id,is_default,enabled,created_at) VALUES (?,?,1,1,?) ON CONFLICT(agent_id,template_id) DO UPDATE SET is_default=1,enabled=1").bind(agent.id).bind(template.id).bind(time).execute(&mut *tx).await.is_err()||sqlx::query("UPDATE agents SET revision=revision+1,updated_at=? WHERE id=? AND revision=?").bind(time).bind(agent.id).bind(expected).execute(&mut *tx).await.map(|v|v.rows_affected()!=1).unwrap_or(true)||audit(&mut *tx,id(&request),"agent",Some(agent.id),"set_default_template",Some(expected),Some(expected+1),AuditOutcome::Success, 1).await.is_err()||tx.commit().await.is_err(){return error(&request,StatusCode::SERVICE_UNAVAILABLE,"database_unavailable")};
    if let Some(prewarm) = &state.provider_prewarm {
        prewarm.template(template.id).await;
    }
    match get_agent_by(pool, &agent_key).await {
        Ok(v) => Json(v).into_response(),
        Err(e) => sql_error(&request, &e),
    }
}

pub(super) async fn assign_template(
    State(state): State<AppState>,
    Path((agent_key, template_key)): Path<(String, String)>,
    request: Request,
) -> Response {
    let expected = match expected(request.headers()) {
        Ok(v) => v,
        Err(c) => return error(&request, StatusCode::BAD_REQUEST, c),
    };
    let pool = match db(&state) {
        Ok(v) => v,
        Err(e) => return e,
    };
    let agent = match get_agent_by(pool, &agent_key).await {
        Ok(v) => v,
        Err(sqlx::Error::RowNotFound) => {
            return error(&request, StatusCode::NOT_FOUND, "not_found");
        }
        Err(e) => return sql_error(&request, &e),
    };
    if agent.revision != expected {
        audit_conflict(pool, id(&request).into(), "agent", agent.id, expected).await;
        return error(&request, StatusCode::CONFLICT, "revision_conflict");
    }
    let template = match template_by(pool, &template_key).await {
        Ok(v) if v.enabled == 1 => v,
        _ => return error(&request, StatusCode::BAD_REQUEST, "invalid_template"),
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
    // An Agent inside the Template mechanism has no path back to server defaults, so leaving it
    // with assignments but no enabled default refuses every one of its devices with a 503. The
    // first assignment therefore takes the default slot whenever it can serve as one: an absent
    // core slot falls back to the deployment default, so only an explicit *broken* core binding
    // (disabled provider, or a provider whose type no longer matches the slot) makes the Template
    // structurally incomplete. The optional Speaker slot is never counted here, so binding a
    // Speaker provider cannot mask a missing core slot. A failed probe promotes anyway, because
    // the invariant matters more than the optimistic read.
    let broken_core: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM template_provider_bindings b JOIN providers p ON p.id=b.provider_id WHERE b.template_id=? AND b.provider_type IN ('vad','asr','llm','tts') AND (p.enabled!=1 OR b.provider_type!=p.type)")
        .bind(template.id).fetch_one(&mut *tx).await.unwrap_or(0);
    let promote: i64 = if broken_core == 0 {
        sqlx::query_scalar::<_, i64>("SELECT NOT EXISTS(SELECT 1 FROM agent_template_assignments WHERE agent_id=? AND enabled=1 AND is_default=1)")
            .bind(agent.id).fetch_one(&mut *tx).await.unwrap_or(1)
    } else {
        0
    };
    let mutated = sqlx::query("INSERT INTO agent_template_assignments(agent_id,template_id,is_default,enabled,created_at) VALUES (?,?,?,1,?) ON CONFLICT(agent_id,template_id) DO UPDATE SET enabled=1,is_default=MAX(is_default,excluded.is_default)")
        .bind(agent.id).bind(template.id).bind(promote).bind(now()).execute(&mut *tx).await.is_ok()
        && sqlx::query("UPDATE agents SET revision=revision+1,updated_at=? WHERE id=? AND revision=?").bind(now()).bind(agent.id).bind(expected).execute(&mut *tx).await.map(|v| v.rows_affected() == 1).unwrap_or(false);
    if !mutated {
        let _ = tx.rollback().await;
        audit_conflict(pool, id(&request).into(), "agent", agent.id, expected).await;
        return error(&request, StatusCode::CONFLICT, "revision_conflict");
    }
    if audit(
        &mut *tx,
        id(&request),
        "agent",
        Some(agent.id),
        "assign_template",
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
    match get_agent_by(pool, &agent_key).await {
        Ok(v) => Json(v).into_response(),
        Err(e) => sql_error(&request, &e),
    }
}
