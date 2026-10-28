//! Override validation, Custom Sub-Agent CRUD lifecycle (`create_custom_subagent`,
//! `delete_custom_subagent`), and live `Orchestrator` synchronization.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use orchestrator_agent_rs::AgentMode;
use serde::{Deserialize, Serialize};

use crate::app::AppState;

use super::media_backend::{install_server_tts_runtime, update_live_media_catalog};
use super::persistence::{
    AgentConfigOverride, PersistedBackgroundSettings, default_true, save_persisted_settings,
    write_agent_prompts,
};
use super::provider_catalog::{
    ResolvedLlmProvider, agent_supports_provider, resolve_llm_provider_catalog,
};
use super::resolution::build_agent_definitions;
use super::specs::{custom_subagent_flow_policy, find_agent_spec, valid_custom_subagent_id};

pub fn validate_agent_overrides(
    orchestrator_provider: &str,
    worker_provider: &str,
    overrides: &BTreeMap<String, AgentConfigOverride>,
    catalog: &[ResolvedLlmProvider],
) -> Result<(), &'static str> {
    let default_skills = super::super::skills::catalog_ids_from_dir(None);
    validate_agent_overrides_with_skills(
        orchestrator_provider,
        worker_provider,
        overrides,
        catalog,
        &default_skills,
    )
}

fn validate_common_subagent_fields(
    agent_id: &str,
    ovr: &AgentConfigOverride,
    available_tools: &[&str],
    catalog: &[ResolvedLlmProvider],
    skill_ids: &BTreeSet<String>,
) -> Result<(), &'static str> {
    if let Some(ref pref) = ovr.provider_ref {
        let trimmed = pref.trim();
        if !trimmed.is_empty() {
            let Some(prov) = catalog.iter().find(|p| p.provider_ref == trimmed) else {
                return Err("invalid_provider");
            };
            if !agent_supports_provider(agent_id, prov) {
                return Err("provider_capability_mismatch");
            }
        }
    }
    if let Some(steps) = ovr.steps
        && !(1..=100).contains(&steps)
    {
        return Err("invalid_arguments");
    }
    if let Some(ref tools) = ovr.allowed_tools
        && tools.iter().any(|t| !available_tools.contains(&t.as_str()))
    {
        return Err("invalid_arguments");
    }
    if let Some(ref skills) = ovr.allowed_skills
        && skills.iter().any(|s| !skill_ids.contains(s.as_str()))
    {
        return Err("invalid_arguments");
    }
    if let Some(ref desc) = ovr.description {
        let trimmed = desc.trim();
        if trimmed.is_empty() || trimmed.chars().count() > 1024 {
            return Err("invalid_arguments");
        }
    }
    if let Some(ref prompt) = ovr.system_prompt {
        let trimmed = prompt.trim();
        if trimmed.is_empty() || trimmed.len() > 65_536 {
            return Err("invalid_arguments");
        }
    }
    Ok(())
}

