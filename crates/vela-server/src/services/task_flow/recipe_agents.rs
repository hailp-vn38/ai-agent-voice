//! Agent definitions the server-owned recipes need: one primary recipe root per recipe, plus the
//! sub-agents its Agent Steps delegate to.

use std::collections::BTreeSet;
use std::path::Path;

use orchestrator_agent_rs::{AgentDefinition, AgentMode};
use orchestrator_flow_rs::{RecipeMetadata, RecipeStepKind};

use super::TaskFlowError;
use super::recipes::{PLAYLIST_COMPLETION_TOOL, PLAYLIST_DOMAIN_SELECTOR, PLAYLIST_RECIPE_ROOT};

/// The selector calls its completion tool once and then answers; the rest is retry room.
const SELECTOR_MAX_STEPS: u32 = 4;
const SELECTOR_CONTEXT_LIMIT_TOKENS: usize = 32_000;
/// The recipe root makes no LLM calls. A high limit keeps auto-compaction from dropping the tool
/// results that `TaskRecipe::next` reads back from the transcript.
const RECIPE_ROOT_CONTEXT_LIMIT_TOKENS: usize = 1_000_000;

/// Host tools a recipe root may call: every tool its outline names, plus `subagent` when it has
/// an Agent Step.
fn recipe_tools(metadata: &RecipeMetadata) -> BTreeSet<String> {
    let mut tools = metadata
        .outline
        .iter()
        .flat_map(|step| step.host_tools.iter().cloned())
        .collect::<BTreeSet<_>>();
    if metadata
        .outline
        .iter()
        .any(|step| step.kind == RecipeStepKind::Agent)
    {
        tools.insert("subagent".to_owned());
    }
    tools
}

fn recipe_subagents(metadata: &RecipeMetadata) -> BTreeSet<String> {
    metadata
        .outline
        .iter()
        .flat_map(|step| step.sub_agents.iter().cloned())
        .collect()
}

/// Agents for the playlist recipe. The selector reuses `daily_root`'s provider and model.
pub(super) fn playlist_recipe_agents(
    prompts_dir: &Path,
    agents: &[AgentDefinition],
    metadata: &RecipeMetadata,
) -> Result<[AgentDefinition; 2], TaskFlowError> {
    let selector_prompt = prompts_dir.join(format!("{PLAYLIST_DOMAIN_SELECTOR}.md"));
    if !selector_prompt.exists() {
        std::fs::write(
            &selector_prompt,
            format!(
                "You classify playlist sources into a Wiki domain and topic. Use only `{PLAYLIST_COMPLETION_TOOL}` as your completion tool. After calling it, return its canonical JSON output as your entire final response, with no prose or markdown. The delegated user message contains the allowed domains and task details."
            ),
        )?;
    }
    let (selector_provider, selector_model) = agents
        .iter()
        .find(|agent| agent.id == "daily_root")
        .map(|agent| (agent.provider_id.clone(), agent.model_id.clone()))
        .ok_or_else(|| {
            TaskFlowError::Config("daily_root agent is unavailable for recipe classifier".into())
        })?;
    let selector =
        AgentDefinition::new(PLAYLIST_DOMAIN_SELECTOR, selector_provider, selector_model)
            .with_mode(AgentMode::Subagent)
            .with_description("Selects a Wiki source domain and topic for a playlist recipe")
            .with_prompt_path(selector_prompt)
            .with_allowed_tools([PLAYLIST_COMPLETION_TOOL])
            .with_steps(SELECTOR_MAX_STEPS)
            .with_context_limit_tokens(SELECTOR_CONTEXT_LIMIT_TOKENS);
    let root = AgentDefinition::new(
        PLAYLIST_RECIPE_ROOT,
        "recipe",
        format!("{}@{}", metadata.id, metadata.version),
    )
    .with_mode(AgentMode::Primary)
    .with_description("Runs the fixed YouTube source collection recipe")
    .with_prompt_path(prompts_dir.join("daily_root.md"))
    .with_allowed_tools(recipe_tools(metadata))
    .with_allowed_subagents(recipe_subagents(metadata))
    .with_steps(metadata.max_steps)
    .with_context_limit_tokens(RECIPE_ROOT_CONTEXT_LIMIT_TOKENS);
    Ok([selector, root])
}
