//! Effective agent resolution, `Orchestrator` `AgentDefinition` construction, and Admin JSON projection.

use std::collections::BTreeMap;

use orchestrator_agent_rs::{AgentDefinition, AgentMode, LlmProviderConfig, ReasoningEffort};
use serde_json::{Value, json};

use crate::config::OrchestratorSectionConfig;

use super::media_backend::resolve_media_tool_provider;
use super::persistence::PersistedBackgroundSettings;
use super::provider_catalog::{
    ResolvedLlmProvider, agent_supports_provider, default_orchestrator_provider,
    find_provider_by_tag,
};
use super::specs::AGENT_SPECS;

struct ResolvedAgentSettings {
    id: String,
    flow_id: &'static str,
    parent_id: Option<&'static str>,
    mode: AgentMode,
    is_builtin: bool,
    enabled: bool,
    explicit_ref: Option<String>,
    resolved_provider: ResolvedLlmProvider,
    execution_provider: ResolvedLlmProvider,
    steps: u32,
    default_steps: u32,
    allowed_tools: Vec<String>,
    default_allowed_tools: Vec<String>,
    available_tools: &'static [&'static str],
    allowed_skills: Vec<String>,
    default_allowed_skills: Vec<String>,
    available_skills: Vec<String>,
    allowed_subagents: Vec<String>,
    allowed_capability_tags: &'static [&'static str],
    tool_capability_tags: &'static [(&'static str, &'static str)],
    description: String,
    default_description: String,
    system_prompt: String,
    default_system_prompt: String,
}

