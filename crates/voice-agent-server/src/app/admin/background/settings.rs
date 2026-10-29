use super::*;
use crate::services::task_flow::{
    AgentConfigOverride, CreateCustomSubagentInput, DEFAULT_ORCHESTRATOR_PROVIDER_REF,
    PersistedBackgroundSettings, SubagentMutationError, create_custom_subagent,
    delete_custom_subagent, find_agent_spec, project_agents_json, resolve_llm_provider_catalog,
    save_persisted_settings, skills, sync_orchestrator_from_state,
    validate_agent_overrides_with_skills,
};
use serde::Deserialize;
use std::collections::BTreeMap;

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
    let llm_providers: Vec<Value> = catalog.iter().map(|p| p.to_json_summary()).collect();
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

pub(in crate::app::admin) async fn settings(
    State(state): State<AppState>,
    _request: Request,
) -> Response {
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
pub(in crate::app::admin) struct SettingsUpdateInput {
    pub enabled: bool,
    #[serde(default)]
    pub orchestrator_llm_provider: Option<String>,
    #[serde(default)]
    pub worker_llm_provider: Option<String>,
    #[serde(default)]
    pub agents: Option<BTreeMap<String, AgentConfigOverride>>,
}

pub(in crate::app::admin) async fn save_settings(
    State(state): State<AppState>,
    request: Request,
) -> Response {
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

pub(in crate::app::admin) async fn create_agent(
    State(state): State<AppState>,
    request: Request,
) -> Response {
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

pub(in crate::app::admin) async fn delete_agent(
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
