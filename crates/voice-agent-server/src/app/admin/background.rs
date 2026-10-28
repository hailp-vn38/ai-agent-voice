use super::*;
use crate::services::task_flow::{
    AgentConfigOverride, CreateCustomSubagentInput, DEFAULT_ORCHESTRATOR_PROVIDER_REF,
    PersistedBackgroundSettings, SubagentMutationError, create_custom_subagent,
    delete_custom_subagent, find_agent_spec, project_agents_json, project_artifact_catalog_page,
    project_task_events, project_task_to_admin_job, resolve_llm_provider_catalog,
    save_persisted_settings,
    skills::{self, Precondition, SaveJobSkill, SkillCatalogError},
    sync_orchestrator_from_state, validate_agent_overrides_with_skills,
};
use orchestrator_flow_rs::{
    CreateTaskRequest, ErrorCode, TaskArtifactCatalogPage, TaskArtifactFilter, TaskArtifactKind,
    TaskArtifactRole, TaskError, TaskEvent, TaskFeedItem, TaskFilter, TaskManager, TaskSource,
    TaskStatus,
};
use serde::Deserialize;
use std::collections::BTreeMap;
use std::sync::Arc;

#[allow(clippy::result_large_err)]
fn task_manager(state: &AppState, req: &Request) -> Result<Arc<TaskManager>, Response> {
    state
        .task_manager
        .as_ref()
        .cloned()
        .ok_or_else(|| error(req, StatusCode::SERVICE_UNAVAILABLE, "runtime_unavailable"))
}

#[allow(clippy::result_large_err)]
fn task_manager_for_submit(state: &AppState, req: &Request) -> Result<Arc<TaskManager>, Response> {
    let disabled = state
        .admin_task_control
        .lock()
        .expect("admin_task_control mutex poisoned")
        .background_enabled_override
        == Some(false);
    if disabled {
        return Err(error(
            req,
            StatusCode::SERVICE_UNAVAILABLE,
            "runtime_unavailable",
        ));
    }
    task_manager(state, req)
}

fn task_failure(request: &Request, cause: TaskError) -> Response {
    let status = match cause.code() {
        ErrorCode::InvalidInput | ErrorCode::InvalidConfig => StatusCode::BAD_REQUEST,
        ErrorCode::NotFound => StatusCode::NOT_FOUND,
        ErrorCode::PermissionDenied => StatusCode::FORBIDDEN,
        ErrorCode::StoreAlreadyOwned => StatusCode::CONFLICT,
        ErrorCode::NotReady | ErrorCode::ShuttingDown => StatusCode::SERVICE_UNAVAILABLE,
        ErrorCode::StoreError | ErrorCode::OrchestratorError => StatusCode::SERVICE_UNAVAILABLE,
    };
    error(request, status, cause.code().as_str())
}

async fn build_settings_json(
    state: &AppState,
    settings: &PersistedBackgroundSettings,
    revision: i64,
) -> Value {
    let runtime_ready = state.task_manager.is_some();
    let enabled = settings
        .enabled
        .unwrap_or(state.config.orchestrator.enabled && runtime_ready);
    let catalog = resolve_llm_provider_catalog(state).await;
    let valid_ref = |pref: &str| -> String {
        catalog
            .iter()
            .find(|p| p.provider_ref == pref && p.supports_tag("chat"))
            .map(|p| p.provider_ref.clone())
            .unwrap_or_else(|| DEFAULT_ORCHESTRATOR_PROVIDER_REF.to_owned())
    };
    let orchestrator_llm_provider = valid_ref(&settings.orchestrator_llm_provider);
    let worker_llm_provider = valid_ref(&settings.worker_llm_provider);
    let llm_providers: Vec<Value> = catalog
        .iter()
        .map(|p| p.to_json_summary())
        .collect();
    let agents = project_agents_json(&state.config.orchestrator, settings, &catalog);
    let skills_list = skills::list(state).unwrap_or_default();
    let available_agents = state
        .task_manager
        .as_ref()
        .map(|tm| tm.available_agents())
        .unwrap_or_default();
    serde_json::json!({
        "runtime_ready": runtime_ready,
        "enabled": enabled,
        "orchestrator_llm_provider": orchestrator_llm_provider,
        "worker_llm_provider": worker_llm_provider,
        "revision": revision,
        "llm_providers": llm_providers,
        "available_agents": available_agents,
        "agents": agents,
        "skills": skills_list,
    })
}