fn resolve_effective_agents(
    cfg: &OrchestratorSectionConfig,
    settings: &PersistedBackgroundSettings,
    catalog: &[ResolvedLlmProvider],
) -> Vec<ResolvedAgentSettings> {
    let skill_ids =
        crate::services::task_flow::skills::catalog_ids_from_dir(cfg.skills_dir.as_deref());
    let fallback = find_provider_by_tag(catalog, "chat")
        .cloned()
        .unwrap_or_else(|| default_orchestrator_provider(cfg));
    let find_provider = |agent_id: &str, pref: &str| -> ResolvedLlmProvider {
        catalog
            .iter()
            .find(|p| p.provider_ref == pref && agent_supports_provider(agent_id, p))
            .cloned()
            .unwrap_or_else(|| fallback.clone())
    };
    let find_chat_provider = |pref: &str| -> ResolvedLlmProvider {
        catalog
            .iter()
            .find(|p| p.provider_ref == pref && p.supports_tag("chat"))
            .cloned()
            .unwrap_or_else(|| fallback.clone())
    };

    let mut out: Vec<ResolvedAgentSettings> = AGENT_SPECS
        .iter()
        .map(|spec| {
            let ovr = settings.agents.get(spec.id);
            let enabled = if spec.mode == AgentMode::Primary {
                true
            } else {
                ovr.is_none_or(|o| o.enabled)
            };
            let explicit_ref = ovr
                .and_then(|o| o.provider_ref.as_deref())
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_owned);
            let effective_ref = explicit_ref.as_deref().unwrap_or_else(|| match spec.mode {
                AgentMode::Primary => settings.orchestrator_llm_provider.as_str(),
                _ => settings.worker_llm_provider.as_str(),
            });
            let resolved_provider = find_provider(spec.id, effective_ref);
            // Subagents in `Orchestrator` always run a chat/tool-calling loop over `/v1/chat/completions`.
            // When `media_creator` is configured with a dedicated `image` or `video` provider (without `chat`),
            // keep `resolved_provider` for UI projection & media backend selection, while using the worker
            // `chat` provider for the Orchestrator's tool-calling session.
            let execution_provider = if resolved_provider.supports_tag("chat") {
                resolved_provider.clone()
            } else {
                find_chat_provider(&settings.worker_llm_provider)
            };
            let steps = ovr
                .and_then(|o| o.steps)
                .unwrap_or(spec.default_steps)
                .clamp(1, 100);
            let available_skills: Vec<String> = if spec.mode == AgentMode::Primary {
                Vec::new()
            } else {
                let mut avail: Vec<String> = spec
                    .suggested_skills
                    .iter()
                    .filter(|s| skill_ids.contains(**s))
                    .map(|s| (*s).to_owned())
                    .collect();
                for s in &skill_ids {
                    if !avail.iter().any(|existing| existing == s) {
                        avail.push(s.clone());
                    }
                }
                avail
            };
            let allowed_skills: Vec<String> = if spec.mode == AgentMode::Primary {
                Vec::new()
            } else {
                match ovr.and_then(|o| o.allowed_skills.as_ref()) {
                    Some(list) => list
                        .iter()
                        .filter(|s| skill_ids.contains(s.as_str()))
                        .cloned()
                        .collect(),
                    None => spec
                        .default_allowed_skills
                        .iter()
                        .filter(|s| skill_ids.contains(**s))
                        .map(|s| (*s).to_owned())
                        .collect(),
                }
            };
            let default_allowed_skills: Vec<String> = if spec.mode == AgentMode::Primary {
                Vec::new()
            } else {
                spec.default_allowed_skills
                    .iter()
                    .filter(|s| skill_ids.contains(**s))
                    .map(|s| (*s).to_owned())
                    .collect()
            };
            let default_allowed_tools: Vec<String> = spec
                .default_allowed_tools
                .iter()
                .map(|t| (*t).to_owned())
                .collect();
            let allowed_tools: Vec<String> = if spec.mode == AgentMode::Primary {
                vec!["subagent".to_owned()]
            } else {
                let mut tools: Vec<String> = match ovr.and_then(|o| o.allowed_tools.as_ref()) {
                    Some(list) => list
                        .iter()
                        .filter(|t| spec.available_tools.contains(&t.as_str()))
                        .cloned()
                        .collect(),
                    None => default_allowed_tools.clone(),
                };
                if !allowed_skills.is_empty() && !tools.iter().any(|t| t == "skill") {
                    tools.insert(0, "skill".to_owned());
                }
                tools
            };
            let allowed_subagents: Vec<String> = if spec.mode == AgentMode::Primary {
                let mut subs: Vec<String> = spec
                    .default_subagents
                    .iter()
                    .filter(|sub_id| {
                        settings
                            .agents
                            .get(**sub_id)
                            .is_none_or(|sub_ovr| sub_ovr.enabled)
                    })
                    .map(|s| (*s).to_owned())
                    .collect();
                for (custom_id, custom_ovr) in &settings.agents {
                    if let Some(valid_custom) = custom_ovr.as_valid_custom_subagent(custom_id)
                        && custom_ovr.enabled
                        && valid_custom.flow_id == spec.flow_id
                    {
                        subs.push(custom_id.clone());
                    }
                }
                subs
            } else {
                Vec::new()
            };
            let description = ovr
                .and_then(|o| o.description.as_deref())
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .unwrap_or(spec.default_description)
                .to_owned();
            let system_prompt = ovr
                .and_then(|o| o.system_prompt.as_deref())
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .unwrap_or(spec.default_prompt)
                .to_owned();

            ResolvedAgentSettings {
                id: spec.id.to_owned(),
                flow_id: spec.flow_id,
                parent_id: spec.parent_id,
                mode: spec.mode,
                is_builtin: true,
                enabled,
                explicit_ref,
                resolved_provider,
                execution_provider,
                steps,
                default_steps: spec.default_steps,
                allowed_tools,
                default_allowed_tools,
                available_tools: spec.available_tools,
                allowed_skills,
                default_allowed_skills,
                available_skills,
                allowed_subagents,
                allowed_capability_tags: spec.allowed_capability_tags(),
                tool_capability_tags: spec.tool_capability_tags(),
                description,
                default_description: spec.default_description.to_owned(),
                system_prompt,
                default_system_prompt: spec.default_prompt.to_owned(),
            }
        })
        .collect();

    // Resolve Custom Sub-Agents defined in `settings.agents`
    for (custom_id, ovr) in &settings.agents {
        let Some(valid) = ovr.as_valid_custom_subagent(custom_id) else {
            continue;
        };

        let explicit_ref = ovr
            .provider_ref
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_owned);
        let effective_ref = explicit_ref
            .as_deref()
            .unwrap_or(settings.worker_llm_provider.as_str());
        let resolved_provider = find_provider(custom_id, effective_ref);
        let execution_provider = if resolved_provider.supports_tag("chat") {
            resolved_provider.clone()
        } else {
            find_chat_provider(&settings.worker_llm_provider)
        };
        let steps = ovr.steps.unwrap_or(15).clamp(1, 100);
        let available_skills: Vec<String> = skill_ids.iter().cloned().collect();
        let allowed_skills: Vec<String> = ovr
            .allowed_skills
            .as_ref()
            .map(|list| {
                list.iter()
                    .filter(|s| skill_ids.contains(s.as_str()))
                    .cloned()
                    .collect()
            })
            .unwrap_or_default();
        let mut allowed_tools: Vec<String> = match ovr.allowed_tools.as_ref() {
            Some(list) => list
                .iter()
                .filter(|t| valid.available_tools.contains(&t.as_str()))
                .cloned()
                .collect(),
            None => vec!["skill".to_owned()],
        };
        if !allowed_skills.is_empty() && !allowed_tools.iter().any(|t| t == "skill") {
            allowed_tools.insert(0, "skill".to_owned());
        }
        let default_allowed_tools = allowed_tools.clone();
        let default_allowed_skills = allowed_skills.clone();

        out.push(ResolvedAgentSettings {
            id: custom_id.clone(),
            flow_id: valid.flow_id,
            parent_id: Some(valid.parent_id),
            mode: AgentMode::Subagent,
            is_builtin: false,
            enabled: ovr.enabled,
            explicit_ref,
            resolved_provider,
            execution_provider,
            steps,
            default_steps: steps,
            allowed_tools,
            default_allowed_tools,
            available_tools: valid.available_tools,
            allowed_skills,
            default_allowed_skills,
            available_skills,
            allowed_subagents: Vec::new(),
            allowed_capability_tags: &["chat", "reasoning"],
            tool_capability_tags: &[],
            description: valid.description.to_owned(),
            default_description: valid.description.to_owned(),
            system_prompt: valid.system_prompt.to_owned(),
            default_system_prompt: valid.system_prompt.to_owned(),
        });
    }

    out
}