pub fn validate_agent_overrides_with_skills(
    orchestrator_provider: &str,
    worker_provider: &str,
    overrides: &BTreeMap<String, AgentConfigOverride>,
    catalog: &[ResolvedLlmProvider],
    skill_ids: &BTreeSet<String>,
) -> Result<(), &'static str> {
    let find_prov = |pref: &str| catalog.iter().find(|p| p.provider_ref == pref);
    let Some(orch_prov) = find_prov(orchestrator_provider.trim()) else {
        return Err("invalid_provider");
    };
    let Some(work_prov) = find_prov(worker_provider.trim()) else {
        return Err("invalid_provider");
    };
    if !orch_prov.supports_tag("chat") || !work_prov.supports_tag("chat") {
        return Err("provider_capability_mismatch");
    }

    for (id, ovr) in overrides {
        if let Some(spec) = find_agent_spec(id) {
            if ovr
                .flow_id
                .as_deref()
                .is_some_and(|f| f.trim() != spec.flow_id)
            {
                return Err("invalid_arguments");
            }
            if spec.mode == AgentMode::Primary {
                if !ovr.enabled {
                    return Err("invalid_arguments");
                }
                if ovr
                    .allowed_skills
                    .as_ref()
                    .is_some_and(|skills| !skills.is_empty())
                {
                    return Err("invalid_arguments");
                }
            }
            if let Some(ref tool_map) = ovr.tool_providers {
                for (tool_name, pref) in tool_map {
                    let Some(required_tag) = spec.tool_required_tag(tool_name) else {
                        return Err("invalid_arguments");
                    };
                    let trimmed = pref.trim();
                    if !trimmed.is_empty() {
                        let Some(prov) = find_prov(trimmed) else {
                            return Err("invalid_provider");
                        };
                        if !prov.supports_tag(required_tag) {
                            return Err("provider_capability_mismatch");
                        }
                    }
                }
            }
            validate_common_subagent_fields(
                spec.id,
                ovr,
                spec.available_tools,
                catalog,
                skill_ids,
            )?;
        } else {
            if !valid_custom_subagent_id(id) {
                return Err("invalid_agent_id");
            }
            let Some(flow_id) = ovr.flow_id.as_deref().map(str::trim) else {
                return Err("invalid_arguments");
            };
            let Some((_, available_tools)) = custom_subagent_flow_policy(flow_id) else {
                return Err("invalid_arguments");
            };
            if ovr.tool_providers.as_ref().is_some_and(|m| !m.is_empty()) {
                return Err("invalid_arguments");
            }
            if ovr.description.is_none() || ovr.system_prompt.is_none() {
                return Err("invalid_arguments");
            }
            validate_common_subagent_fields(id, ovr, available_tools, catalog, skill_ids)?;
        }
    }

    Ok(())
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct CreateCustomSubagentInput {
    pub id: String,
    pub flow_id: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default)]
    pub provider_ref: Option<String>,
    #[serde(default)]
    pub steps: Option<u32>,
    #[serde(default)]
    pub allowed_tools: Option<Vec<String>>,
    #[serde(default)]
    pub allowed_skills: Option<Vec<String>>,
    pub description: String,
    pub system_prompt: String,
}