pub(super) async fn settings(State(state): State<AppState>, _request: Request) -> Response {
    let (settings, revision) = {
        let guard = state
            .admin_task_control
            .lock()
            .expect("admin_task_control mutex poisoned");
        (guard.background_settings.clone(), guard.background_revision)
    };
    Json(build_settings_json(&state, &settings, revision).await).into_response()
}

#[derive(Deserialize)]
pub(super) struct SettingsUpdateInput {
    pub enabled: bool,
    #[serde(default)]
    pub orchestrator_llm_provider: Option<String>,
    #[serde(default)]
    pub worker_llm_provider: Option<String>,
    #[serde(default)]
    pub agents: Option<BTreeMap<String, AgentConfigOverride>>,
}

pub(super) async fn save_settings(State(state): State<AppState>, request: Request) -> Response {
    let expected_rev = match expected(request.headers()) {
        Ok(v) => v,
        Err(e) => return error(&request, StatusCode::BAD_REQUEST, e),
    };
    let (request, input): (_, SettingsUpdateInput) = match json(request).await {
        Ok(v) => v,
        Err(e) => return e,
    };

    let catalog = resolve_llm_provider_catalog(&state).await;
    let current = {
        let guard = state
            .admin_task_control
            .lock()
            .expect("admin_task_control mutex poisoned");
        if guard.background_revision != expected_rev {
            return error(&request, StatusCode::CONFLICT, "revision_conflict");
        }
        guard.background_settings.clone()
    };

    let next_orchestrator_provider = input
        .orchestrator_llm_provider
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .unwrap_or_else(|| current.orchestrator_llm_provider.clone());
    let next_worker_provider = input
        .worker_llm_provider
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .unwrap_or_else(|| current.worker_llm_provider.clone());
    let skill_ids = skills::catalog_ids(&state);
    let mut pruned_current_agents = current.agents.clone();
    for ovr in pruned_current_agents.values_mut() {
        if let Some(ref mut allowed) = ovr.allowed_skills {
            allowed.retain(|s| skill_ids.contains(s.as_str()));
        }
    }
    let next_agents = match input.agents {
        Some(incoming) => {
            let mut merged = pruned_current_agents;
            for (k, mut v) in incoming {
                if find_agent_spec(&k).is_none()
                    && let Some(prev) = merged.get(&k)
                {
                    v.merge_from_existing_custom(prev);
                }
                merged.insert(k, v);
            }
            merged
        }
        None => pruned_current_agents,
    };

    if let Err(code) = validate_agent_overrides_with_skills(
        &next_orchestrator_provider,
        &next_worker_provider,
        &next_agents,
        &catalog,
        &skill_ids,
    ) {
        return error(&request, StatusCode::BAD_REQUEST, code);
    }

    let mut next_settings = PersistedBackgroundSettings {
        enabled: Some(input.enabled),
        revision: expected_rev.saturating_add(1),
        orchestrator_llm_provider: next_orchestrator_provider,
        worker_llm_provider: next_worker_provider,
        agents: next_agents,
    };

    let new_rev = {
        let mut guard = state
            .admin_task_control
            .lock()
            .expect("admin_task_control mutex poisoned");
        if guard.background_revision != expected_rev {
            return error(&request, StatusCode::CONFLICT, "revision_conflict");
        }
        guard.background_enabled_override = Some(input.enabled);
        guard.background_revision = guard.background_revision.saturating_add(1);
        next_settings.revision = guard.background_revision;
        guard.background_settings = next_settings.clone();
        guard.background_revision
    };

    let _ = save_persisted_settings(&state.config.orchestrator.store_dir, &next_settings);
    if let Err(err) = sync_orchestrator_from_state(&state).await {
        tracing::warn!(error = %err, "failed to reconfigure orchestrator after background settings update");
    }

    Json(build_settings_json(&state, &next_settings, new_rev).await).into_response()
}

