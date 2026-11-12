//! Persistent `agent_settings.json` storage, partial override merging, and Markdown prompt
//! materialization (`<prompts_dir>/<id>.md` plus server-owned Sub-Agent guidance blocks).

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use orchestrator_agent_rs::{AgentMode, SkillDocument, valid_skill_id};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::DEFAULT_ORCHESTRATOR_PROVIDER_REF;
use super::specs::{
    AGENT_SPECS, custom_subagent_flow_policy, find_agent_spec, valid_custom_subagent_id,
};
use crate::services::task_flow::workspace_seed::{
    BUILTIN_SKILLS, daily_root_prompt_with_routing_guidance,
};

const AGENT_SETTINGS_FILE: &str = "agent_settings.json";
const REQUIRED_JOB_SKILLS_OPEN: &str = "<required_job_skills>";
const REQUIRED_JOB_SKILLS_CLOSE: &str = "</required_job_skills>";
const REQUIRED_SKILL_MAX_BYTES: usize = 1024 * 1024;

/// SHA-256 of the trimmed `cloud_worker` default prompts of earlier releases. The Admin form saves
/// the prompt it shows, so an unedited copy would otherwise pin an old default forever.
const OLD_CLOUD_WORKER_DEFAULT_PROMPTS: &[&str] = &[
    "fde64e70d90917fd1b8c40b5bc653b4690f2da843c724b255c7345be71eacac7",
    "94f78b0e8afab5f8c9d35cca7410046da155ed8587ec4dee4634cb5032f97810",
    "19a0f19656f7fa03aa67ab1f086cf5e96ad1815141df1229a9148a4ced49ee69",
];

fn is_old_cloud_worker_default_prompt(prompt: &str) -> bool {
    let digest: String = Sha256::digest(prompt.trim().as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    OLD_CLOUD_WORKER_DEFAULT_PROMPTS.contains(&digest.as_str())
}

/// SHA-256 of the `wiki_ingestor` and `wiki_curator` defaults before Required Job Skills were
/// preloaded. Admin Web may have persisted these exact defaults as overrides on older servers.
/// Verified against the `WIKI_INGESTOR_PROMPT` / `WIKI_CURATOR_PROMPT` literals in llm-wiki-rs
/// `src/prompts/subagents.rs` at commit f9f05179 on `main` (trimmed SHA-256).
const OLD_WIKI_SUBAGENT_DEFAULT_PROMPTS: &[(&str, &str)] = &[
    (
        "wiki_ingestor",
        "21e00cf28da2c2cf6f1bd4b68a376be335479a21ee06a73fc41437824a703a40",
    ),
    (
        "wiki_curator",
        "284339c1ae7f7f5b10c562664626a023e975c8d9a0737a879059aee64651893d",
    ),
];

fn is_old_wiki_subagent_default_prompt(agent_id: &str, prompt: &str) -> bool {
    let Some((_, expected_hash)) = OLD_WIKI_SUBAGENT_DEFAULT_PROMPTS
        .iter()
        .find(|(id, _)| *id == agent_id)
    else {
        return false;
    };
    let digest: String = Sha256::digest(prompt.trim().as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    digest == *expected_hash
}

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

#[derive(Clone, Debug, Deserialize, Serialize)]
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

impl Default for AgentConfigOverride {
    fn default() -> Self {
        Self {
            enabled: true,
            flow_id: None,
            provider_ref: None,
            tool_providers: None,
            steps: None,
            allowed_tools: None,
            allowed_skills: None,
            description: None,
            system_prompt: None,
        }
    }
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wiki_chat_llm_provider: Option<String>,
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
            wiki_chat_llm_provider: None,
            agents: BTreeMap::new(),
        }
    }
}