#[derive(Debug, PartialEq, Eq)]
pub enum SubagentMutationError {
    InvalidId,
    InvalidFlowId,
    InvalidProvider,
    ProviderCapabilityMismatch,
    InvalidArguments(&'static str),
    AlreadyExists,
    NotFound,
    BuiltinCannotBeDeleted,
    RevisionConflict,
    Io,
}

async fn persist_and_sync_after_mutation(
    state: &AppState,
    subagent_id: &str,
    persisted: &PersistedBackgroundSettings,
) -> Result<i64, SubagentMutationError> {
    save_persisted_settings(&state.config.orchestrator.store_dir, persisted)
        .map_err(|_| SubagentMutationError::Io)?;
    write_agent_prompts(&state.config.orchestrator.prompts_dir, persisted)
        .map_err(|_| SubagentMutationError::Io)?;
    if let Err(err) = reconfigure_orchestrator_with_settings(state, persisted).await {
        tracing::warn!(
            error = %err,
            subagent_id = %subagent_id,
            "failed to reconfigure orchestrator after custom subagent mutation"
        );
    }
    Ok(persisted.revision)
}

pub async fn create_custom_subagent(
    state: &AppState,
    expected_revision: i64,
    input: CreateCustomSubagentInput,
) -> Result<i64, SubagentMutationError> {
    let id = input.id.trim();
    if !valid_custom_subagent_id(id) {
        return Err(SubagentMutationError::InvalidId);
    }
    let flow_id = input.flow_id.trim();
    if custom_subagent_flow_policy(flow_id).is_none() {
        return Err(SubagentMutationError::InvalidFlowId);
    }

    let allowed_tools = match input.allowed_tools {
        Some(tools) if !tools.is_empty() => tools,
        _ => vec!["skill".to_owned()],
    };
    let ovr = AgentConfigOverride {
        enabled: input.enabled,
        flow_id: Some(flow_id.to_owned()),
        provider_ref: input
            .provider_ref
            .map(|s| s.trim().to_owned())
            .filter(|s| !s.is_empty()),
        tool_providers: None,
        steps: Some(input.steps.unwrap_or(15)),
        allowed_tools: Some(allowed_tools),
        allowed_skills: Some(input.allowed_skills.unwrap_or_default()),
        description: Some(input.description.trim().to_owned()),
        system_prompt: Some(input.system_prompt.trim().to_owned()),
    };

    let catalog = resolve_llm_provider_catalog(state).await;
    let skill_ids = super::super::skills::catalog_ids(state);

    let persisted = {
        let mut control = state
            .admin_task_control
            .lock()
            .expect("admin_task_control mutex poisoned");
        if control.background_revision != expected_revision {
            return Err(SubagentMutationError::RevisionConflict);
        }
        if control.background_settings.agents.contains_key(id) {
            return Err(SubagentMutationError::AlreadyExists);
        }
        let mut next_agents = control.background_settings.agents.clone();
        next_agents.insert(id.to_owned(), ovr);
        if let Err(code) = validate_agent_overrides_with_skills(
            &control.background_settings.orchestrator_llm_provider,
            &control.background_settings.worker_llm_provider,
            &next_agents,
            &catalog,
            &skill_ids,
        ) {
            return Err(match code {
                "invalid_provider" => SubagentMutationError::InvalidProvider,
                "provider_capability_mismatch" => {
                    SubagentMutationError::ProviderCapabilityMismatch
                }
                "invalid_agent_id" => SubagentMutationError::InvalidId,
                other => SubagentMutationError::InvalidArguments(other),
            });
        }
        let next_revision = control.background_revision + 1;
        control.background_revision = next_revision;
        control.background_settings.revision = next_revision;
        control.background_settings.agents = next_agents;
        control.background_settings.clone()
    };

    persist_and_sync_after_mutation(state, id, &persisted).await
}

pub async fn delete_custom_subagent(
    state: &AppState,
    expected_revision: i64,
    id: &str,
) -> Result<i64, SubagentMutationError> {
    let id = id.trim();
    if find_agent_spec(id).is_some() {
        return Err(SubagentMutationError::BuiltinCannotBeDeleted);
    }
    if !valid_custom_subagent_id(id) {
        return Err(SubagentMutationError::InvalidId);
    }

    let persisted = {
        let mut control = state
            .admin_task_control
            .lock()
            .expect("admin_task_control mutex poisoned");
        if control.background_revision != expected_revision {
            return Err(SubagentMutationError::RevisionConflict);
        }
        if !control.background_settings.agents.contains_key(id) {
            return Err(SubagentMutationError::NotFound);
        }
        control.background_settings.agents.remove(id);
        let next_revision = control.background_revision + 1;
        control.background_revision = next_revision;
        control.background_settings.revision = next_revision;
        control.background_settings.clone()
    };

    persist_and_sync_after_mutation(state, id, &persisted).await
}

/// Synchronizes the running `Orchestrator` (if active) with the latest LLM provider catalog
/// and persisted background agent settings.
pub async fn sync_orchestrator_from_state(state: &AppState) -> Result<(), String> {
    let settings = {
        let guard = state
            .admin_task_control
            .lock()
            .expect("admin_task_control mutex poisoned");
        guard.background_settings.clone()
    };
    if state.orchestrator.is_some()
        || state.config.orchestrator.prompts_dir != Path::new("data/orchestrator-prompts")
    {
        let _ = write_agent_prompts(&state.config.orchestrator.prompts_dir, &settings);
    }
    reconfigure_orchestrator_with_settings(state, &settings).await
}

async fn reconfigure_orchestrator_with_settings(
    state: &AppState,
    settings: &PersistedBackgroundSettings,
) -> Result<(), String> {
    install_server_tts_runtime(state);
    let catalog = resolve_llm_provider_catalog(state).await;
    update_live_media_catalog(&catalog, Some(settings));
    let Some(orch) = state.orchestrator.as_ref() else {
        return Ok(());
    };
    let (providers, defs) =
        build_agent_definitions(&state.config.orchestrator, settings, &catalog);
    orch.reconfigure(providers, defs)
        .await
        .map_err(|e| e.to_string())
}