fn subagent_error_response(request: &Request, err: SubagentMutationError) -> Response {
    match err {
        SubagentMutationError::InvalidId => {
            error(request, StatusCode::BAD_REQUEST, "invalid_agent_id")
        }
        SubagentMutationError::InvalidFlowId => {
            error(request, StatusCode::BAD_REQUEST, "invalid_arguments")
        }
        SubagentMutationError::InvalidProvider => {
            error(request, StatusCode::BAD_REQUEST, "invalid_provider")
        }
        SubagentMutationError::ProviderCapabilityMismatch => error(
            request,
            StatusCode::BAD_REQUEST,
            "provider_capability_mismatch",
        ),
        SubagentMutationError::InvalidArguments(code) => {
            error(request, StatusCode::BAD_REQUEST, code)
        }
        SubagentMutationError::AlreadyExists => {
            error(request, StatusCode::CONFLICT, "agent_already_exists")
        }
        SubagentMutationError::NotFound => error(request, StatusCode::NOT_FOUND, "not_found"),
        SubagentMutationError::BuiltinCannotBeDeleted => error(
            request,
            StatusCode::BAD_REQUEST,
            "builtin_agent_cannot_be_deleted",
        ),
        SubagentMutationError::RevisionConflict => {
            error(request, StatusCode::CONFLICT, "revision_conflict")
        }
        SubagentMutationError::Io => error(
            request,
            StatusCode::SERVICE_UNAVAILABLE,
            "storage_unavailable",
        ),
    }
}

pub(super) async fn create_agent(State(state): State<AppState>, request: Request) -> Response {
    let expected_rev = match expected(request.headers()) {
        Ok(v) => v,
        Err(e) => return error(&request, StatusCode::BAD_REQUEST, e),
    };
    let (request, input): (_, CreateCustomSubagentInput) = match json(request).await {
        Ok(v) => v,
        Err(e) => return e,
    };
    match create_custom_subagent(&state, expected_rev, input).await {
        Ok(new_rev) => {
            let current = {
                let guard = state
                    .admin_task_control
                    .lock()
                    .expect("admin_task_control mutex poisoned");
                guard.background_settings.clone()
            };
            (
                StatusCode::CREATED,
                Json(build_settings_json(&state, &current, new_rev).await),
            )
                .into_response()
        }
        Err(err) => subagent_error_response(&request, err),
    }
}

pub(super) async fn delete_agent(
    State(state): State<AppState>,
    Path(id): Path<String>,
    request: Request,
) -> Response {
    let expected_rev = match expected(request.headers()) {
        Ok(v) => v,
        Err(e) => return error(&request, StatusCode::BAD_REQUEST, e),
    };
    match delete_custom_subagent(&state, expected_rev, &id).await {
        Ok(new_rev) => {
            let current = {
                let guard = state
                    .admin_task_control
                    .lock()
                    .expect("admin_task_control mutex poisoned");
                guard.background_settings.clone()
            };
            Json(build_settings_json(&state, &current, new_rev).await).into_response()
        }
        Err(err) => subagent_error_response(&request, err),
    }
}

fn skill_error_response(request: &Request, err: SkillCatalogError) -> Response {
    match err {
        SkillCatalogError::InvalidId => error(request, StatusCode::BAD_REQUEST, "invalid_skill_id"),
        SkillCatalogError::InvalidArguments(code) => error(request, StatusCode::BAD_REQUEST, code),
        SkillCatalogError::PayloadTooLarge => {
            error(request, StatusCode::PAYLOAD_TOO_LARGE, "skill_too_large")
        }
        SkillCatalogError::NotFound => error(request, StatusCode::NOT_FOUND, "not_found"),
        SkillCatalogError::AlreadyExists => {
            error(request, StatusCode::CONFLICT, "already_exists")
        }
        SkillCatalogError::RevisionConflict => {
            error(request, StatusCode::CONFLICT, "revision_conflict")
        }
        SkillCatalogError::BuiltinCannotBeDeleted => error(
            request,
            StatusCode::BAD_REQUEST,
            "builtin_skill_cannot_be_deleted",
        ),
        SkillCatalogError::Io => error(
            request,
            StatusCode::SERVICE_UNAVAILABLE,
            "storage_unavailable",
        ),
    }
}

