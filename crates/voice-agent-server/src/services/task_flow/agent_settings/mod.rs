//! Persistent & live configuration for Flow 1 (`daily_root`) & Flow 2 (`wiki_root`) Root Agents
//! and their 7 specialist Subagents, integrated with the Server LLM Provider Catalog.

mod catalog;
mod specs;

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use orchestrator_agent_rs::{AgentMode, valid_skill_id};
use serde::{Deserialize, Serialize};

use crate::app::AppState;

pub use catalog::{
    CatalogImageGeneratorBackend, CatalogMediaBackend, CatalogTtsSynthesizerBackend,
    CatalogVideoGeneratorBackend, ResolvedLlmProvider, build_agent_definitions,
    default_orchestrator_provider, find_provider_by_tag, project_agents_json,
    resolve_llm_provider_catalog,
};
pub(crate) use catalog::install_server_tts_runtime;
use catalog::{agent_supports_provider, update_live_media_catalog};
pub use specs::{
    AGENT_SPECS, AgentSpec, DEFAULT_USER_ROOT_AGENT_ID, WIKI_AUTOFLOW_ROOT_AGENT_ID,
    find_agent_spec, is_user_routable_root_agent, normalize_user_target_agent,
};

pub const DEFAULT_ORCHESTRATOR_PROVIDER_REF: &str = "orchestrator:default";
const AGENT_SETTINGS_FILE: &str = "agent_settings.json";

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct AgentConfigOverride {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_ref: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_providers: Option<BTreeMap<String, String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub steps: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allowed_tools: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allowed_skills: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub system_prompt: Option<String>,
}

fn default_true() -> bool {
    true
}

fn default_revision() -> i64 {
    1
}

fn default_provider_ref() -> String {
    DEFAULT_ORCHESTRATOR_PROVIDER_REF.to_owned()
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct PersistedBackgroundSettings {
    #[serde(default)]
    pub enabled: Option<bool>,
    #[serde(default = "default_revision")]
    pub revision: i64,
    #[serde(default = "default_provider_ref")]
    pub orchestrator_llm_provider: String,
    #[serde(default = "default_provider_ref")]
    pub worker_llm_provider: String,
    #[serde(default)]
    pub agents: BTreeMap<String, AgentConfigOverride>,
}

impl Default for PersistedBackgroundSettings {
    fn default() -> Self {
        Self {
            enabled: None,
            revision: 1,
            orchestrator_llm_provider: default_provider_ref(),
            worker_llm_provider: default_provider_ref(),
            agents: BTreeMap::new(),
        }
    }
}

pub fn load_persisted_settings(store_dir: &Path) -> Option<PersistedBackgroundSettings> {
    let path = store_dir.join(AGENT_SETTINGS_FILE);
    let raw = std::fs::read_to_string(path).ok()?;
    let mut settings: PersistedBackgroundSettings = serde_json::from_str(&raw).ok()?;
    for (id, ovr) in &mut settings.agents {
        if let Some(spec) = find_agent_spec(id) {
            if let Some(ref mut tools) = ovr.allowed_tools {
                tools.retain(|t| spec.available_tools.contains(&t.as_str()));
            }
            if let Some(ref mut skills) = ovr.allowed_skills {
                if spec.mode == AgentMode::Primary {
                    skills.clear();
                } else {
                    skills.retain(|s| valid_skill_id(s));
                }
            }
            if let Some(ref mut tool_map) = ovr.tool_providers {
                tool_map.retain(|tool, pref| {
                    !pref.trim().is_empty() && spec.tool_required_tag(tool).is_some()
                });
            }
        }
    }
    Some(settings)
}

pub fn save_persisted_settings(
    store_dir: &Path,
    settings: &PersistedBackgroundSettings,
) -> std::io::Result<()> {
    std::fs::create_dir_all(store_dir)?;
    let path = store_dir.join(AGENT_SETTINGS_FILE);
    let body = serde_json::to_string_pretty(settings).map_err(std::io::Error::other)?;
    std::fs::write(path, body)
}

pub fn write_agent_prompts(
    prompts_dir: &Path,
    settings: &PersistedBackgroundSettings,
) -> std::io::Result<()> {
    std::fs::create_dir_all(prompts_dir)?;
    for spec in AGENT_SPECS {
        let prompt = settings
            .agents
            .get(spec.id)
            .and_then(|o| o.system_prompt.as_deref())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or(spec.default_prompt);
        std::fs::write(prompts_dir.join(format!("{}.md", spec.id)), prompt)?;
    }
    Ok(())
}

pub fn validate_agent_overrides(
    orchestrator_provider: &str,
    worker_provider: &str,
    overrides: &BTreeMap<String, AgentConfigOverride>,
    catalog: &[ResolvedLlmProvider],
) -> Result<(), &'static str> {
    let default_skills = super::skills::catalog_ids_from_dir(None);
    validate_agent_overrides_with_skills(
        orchestrator_provider,
        worker_provider,
        overrides,
        catalog,
        &default_skills,
    )
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
        let Some(spec) = find_agent_spec(id) else {
            return Err("invalid_arguments");
        };
        if spec.mode == AgentMode::Primary && !ovr.enabled {
            return Err("invalid_arguments");
        }
        if let Some(ref pref) = ovr.provider_ref {
            let trimmed = pref.trim();
            if !trimmed.is_empty() {
                let Some(prov) = find_prov(trimmed) else {
                    return Err("invalid_provider");
                };
                if !agent_supports_provider(spec.id, prov) {
                    return Err("provider_capability_mismatch");
                }
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
        if let Some(steps) = ovr.steps
            && !(1..=100).contains(&steps)
        {
            return Err("invalid_arguments");
        }
        if let Some(ref tools) = ovr.allowed_tools {
            if spec.mode == AgentMode::Primary {
                if tools.iter().any(|t| t != "subagent") {
                    return Err("invalid_arguments");
                }
            } else if tools
                .iter()
                .any(|t| !spec.available_tools.contains(&t.as_str()))
            {
                return Err("invalid_arguments");
            }
        }
        if let Some(ref skills) = ovr.allowed_skills {
            if spec.mode == AgentMode::Primary && !skills.is_empty() {
                return Err("invalid_arguments");
            }
            if skills.iter().any(|s| !skill_ids.contains(s.as_str())) {
                return Err("invalid_arguments");
            }
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
    }

    Ok(())
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
    install_server_tts_runtime(state);
    let catalog = resolve_llm_provider_catalog(state).await;
    update_live_media_catalog(&catalog, Some(&settings));
    let Some(orch) = state.orchestrator.as_ref() else {
        return Ok(());
    };
    let (providers, defs) =
        build_agent_definitions(&state.config.orchestrator, &settings, &catalog);
    orch.reconfigure(providers, defs)
        .await
        .map_err(|e| e.to_string())
}