pub fn load_persisted_settings(store_dir: &Path) -> Option<PersistedBackgroundSettings> {
    let path = store_dir.join(AGENT_SETTINGS_FILE);
    let raw = std::fs::read_to_string(path).ok()?;
    let mut settings: PersistedBackgroundSettings = serde_json::from_str(&raw).ok()?;

    // Seamless migration: move legacy `tts_synthesize` / `asr_transcribe` `tool_providers`
    // from `media_creator` to `media_processor` if not already set on `media_processor`.
    let mut migrated_proc_tools = BTreeMap::new();
    if let Some(creator_ovr) = settings.agents.get_mut("media_creator")
        && let Some(ref mut tool_map) = creator_ovr.tool_providers
    {
        for moved_tool in ["tts_synthesize", "asr_transcribe"] {
            if let Some(pref) = tool_map.remove(moved_tool)
                && !pref.trim().is_empty()
            {
                migrated_proc_tools.insert(moved_tool.to_owned(), pref);
            }
        }
    }
    if !migrated_proc_tools.is_empty() {
        let proc_ovr = settings
            .agents
            .entry("media_processor".to_owned())
            .or_default();
        let proc_map = proc_ovr.tool_providers.get_or_insert_with(BTreeMap::new);
        for (tool, pref) in migrated_proc_tools {
            proc_map.entry(tool).or_insert(pref);
        }
    }

    if let Some(cloud_worker) = settings.agents.get_mut("cloud_worker")
        && cloud_worker
            .system_prompt
            .as_deref()
            .is_some_and(is_old_cloud_worker_default_prompt)
    {
        cloud_worker.system_prompt = None;
    }

    for (agent_id, agent) in &mut settings.agents {
        if agent
            .system_prompt
            .as_deref()
            .is_some_and(|prompt| is_old_wiki_subagent_default_prompt(agent_id, prompt))
        {
            agent.system_prompt = None;
        }
    }

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
        if !ovr.map_or_else(
            || find_agent_spec(sub_id).is_some_and(|s| s.default_enabled),
            |o| o.enabled,
        ) {
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
    let cloud_guard = if flow_id == "daily" {
        "\nLong-running cloud research is only for Chat/API tasks. Never delegate it for Task source Voice, whose Job Budget is five minutes. Follow-up after a use-once cloud Sub-Agent ends requires a new Sub-Agent with explicit context.\n"
    } else {
        ""
    };
    let cloud_worker_rules = if lines
        .iter()
        .any(|line| line.starts_with("- `cloud_worker`"))
    {
        "When delegating to `cloud_worker`:\n- List every requested goal and its order before the first delegation. For example, \"A sau khi hoàn thành tiếp với B\" means A first, then B.\n- Use one new `cloud_worker` per goal, and wait for each to finish before starting the next.\n- Do not add constraints the user did not ask for, such as forbidding artifacts.\n- Say a goal or earlier work is done only if a Sub-Agent result in this session shows it.\n- In the final answer, report each goal's outcome, include the returned artifact IDs and source URLs, keep reported data limitations, and say the content comes from an external service.\n- Muse sessions are kept by default. Tell `cloud_worker` to delete its session only when the user explicitly asks for that.\n"
    } else {
        ""
    };
    format!(
        "\n\n<active_subagents>\nCurrently enabled subagents you may delegate to via the `subagent` tool (`agent` parameter):\n{body}\nDo NOT delegate to any subagent not listed in <active_subagents>.\n{cloud_guard}{cloud_worker_rules}</active_subagents>\n"
    )
}

pub fn write_agent_prompts(
    prompts_dir: &Path,
    settings: &PersistedBackgroundSettings,
) -> std::io::Result<()> {
    write_agent_prompts_with_skills_dir(prompts_dir, settings, None)
}

pub fn write_agent_prompts_with_skills_dir(
    prompts_dir: &Path,
    settings: &PersistedBackgroundSettings,
    skills_root: Option<&Path>,
) -> std::io::Result<()> {
    std::fs::create_dir_all(prompts_dir)?;
    let mut expected_files = BTreeSet::new();

    for spec in AGENT_SPECS {
        let configured_prompt = settings
            .agents
            .get(spec.id)
            .and_then(|o| o.system_prompt.as_deref())
            .map(str::trim)
            .filter(|s| !s.is_empty());
        let is_default_prompt = configured_prompt.is_none();
        let base_prompt = configured_prompt.unwrap_or(spec.default_prompt);
        let filename = format!("{}.md", spec.id);
        expected_files.insert(filename.clone());
        if spec.mode == AgentMode::Primary {
            // The discovery block is server-owned. Old saved prompts may contain a snapshot;
            // replace that block while preserving the Admin's surrounding custom instructions.
            let mut prompt = if spec.id == "daily_root" && is_default_prompt {
                daily_root_prompt_with_routing_guidance(base_prompt)
            } else {
                base_prompt.to_owned()
            };
            while let Some(start) = prompt.find("<active_subagents>") {
                let Some(relative_end) = prompt[start..].find("</active_subagents>") else {
                    break;
                };
                let end = start + relative_end + "</active_subagents>".len();
                prompt.replace_range(start..end, "");
            }
            let block =
                format_active_subagents_block(spec.flow_id, spec.default_subagents, settings);
            std::fs::write(prompts_dir.join(&filename), format!("{prompt}{block}"))?;
        } else {
            let prompt = with_required_job_skills(base_prompt, spec.id, skills_root);
            std::fs::write(prompts_dir.join(&filename), prompt)?;
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

fn with_required_job_skills(
    base_prompt: &str,
    agent_id: &str,
    skills_root: Option<&Path>,
) -> String {
    let required = super::specs::required_job_skills(agent_id);
    if required.is_empty() {
        return base_prompt.to_owned();
    }

    let mut prompt = strip_required_job_skills_block(base_prompt);
    let mut entries = Vec::with_capacity(required.len());
    for skill_id in required {
        let Some(body) = required_job_skill_body(skill_id, skills_root) else {
            tracing::warn!(
                agent_id,
                skill_id,
                "required Job Skill has no valid embedded document"
            );
            continue;
        };
        entries.push(format!("## {skill_id}\n\n{body}"));
    }
    if entries.is_empty() {
        return prompt;
    }

    prompt.push_str("\n\n<required_job_skills>\n");
    prompt.push_str(
        "The Required Job Skills below are already loaded. Follow their instructions unless they conflict with the instructions above. Do not call the `skill` tool to load them.\n\n",
    );
    prompt.push_str(&entries.join("\n\n"));
    prompt.push_str("\n</required_job_skills>\n");
    prompt
}

fn strip_required_job_skills_block(prompt: &str) -> String {
    let mut cleaned = prompt.to_owned();
    while let Some(start) = cleaned.find(REQUIRED_JOB_SKILLS_OPEN) {
        // An unclosed opening tag is Admin text, not a server block: leave it and everything after
        // it untouched rather than deleting the rest of the prompt.
        let Some(relative_end) = cleaned[start..].find(REQUIRED_JOB_SKILLS_CLOSE) else {
            break;
        };
        let end = start + relative_end + REQUIRED_JOB_SKILLS_CLOSE.len();
        cleaned.replace_range(start..end, "");
    }
    cleaned
}

fn required_job_skill_body(skill_id: &str, skills_root: Option<&Path>) -> Option<String> {
    if let Some(root) = skills_root
        && let Some(body) = read_valid_skill_body(root, skill_id)
    {
        return Some(body);
    }

    let embedded = BUILTIN_SKILLS
        .iter()
        .find_map(|(id, text)| (*id == skill_id).then_some(*text))?;
    SkillDocument::parse(skill_id, embedded, REQUIRED_SKILL_MAX_BYTES)
        .ok()
        .map(|doc| doc.body)
}

fn read_valid_skill_body(skills_root: &Path, skill_id: &str) -> Option<String> {
    let canonical_root = std::fs::canonicalize(skills_root).ok()?;
    if !std::fs::metadata(&canonical_root).ok()?.is_dir() {
        return None;
    }
    let skill_dir = skills_root.join(skill_id);
    let dir_meta = std::fs::symlink_metadata(&skill_dir).ok()?;
    if dir_meta.file_type().is_symlink() || !dir_meta.is_dir() {
        return None;
    }
    let canonical_dir = std::fs::canonicalize(&skill_dir).ok()?;
    if !canonical_dir.starts_with(&canonical_root) {
        return None;
    }
    let skill_file = skill_dir.join("SKILL.md");
    let file_meta = std::fs::symlink_metadata(&skill_file).ok()?;
    if file_meta.file_type().is_symlink() || !file_meta.is_file() {
        return None;
    }
    let canonical_file = std::fs::canonicalize(&skill_file).ok()?;
    if !canonical_file.starts_with(&canonical_dir)
        || file_meta.len() > REQUIRED_SKILL_MAX_BYTES as u64
    {
        return None;
    }
    let raw = std::fs::read_to_string(&skill_file).ok()?;
    SkillDocument::parse(skill_id, &raw, REQUIRED_SKILL_MAX_BYTES)
        .ok()
        .map(|doc| doc.body)
}

#[cfg(test)]
mod required_block_tests {
    use super::strip_required_job_skills_block;

    #[test]
    fn unclosed_required_tag_keeps_the_rest_of_the_admin_prompt() {
        let prompt = "Admin text <required_job_skills> still here\nMore instructions.";
        assert_eq!(strip_required_job_skills_block(prompt), prompt);
    }

    #[test]
    fn closed_required_block_is_removed_and_surrounding_text_kept() {
        let prompt = "Before\n<required_job_skills>\nold\n</required_job_skills>\nAfter";
        assert_eq!(strip_required_job_skills_block(prompt), "Before\n\nAfter");
    }
}