fn parse_skill_etag(headers: &HeaderMap) -> Option<String> {
    let raw = headers.get(header::IF_MATCH)?.to_str().ok()?.trim();
    let unquoted = raw
        .strip_prefix('"')
        .and_then(|s| s.strip_suffix('"'))
        .unwrap_or(raw)
        .trim();
    (!unquoted.is_empty()).then(|| unquoted.to_owned())
}

fn parse_save_precondition(headers: &HeaderMap) -> Result<Precondition, &'static str> {
    if let Some(if_none) = headers
        .get(header::IF_NONE_MATCH)
        .and_then(|v| v.to_str().ok())
        .map(str::trim)
        && if_none == "*"
    {
        return Ok(Precondition::IfNoneMatchStar);
    }
    if let Some(etag) = parse_skill_etag(headers) {
        return Ok(Precondition::IfMatch(etag));
    }
    Err("invalid_if_match")
}

pub(super) async fn get_skill(
    State(state): State<AppState>,
    Path(id): Path<String>,
    request: Request,
) -> Response {
    match skills::get(&state, &id) {
        Ok(detail) => {
            let etag = format!("\"{}\"", detail.summary.etag);
            (
                StatusCode::OK,
                [(header::ETAG, etag)],
                Json(detail),
            )
                .into_response()
        }
        Err(err) => skill_error_response(&request, err),
    }
}

pub(super) async fn save_skill(
    State(state): State<AppState>,
    Path(id): Path<String>,
    request: Request,
) -> Response {
    let precondition = match parse_save_precondition(request.headers()) {
        Ok(p) => p,
        Err(code) => return error(&request, StatusCode::BAD_REQUEST, code),
    };
    let (request, input): (_, SaveJobSkill) = match json(request).await {
        Ok(v) => v,
        Err(resp) => return resp,
    };
    match skills::save(&state, &id, input, precondition).await {
        Ok((detail, created)) => {
            let status = if created {
                StatusCode::CREATED
            } else {
                StatusCode::OK
            };
            let etag = format!("\"{}\"", detail.summary.etag);
            (status, [(header::ETAG, etag)], Json(detail)).into_response()
        }
        Err(err) => skill_error_response(&request, err),
    }
}

pub(super) async fn delete_skill(
    State(state): State<AppState>,
    Path(id): Path<String>,
    request: Request,
) -> Response {
    let Some(expected_etag) = parse_skill_etag(request.headers()) else {
        return error(&request, StatusCode::BAD_REQUEST, "invalid_if_match");
    };
    match skills::delete(&state, &id, &expected_etag).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(err) => skill_error_response(&request, err),
    }
}

#[derive(Deserialize)]
pub(super) struct CreateJobInput {
    pub goal: String,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub target_agent: Option<String>,
    #[serde(default)]
    pub related_wiki_links: Vec<String>,
}

fn extract_idempotency_key(
    headers: &axum::http::HeaderMap,
) -> Result<Option<String>, &'static str> {
    let Some(raw) = headers.get("Idempotency-Key") else {
        return Ok(None);
    };
    let s = raw.to_str().map_err(|_| "invalid_arguments")?.trim();
    if s.is_empty() || s.len() > 128 || !s.is_ascii() {
        return Err("invalid_arguments");
    }
    Ok(Some(s.to_owned()))
}

