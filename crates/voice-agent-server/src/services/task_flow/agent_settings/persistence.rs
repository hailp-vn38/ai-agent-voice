//! Persistent `agent_settings.json` storage, partial override merging, and Markdown prompt
//! materialization (`<prompts_dir>/<id>.md` + `<active_subagents>` injection for Root Orchestrators).

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use orchestrator_agent_rs::{AgentMode, valid_skill_id};
use serde::{Deserialize, Serialize};

use super::DEFAULT_ORCHESTRATOR_PROVIDER_REF;
use super::specs::{
    AGENT_SPECS, custom_subagent_flow_policy, find_agent_spec, valid_custom_subagent_id,
};

const AGENT_SETTINGS_FILE: &str = "agent_settings.json";

pub(crate) fn default_true() -> bool {
    true
}

fn default_revision() -> i64 {
    1
}

fn default_provider_ref() -> String {
    DEFAULT_ORCHESTRATOR_PROVIDER_REF.to_owned()
}

/// Validated view of a Custom Sub-Agent entry stored in [`PersistedBackgroundSettings::agents`].
#[derive(Clone, Copy, Debug)]
pub struct ValidCustomSubagentRef<'a> {
    pub id: &'a str,
    pub flow_id: &'static str,
    pub parent_id: &'static str,
    pub available_tools: &'static [&'static str],
    pub description: &'a str,
    pub system_prompt: &'a str,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct AgentConfigOverride {
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Only used by Custom Sub-Agents (`"daily"` | `"wiki"`). Built-in agents use `spec.flow_id`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub flow_id: Option<String>,
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

impl AgentConfigOverride {
    /// Returns `Some(ValidCustomSubagentRef)` if `id` and `self` form a structurally valid
    /// Custom Sub-Agent with a supported `flow_id`, non-empty `description`, and non-empty `system_prompt`.
    pub fn as_valid_custom_subagent<'a>(
        &'a self,
        id: &'a str,
    ) -> Option<ValidCustomSubagentRef<'a>> {
        if !valid_custom_subagent_id(id) {
            return None;
        }
        let raw_flow_id = self.flow_id.as_deref()?.trim();
        let (flow_id, (parent_id, available_tools)) = match raw_flow_id {
            "daily" => ("daily", custom_subagent_flow_policy("daily")?),
            "wiki" => ("wiki", custom_subagent_flow_policy("wiki")?),
            _ => return None,
        };
        let description = self
            .description
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())?;
        let system_prompt = self
            .system_prompt
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())?;
        Some(ValidCustomSubagentRef {
            id,
            flow_id,
            parent_id,
            available_tools,
            description,
            system_prompt,
        })
    }

    /// Merges missing fields from a previously persisted Custom Sub-Agent override so partial
    /// updates (e.g. toggling `enabled` on `BackgroundPage.vue`) preserve immutable `flow_id`
    /// and existing prompts/tools/skills.
    pub fn merge_from_existing_custom(&mut self, prev: &Self) {
        let is_partial = self.description.is_none() && self.system_prompt.is_none();
        if self.flow_id.is_none() {
            self.flow_id = prev.flow_id.clone();
        }
        if self.description.is_none() {
            self.description = prev.description.clone();
        }
        if self.system_prompt.is_none() {
            self.system_prompt = prev.system_prompt.clone();
        }
        if self.steps.is_none() {
            self.steps = prev.steps;
        }
        if self.allowed_tools.is_none() {
            self.allowed_tools = prev.allowed_tools.clone();
        }
        if self.allowed_skills.is_none() {
            self.allowed_skills = prev.allowed_skills.clone();
        }
        if is_partial && self.provider_ref.is_none() {
            self.provider_ref = prev.provider_ref.clone();
        }
    }
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
    settings.agents.retain(|id, ovr| {
        if let Some(spec) = find_agent_spec(id) {
            ovr.flow_id = None;
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
            true
        } else if let Some(valid_custom) = ovr.as_valid_custom_subagent(id) {
            let flow_id = valid_custom.flow_id;
            let available_tools = valid_custom.available_tools;
            ovr.flow_id = Some(flow_id.to_owned());
            ovr.tool_providers = None;
            if let Some(ref mut tools) = ovr.allowed_tools {
                tools.retain(|t| available_tools.contains(&t.as_str()));
            }
            if let Some(ref mut skills) = ovr.allowed_skills {
                skills.retain(|s| valid_skill_id(s));
            }
            true
        } else {
            false
        }
    });
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

fn format_active_subagents_block(
    flow_id: &str,
    default_subagents: &[&str],
    settings: &PersistedBackgroundSettings,
) -> String {
    let mut lines = Vec::new();
    for &sub_id in default_subagents {
        let ovr = settings.agents.get(sub_id);
        if !ovr.is_none_or(|o| o.enabled) {
            continue;
        }
        let desc = ovr
            .and_then(|o| o.description.as_deref())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| find_agent_spec(sub_id).map_or("", |s| s.default_description));
        lines.push(format!("- `{sub_id}` (built-in): {desc}"));
    }
    for (custom_id, ovr) in &settings.agents {
        let Some(custom) = ovr.as_valid_custom_subagent(custom_id) else {
            continue;
        };
        if !ovr.enabled || custom.flow_id != flow_id {
            continue;
        }
        lines.push(format!(
            "- `{}` (custom): {}",
            custom.id, custom.description
        ));
    }
    let body = if lines.is_empty() {
        "(none enabled)".to_owned()
    } else {
        lines.join("\n")
    };
    format!(
        "\n\n<active_subagents>\nCurrently enabled subagents you may delegate to via the `subagent` tool (`agent` parameter):\n{body}\nDo NOT delegate to any subagent not listed in <active_subagents>.\n</active_subagents>\n"
    )
}

pub fn write_agent_prompts(
    prompts_dir: &Path,
    settings: &PersistedBackgroundSettings,
) -> std::io::Result<()> {
    std::fs::create_dir_all(prompts_dir)?;
    let mut expected_files = BTreeSet::new();

    for spec in AGENT_SPECS {
        let base_prompt = settings
            .agents
            .get(spec.id)
            .and_then(|o| o.system_prompt.as_deref())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or(spec.default_prompt);
        let filename = format!("{}.md", spec.id);
        expected_files.insert(filename.clone());
        if spec.mode == AgentMode::Primary && !base_prompt.contains("<active_subagents>") {
            let block =
                format_active_subagents_block(spec.flow_id, spec.default_subagents, settings);
            std::fs::write(prompts_dir.join(&filename), format!("{base_prompt}{block}"))?;
        } else {
            std::fs::write(prompts_dir.join(&filename), base_prompt)?;
        }
    }

    for (custom_id, ovr) in &settings.agents {
        let Some(custom) = ovr.as_valid_custom_subagent(custom_id) else {
            continue;
        };
        let filename = format!("{}.md", custom.id);
        expected_files.insert(filename.clone());
        std::fs::write(prompts_dir.join(&filename), custom.system_prompt)?;
    }

    // Prune stale `.md` files (e.g. deleted Custom Sub-Agents)
    if let Ok(rd) = std::fs::read_dir(prompts_dir) {
        for entry in rd.flatten() {
            let Ok(ft) = entry.file_type() else {
                continue;
            };
            if ft.is_symlink() || !ft.is_file() {
                continue;
            }
            let Ok(name) = entry.file_name().into_string() else {
                continue;
            };
            if name.ends_with(".md") && !expected_files.contains(&name) {
                let _ = std::fs::remove_file(entry.path());
            }
        }
    }

    Ok(())
}