pub fn build_agent_definitions(
    cfg: &OrchestratorSectionConfig,
    settings: &PersistedBackgroundSettings,
    catalog: &[ResolvedLlmProvider],
) -> (Vec<LlmProviderConfig>, Vec<AgentDefinition>) {
    let providers: Vec<LlmProviderConfig> = catalog
        .iter()
        .filter(|p| {
            !p.is_tts_adapter()
                && !p.is_asr_adapter()
                && (p.base_url.starts_with("http://") || p.base_url.starts_with("https://"))
        })
        .map(ResolvedLlmProvider::to_orchestrator_provider)
        .collect();

    let defs = resolve_effective_agents(cfg, settings, catalog)
        .into_iter()
        .map(|eff| {
            let resolved = &eff.execution_provider;
            let mut description = eff.description;
            if eff.allowed_tools.iter().any(|t| t == "video_create") {
                let vc = super::media_backend::resolve_media_tool_provider(
                    catalog,
                    Some(settings),
                    "video_create",
                    "video",
                    true,
                )
                .map(|p| p.video_constraints())
                .unwrap_or_default();
                description = format!(
                    "{description} [Active video_create provider: {}; max {}s/clip, up to {} clip(s) = {}s per call; merge multi-clip outputs with video_concat]",
                    vc.provider_label,
                    vc.max_clip_duration_secs,
                    vc.max_clips_per_call,
                    vc.max_total_duration_secs
                );
            }
            let mut def = AgentDefinition::new(&eff.id, &resolved.provider_ref, &resolved.model)
                .with_mode(eff.mode)
                .with_description(description)
                .with_prompt_path(cfg.prompts_dir.join(format!("{}.md", eff.id)))
                .with_context_limit_tokens(resolved.context_limit_tokens)
                .with_max_output_tokens(resolved.max_tokens)
                .with_allowed_tools(eff.allowed_tools)
                .with_allowed_skills(eff.allowed_skills)
                .with_steps(eff.steps);

            if let Some(pct) = resolved.temperature_percent {
                def = def.with_temperature((f32::from(pct) / 100.0).clamp(0.0, 2.0));
            }
            if let Some(ref effort) = resolved.reasoning_effort {
                match effort.as_str() {
                    "low" => def = def.with_reasoning_effort(ReasoningEffort::Low),
                    "medium" => def = def.with_reasoning_effort(ReasoningEffort::Medium),
                    "high" => def = def.with_reasoning_effort(ReasoningEffort::High),
                    _ => {}
                }
            } else if resolved.supports_tag("reasoning") {
                def = def.with_reasoning_effort(ReasoningEffort::Medium);
            }
            if eff.mode == AgentMode::Primary {
                def = def.with_allowed_subagents(eff.allowed_subagents);
            }
            def
        })
        .collect();

    (providers, defs)
}