async fn lookup_idempotent_job(
    state: &AppState,
    tm: &TaskManager,
    key: Option<&str>,
    fingerprint: &str,
) -> Result<Option<Value>, &'static str> {
    let Some(key) = key else {
        return Ok(None);
    };
    let existing = state
        .admin_task_control
        .lock()
        .expect("admin_task_control mutex poisoned")
        .get_idempotent(key);
    let Some((saved_fp, task_id)) = existing else {
        return Ok(None);
    };
    if saved_fp != fingerprint {
        return Err("idempotency_key_conflict");
    }
    if let Ok(task) = tm.get_task_any_scope(&task_id).await {
        let detail = tm.get_task_detail_any_scope(&task.id).await.ok();
        return Ok(Some(project_task_to_admin_job(&task, detail.as_ref())));
    }
    Ok(None)
}

fn store_idempotent_job(
    state: &AppState,
    key: Option<String>,
    fingerprint: String,
    task_id: String,
) {
    if let Some(key) = key {
        state
            .admin_task_control
            .lock()
            .expect("admin_task_control mutex poisoned")
            .insert_idempotent(key, fingerprint, task_id);
    }
}

pub(super) async fn create(State(state): State<AppState>, request: Request) -> Response {
    let idempotency_key = match extract_idempotency_key(request.headers()) {
        Ok(v) => v,
        Err(code) => return error(&request, StatusCode::BAD_REQUEST, code),
    };
    let (request, input): (_, CreateJobInput) = match json(request).await {
        Ok(v) => v,
        Err(e) => return e,
    };
    let tm = match task_manager_for_submit(&state, &request) {
        Ok(tm) => tm,
        Err(resp) => return resp,
    };
    let goal = input.goal.trim();
    if goal.is_empty() {
        return error(&request, StatusCode::BAD_REQUEST, "invalid_arguments");
    }
    let title = input
        .title
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .unwrap_or_else(|| goal.chars().take(80).collect());
    let target_agent = input
        .target_agent
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned);
    let fingerprint = format!(
        "create:{title}:{goal}:{}:{:?}",
        target_agent.as_deref().unwrap_or(""),
        input.related_wiki_links
    );

    match lookup_idempotent_job(&state, &tm, idempotency_key.as_deref(), &fingerprint).await {
        Ok(Some(job_json)) => return (StatusCode::OK, Json(job_json)).into_response(),
        Ok(None) => {}
        Err(code) => return error(&request, StatusCode::CONFLICT, code),
    }

    let mut req = CreateTaskRequest::new(title, goal)
        .with_source(TaskSource::Api)
        .with_wiki_links(input.related_wiki_links);
    if let Some(target) = target_agent {
        req = req.with_target(target);
    }

    match tm.create_task(req).await {
        Ok(task) => {
            store_idempotent_job(&state, idempotency_key, fingerprint, task.id.clone());
            let detail = tm.get_task_detail_any_scope(&task.id).await.ok();
            let job_json = project_task_to_admin_job(&task, detail.as_ref());
            (StatusCode::CREATED, Json(job_json)).into_response()
        }
        Err(e) => task_failure(&request, e),
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct JobQuery {
    page: Option<u32>,
    page_size: Option<u32>,
    status: Option<String>,
    q: Option<String>,
}

fn parse_status_filter(status: Option<&str>) -> Option<TaskStatus> {
    match status?.trim() {
        "queued" => Some(TaskStatus::Queued),
        "running" | "planning" | "cancelling" => Some(TaskStatus::Running),
        "succeeded" | "completed" | "partial_success" => Some(TaskStatus::Completed),
        "failed" => Some(TaskStatus::Failed),
        "cancelled" => Some(TaskStatus::Cancelled),
        "interrupted" => Some(TaskStatus::Interrupted),
        _ => None,
    }
}

pub(super) async fn list(
    State(state): State<AppState>,
    Query(query): Query<JobQuery>,
    request: Request,
) -> Response {
    let page = query.page.unwrap_or(1).max(1);
    let size = query.page_size.unwrap_or(30).clamp(1, 200);
    let Some(tm) = state.task_manager.as_ref() else {
        return Json(serde_json::json!({
            "items": [],
            "total": 0,
            "page": page,
            "page_size": size,
        }))
        .into_response();
    };

    let offset = (page - 1).saturating_mul(size);
    let filter = TaskFilter {
        status: parse_status_filter(query.status.as_deref()),
        query: query.q,
        limit: Some(size),
        offset: Some(offset),
        ..Default::default()
    };

    match tm.list_all_tasks(&filter).await {
        Ok(task_page) => {
            let items: Vec<Value> = task_page
                .items
                .iter()
                .map(|task| project_task_to_admin_job(task, None))
                .collect();
            Json(serde_json::json!({
                "items": items,
                "total": task_page.total,
                "page": page,
                "page_size": size,
            }))
            .into_response()
        }
        Err(e) => task_failure(&request, e),
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ArtifactQuery {
    page: Option<u32>,
    page_size: Option<u32>,
    kind: Option<String>,
    role: Option<String>,
    job_id: Option<String>,
    task_id: Option<String>,
    flow_id: Option<String>,
    agent_id: Option<String>,
    q: Option<String>,
}

pub(super) async fn list_artifacts(
    State(state): State<AppState>,
    Query(query): Query<ArtifactQuery>,
    request: Request,
) -> Response {
    let page = query.page.unwrap_or(1).max(1);
    let size = query.page_size.unwrap_or(30).clamp(1, 200);
    let Some(tm) = state.task_manager.as_ref() else {
        let empty = TaskArtifactCatalogPage::default();
        return Json(project_artifact_catalog_page(&empty, page, size)).into_response();
    };

    let kind = match query
        .kind
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty() && *s != "all")
    {
        Some(raw) => match TaskArtifactKind::parse(raw) {
            Some(k) => Some(k),
            None => return error(&request, StatusCode::BAD_REQUEST, "invalid_arguments"),
        },
        None => None,
    };
    let role = match query
        .role
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty() && *s != "all")
    {
        Some(raw) => match TaskArtifactRole::parse(raw) {
            Some(r) => Some(r),
            None => return error(&request, StatusCode::BAD_REQUEST, "invalid_arguments"),
        },
        None => None,
    };

    let offset = (page - 1).saturating_mul(size);
    let filter = TaskArtifactFilter {
        kind,
        role,
        task_id: query.job_id.or(query.task_id),
        flow_id: query.flow_id,
        producer_agent_id: query.agent_id,
        query: query.q,
        limit: Some(size),
        offset: Some(offset),
    };

    match tm.list_all_artifacts(&filter).await {
        Ok(catalog_page) => {
            Json(project_artifact_catalog_page(&catalog_page, page, size)).into_response()
        }
        Err(e) => task_failure(&request, e),
    }
}

pub(super) async fn get(
    State(state): State<AppState>,
    Path(id): Path<String>,
    request: Request,
) -> Response {
    let tm = match task_manager(&state, &request) {
        Ok(tm) => tm,
        Err(resp) => return resp,
    };
    match tm.get_task_detail_any_scope(&id).await {
        Ok(detail) => Json(project_task_to_admin_job(&detail.task, Some(&detail))).into_response(),
        Err(e) => task_failure(&request, e),
    }
}

pub(super) async fn delete(
    State(state): State<AppState>,
    Path(id): Path<String>,
    request: Request,
) -> Response {
    let tm = match task_manager(&state, &request) {
        Ok(tm) => tm,
        Err(resp) => return resp,
    };
    match tm.delete_task_any_scope(&id).await {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(e) => task_failure(&request, e),
    }
}

pub(super) async fn artifact(
    State(state): State<AppState>,
    Path((id, artifact_id)): Path<(String, String)>,
    request: Request,
) -> Response {
    let tm = match task_manager(&state, &request) {
        Ok(tm) => tm,
        Err(resp) => return resp,
    };
    match tm
        .read_task_artifact_bytes_any_scope(&id, &artifact_id)
        .await
    {
        Ok((meta, bytes)) => {
            let safe_filename: String = meta
                .name
                .chars()
                .filter(|c| !matches!(c, '"' | '\\' | '\r' | '\n'))
                .collect();
            let disposition = format!("inline; filename=\"{safe_filename}\"");
            let etag = format!("\"{}\"", meta.sha256);
            (
                StatusCode::OK,
                [
                    (axum::http::header::CONTENT_TYPE, meta.mime_type),
                    (axum::http::header::CONTENT_DISPOSITION, disposition),
                    (axum::http::header::ETAG, etag),
                ],
                bytes,
            )
                .into_response()
        }
        Err(e) => task_failure(&request, e),
    }
}

pub(super) async fn cancel(
    State(state): State<AppState>,
    Path(id): Path<String>,
    request: Request,
) -> Response {
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Cancel {}
    let (request, _): (_, Cancel) = match json(request).await {
        Ok(v) => v,
        Err(e) => return e,
    };
    let tm = match task_manager(&state, &request) {
        Ok(tm) => tm,
        Err(resp) => return resp,
    };
    match tm.cancel_task_any_scope(&id).await {
        Ok(task) => {
            let detail = tm.get_task_detail_any_scope(&task.id).await.ok();
            Json(project_task_to_admin_job(&task, detail.as_ref())).into_response()
        }
        Err(e) => task_failure(&request, e),
    }
}

pub(super) async fn retry(
    State(state): State<AppState>,
    Path(id): Path<String>,
    request: Request,
) -> Response {
    #[derive(Deserialize)]
    struct Retry {}
    let idempotency_key = match extract_idempotency_key(request.headers()) {
        Ok(v) => v,
        Err(code) => return error(&request, StatusCode::BAD_REQUEST, code),
    };
    let (request, _input): (_, Retry) = match json(request).await {
        Ok(value) => value,
        Err(error) => return error,
    };
    let tm = match task_manager_for_submit(&state, &request) {
        Ok(tm) => tm,
        Err(resp) => return resp,
    };
    let fingerprint = format!("retry:{id}");
    match lookup_idempotent_job(&state, &tm, idempotency_key.as_deref(), &fingerprint).await {
        Ok(Some(mut job_json)) => {
            job_json["retry_of_job_id"] = serde_json::json!(id);
            return (StatusCode::OK, Json(job_json)).into_response();
        }
        Ok(None) => {}
        Err(code) => return error(&request, StatusCode::CONFLICT, code),
    }
    let original = match tm.get_task_any_scope(&id).await {
        Ok(task) => task,
        Err(e) => return task_failure(&request, e),
    };
    let req = CreateTaskRequest::new(original.title, original.prompt)
        .with_target(format!("{}/{}", original.flow_id, original.agent_id))
        .with_source(TaskSource::Api)
        .with_wiki_links(original.related_wiki_links);
    match tm.create_task(req).await {
        Ok(task) => {
            store_idempotent_job(&state, idempotency_key, fingerprint, task.id.clone());
            let detail = tm.get_task_detail_any_scope(&task.id).await.ok();
            let mut job_json = project_task_to_admin_job(&task, detail.as_ref());
            job_json["retry_of_job_id"] = serde_json::json!(id);
            (StatusCode::CREATED, Json(job_json)).into_response()
        }
        Err(e) => task_failure(&request, e),
    }
}

#[derive(Deserialize)]
pub(super) struct FollowupInput {
    pub prompt: String,
}

pub(super) async fn followup(
    State(state): State<AppState>,
    Path(id): Path<String>,
    request: Request,
) -> Response {
    let (request, input): (_, FollowupInput) = match json(request).await {
        Ok(v) => v,
        Err(e) => return e,
    };
    let tm = match task_manager(&state, &request) {
        Ok(tm) => tm,
        Err(resp) => return resp,
    };
    match tm
        .send_followup_any_scope(&id, input.prompt, TaskSource::Api)
        .await
    {
        Ok(followup_view) => (StatusCode::ACCEPTED, Json(followup_view)).into_response(),
        Err(e) => task_failure(&request, e),
    }
}

#[derive(Deserialize)]
pub(super) struct HistoryQuery {
    agent: Option<String>,
    limit: Option<u32>,
}

pub(super) async fn history(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(query): Query<HistoryQuery>,
    request: Request,
) -> Response {
    let tm = match task_manager(&state, &request) {
        Ok(tm) => tm,
        Err(resp) => return resp,
    };
    match tm
        .get_task_agent_history_any_scope(&id, query.agent.as_deref(), query.limit)
        .await
    {
        Ok(items) => Json(serde_json::json!({ "items": items })).into_response(),
        Err(e) => task_failure(&request, e),
    }
}

#[derive(Deserialize)]
pub(super) struct EventQuery {
    after: Option<i64>,
    limit: Option<u32>,
}

pub(super) async fn events(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(query): Query<EventQuery>,
    request: Request,
) -> Response {
    let Some(after) = query.after.or(Some(0)).filter(|n| *n >= 0) else {
        return error(&request, StatusCode::BAD_REQUEST, "invalid_arguments");
    };
    let limit = query.limit.unwrap_or(100).clamp(1, 100);
    let tm = match task_manager(&state, &request) {
        Ok(tm) => tm,
        Err(resp) => return resp,
    };
    match tm.get_task_detail_any_scope(&id).await {
        Ok(detail) => {
            let items = project_task_events(&detail, after, limit);
            Json(serde_json::json!({ "items": items })).into_response()
        }
        Err(e) => task_failure(&request, e),
    }
}

fn task_id_from_event(event: &TaskEvent) -> Option<String> {
    match event {
        TaskEvent::TaskCreated { task } | TaskEvent::TaskTerminal { task } => Some(task.id.clone()),
        TaskEvent::TaskProgress { task_id, .. }
        | TaskEvent::TaskDeleted { task_id, .. }
        | TaskEvent::FollowupQueued { task_id, .. } => Some(task_id.clone()),
        TaskEvent::ScheduleTriggered {
            spawned_task_id, ..
        } => Some(spawned_task_id.clone()),
        TaskEvent::TaskExecutionStream {
            task_id,
            state_revision: Some(_),
            ..
        } => Some(task_id.clone()),
        TaskEvent::TaskExecutionStream { .. } | TaskEvent::ReminderDue { .. } => None,
    }
}

/// Real-time SSE stream backed by [`TaskManager::subscribe_all`].
pub(super) async fn stream(State(state): State<AppState>, request: Request) -> Response {
    use axum::response::sse::{Event, KeepAlive, Sse};
    use std::{convert::Infallible, time::Duration};

    let tm = match task_manager(&state, &request) {
        Ok(tm) => tm,
        Err(resp) => return resp,
    };
    let subscription = match tm.subscribe_all() {
        Ok(sub) => sub,
        Err(e) => return task_failure(&request, e),
    };
    let stopping = state.lifecycle.stopping().clone();

    let messages = futures_util::stream::unfold(
        (subscription, stopping, true),
        |(mut sub, stopping, first)| async move {
            if first {
                return Some((
                    Ok::<Event, Infallible>(Event::default().event("ready").data("{}")),
                    (sub, stopping, false),
                ));
            }
            loop {
                let item = tokio::select! {
                    biased;
                    _ = stopping.cancelled() => return None,
                    item = sub.recv() => item?,
                };
                match item {
                    TaskFeedItem::Event(ev) => {
                        if let Some(job_id) = task_id_from_event(&ev) {
                            return Some((
                                Ok(Event::default()
                                    .event("job_changed")
                                    .data(serde_json::json!({ "job_id": job_id }).to_string())),
                                (sub, stopping, false),
                            ));
                        }
                    }
                    TaskFeedItem::ResyncRequired => {
                        return Some((
                            Ok(Event::default().event("resync_required").data("{}")),
                            (sub, stopping, false),
                        ));
                    }
                }
            }
        },
    );

    let mut response = Sse::new(messages)
        .keep_alive(
            KeepAlive::new()
                .interval(Duration::from_secs(15))
                .text("keepalive"),
        )
        .into_response();
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
}