pub fn project_agents_json(
    cfg: &OrchestratorSectionConfig,
    settings: &PersistedBackgroundSettings,
    catalog: &[ResolvedLlmProvider],
) -> Vec<Value> {
    resolve_effective_agents(cfg, settings, catalog)
        .into_iter()
        .map(|eff| {
            let tool_capability_tags: BTreeMap<&str, &str> =
                eff.tool_capability_tags.iter().copied().collect();
            let tool_providers: BTreeMap<String, String> = settings
                .agents
                .get(&eff.id)
                .and_then(|o| o.tool_providers.as_ref())
                .map(|m| {
                    m.iter()
                        .filter(|(t, p)| {
                            !p.trim().is_empty()
                                && eff
                                    .tool_capability_tags
                                    .iter()
                                    .any(|(tool, _)| *tool == t.as_str())
                        })
                        .map(|(t, p)| (t.clone(), p.trim().to_owned()))
                        .collect()
                })
                .unwrap_or_default();
            let effective_tool_providers: BTreeMap<&str, Option<String>> = eff
                .tool_capability_tags
                .iter()
                .map(|(tool_name, required_tag)| {
                    let resolved = resolve_media_tool_provider(
                        catalog,
                        Some(settings),
                        tool_name,
                        required_tag,
                        false,
                    )
                    .map(|p| p.provider_ref);
                    (*tool_name, resolved)
                })
                .collect();
            json!({
                "id": eff.id,
                "flow_id": eff.flow_id,
                "parent_id": eff.parent_id,
                "mode": match eff.mode {
                    AgentMode::Primary => "primary",
                    AgentMode::Subagent => "subagent",
                    AgentMode::All => "all",
                },
                "is_builtin": eff.is_builtin,
                "enabled": eff.enabled,
                "provider_ref": eff.explicit_ref,
                "effective_provider_ref": eff.resolved_provider.provider_ref,
                "effective_model": eff.resolved_provider.model,
                "tool_capability_tags": tool_capability_tags,
                "tool_providers": tool_providers,
                "effective_tool_providers": effective_tool_providers,
                "steps": eff.steps,
                "default_steps": eff.default_steps,
                "allowed_tools": eff.allowed_tools,
                "default_allowed_tools": eff.default_allowed_tools,
                "available_tools": eff.available_tools,
                "allowed_skills": eff.allowed_skills,
                "default_allowed_skills": eff.default_allowed_skills,
                "available_skills": eff.available_skills,
                "allowed_subagents": eff.allowed_subagents,
                "allowed_capability_tags": eff.allowed_capability_tags,
                "description": eff.description,
                "default_description": eff.default_description,
                "system_prompt": eff.system_prompt,
                "default_system_prompt": eff.default_system_prompt,
            })
        })
        .collect()
}
