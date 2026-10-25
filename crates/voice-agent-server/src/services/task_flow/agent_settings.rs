//! Persistent & live configuration for Flow 1 (`wiki_root`) & Flow 2 (`daily_root`) Root Agents
//! and their 7 specialist Subagents, integrated with the Server LLM Provider Catalog.

use std::collections::{BTreeMap, HashSet};
use std::path::Path;
use std::time::Duration;

use orchestrator_agent_rs::{
    AgentDefinition, AgentMode, LlmBackend, LlmProviderConfig, ProviderAuth, ReasoningEffort,
    SecretString,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::workspace_seed::{
    DAILY_ROOT_PROMPT, MEDIA_CREATOR_PROMPT, REMINDER_PLANNER_PROMPT, REPORT_STAGER_PROMPT,
    SOURCE_COLLECTOR_PROMPT, WIKI_CURATOR_PROMPT, WIKI_INGESTOR_PROMPT, WIKI_RESEARCHER_PROMPT,
    WIKI_ROOT_PROMPT,
};
use crate::app::AppState;
use crate::config::{LlmInstanceConfig, OrchestratorSectionConfig};

pub const DEFAULT_ORCHESTRATOR_PROVIDER_REF: &str = "orchestrator:default";
const AGENT_SETTINGS_FILE: &str = "agent_settings.json";

#[derive(Clone, Copy, Debug)]
pub struct AgentSpec {
    pub id: &'static str,
    pub flow_id: &'static str,
    pub parent_id: Option<&'static str>,
    pub mode: AgentMode,
    pub default_description: &'static str,
    pub default_prompt: &'static str,
    pub default_steps: u32,
    pub default_allowed_tools: &'static [&'static str],
    pub available_tools: &'static [&'static str],
    pub default_allowed_skills: &'static [&'static str],
    pub available_skills: &'static [&'static str],
    pub default_subagents: &'static [&'static str],
}

pub const AGENT_SPECS: &[AgentSpec] = &[
    // === Flow 2: Interactive & Daily Background Assistant (`daily_root`) ===
    AgentSpec {
        id: "daily_root",
        flow_id: "daily",
        parent_id: None,
        mode: AgentMode::Primary,
        default_description: "Interactive & background daily assistant: researches Wiki/Web, creates reports, generates images/TTS artifacts, stages knowledge to sources/, and manages reminders.",
        default_prompt: DAILY_ROOT_PROMPT,
        default_steps: 15,
        default_allowed_tools: &["subagent"],
        available_tools: &["subagent"],
        default_allowed_skills: &[],
        available_skills: &[],
        default_subagents: &[
            "wiki_researcher",
            "report_stager",
            "media_creator",
            "reminder_planner",
        ],
    },
    AgentSpec {
        id: "wiki_researcher",
        flow_id: "daily",
        parent_id: Some("daily_root"),
        mode: AgentMode::Subagent,
        default_description: "Read-only specialist for querying and reading the LLM Wiki and inspecting web pages.",
        default_prompt: WIKI_RESEARCHER_PROMPT,
        default_steps: 12,
        default_allowed_tools: &[
            "skill",
            "wiki_tree",
            "wiki_query",
            "wiki_get",
            "wiki_multi_get",
            "wiki_metadata",
            "wiki_status",
            "wiki_graph_neighbors",
            "fetch_html",
            "html_extract_metadata",
            "html_extract_article",
            "html_extract_feed",
            "html_extract_links",
            "html_select_elements",
        ],
        available_tools: &[
            "skill",
            "wiki_tree",
            "wiki_query",
            "wiki_get",
            "wiki_multi_get",
            "wiki_metadata",
            "wiki_status",
            "wiki_graph_neighbors",
            "fetch_html",
            "html_extract_metadata",
            "html_extract_article",
            "html_extract_feed",
            "html_extract_links",
            "html_select_elements",
        ],
        default_allowed_skills: &["wiki-query"],
        available_skills: &["wiki-query", "wiki-taxonomy"],
        default_subagents: &[],
    },
    AgentSpec {
        id: "report_stager",
        flow_id: "daily",
        parent_id: Some("daily_root"),
        mode: AgentMode::Subagent,
        default_description: "Synthesizes discussions into sources/<domain>/<topic>/ via add_to_sources, writes standalone reports via markdown_create, or publishes accompanying image/TTS artifacts.",
        default_prompt: REPORT_STAGER_PROMPT,
        default_steps: 12,
        default_allowed_tools: &[
            "skill",
            "wiki_tree",
            "wiki_get",
            "add_to_sources",
            "markdown_create",
            "markdown_read",
            "image_create",
            "tts_synthesize",
            "artifact_create",
        ],
        available_tools: &[
            "skill",
            "wiki_tree",
            "wiki_get",
            "add_to_sources",
            "markdown_create",
            "markdown_read",
            "image_create",
            "tts_synthesize",
            "artifact_create",
        ],
        default_allowed_skills: &["wiki-taxonomy", "daily-synthesis-staging"],
        available_skills: &["wiki-taxonomy", "daily-synthesis-staging", "wiki-query"],
        default_subagents: &[],
    },
    AgentSpec {
        id: "media_creator",
        flow_id: "daily",
        parent_id: Some("daily_root"),
        mode: AgentMode::Subagent,
        default_description: "Generates PNG/SVG images via image_create, synthesizes TTS audio via tts_synthesize, and publishes multi-format Task Artifacts via artifact_create / markdown_create.",
        default_prompt: MEDIA_CREATOR_PROMPT,
        default_steps: 12,
        default_allowed_tools: &[
            "skill",
            "image_create",
            "tts_synthesize",
            "artifact_create",
            "markdown_create",
            "markdown_read",
        ],
        available_tools: &[
            "skill",
            "image_create",
            "tts_synthesize",
            "artifact_create",
            "markdown_create",
            "markdown_read",
        ],
        default_allowed_skills: &[],
        available_skills: &["daily-synthesis-staging"],
        default_subagents: &[],
    },
    AgentSpec {
        id: "reminder_planner",
        flow_id: "daily",
        parent_id: Some("daily_root"),
        mode: AgentMode::Subagent,
        default_description: "Creates, lists, completes, and deletes user reminders and scheduled tasks.",
        default_prompt: REMINDER_PLANNER_PROMPT,
        default_steps: 10,
        default_allowed_tools: &[
            "skill",
            "reminder_create",
            "reminder_list",
            "reminder_complete",
            "reminder_delete",
        ],
        available_tools: &[
            "skill",
            "reminder_create",
            "reminder_list",
            "reminder_complete",
            "reminder_delete",
        ],
        default_allowed_skills: &["daily-task-reminder"],
        available_skills: &["daily-task-reminder"],
        default_subagents: &[],
    },
    // === Flow 1: Background Karpathy LLM Wiki Pipeline (`wiki_root`) ===
    AgentSpec {
        id: "wiki_root",
        flow_id: "wiki",
        parent_id: None,
        mode: AgentMode::Primary,
        default_description: "Background Wiki pipeline coordinator: collects external sources (YouTube playlists, PDFs, Web pages), ingests new files from sources/, updates/patches Wiki pages, synthesizes topics, and runs wiki_lint.",
        default_prompt: WIKI_ROOT_PROMPT,
        default_steps: 40,
        default_allowed_tools: &["subagent"],
        available_tools: &["subagent"],
        default_allowed_skills: &[],
        available_skills: &[],
        default_subagents: &["source_collector", "wiki_ingestor", "wiki_curator"],
    },
    AgentSpec {
        id: "source_collector",
        flow_id: "wiki",
        parent_id: Some("wiki_root"),
        mode: AgentMode::Subagent,
        default_description: "Inspects external sources (YouTube playlist, PDF via anydoc, or Web via fetch_html/html_*), extracts detailed transcripts/markdown/metadata, classifies topic hierarchy (<domain>/<topic>), and saves raw Markdown files into sources/<domain>/<topic>/<slug>.md.",
        default_prompt: SOURCE_COLLECTOR_PROMPT,
        default_steps: 35,
        default_allowed_tools: &[
            "skill",
            "anydoc_inspect_pdf",
            "anydoc_save_to_sources",
            "yt_list_playlist",
            "yt_fetch_transcript",
            "fetch_html",
            "html_extract_metadata",
            "html_extract_article",
            "html_extract_feed",
            "html_extract_links",
            "html_select_elements",
            "add_to_sources",
            "wiki_tree",
        ],
        available_tools: &[
            "skill",
            "anydoc_inspect_pdf",
            "anydoc_save_to_sources",
            "yt_list_playlist",
            "yt_fetch_transcript",
            "fetch_html",
            "html_extract_metadata",
            "html_extract_article",
            "html_extract_feed",
            "html_extract_links",
            "html_select_elements",
            "add_to_sources",
            "wiki_tree",
        ],
        default_allowed_skills: &["wiki-taxonomy"],
        available_skills: &["wiki-taxonomy", "wiki-ingest"],
        default_subagents: &[],
    },
    AgentSpec {
        id: "wiki_ingestor",
        flow_id: "wiki",
        parent_id: Some("wiki_root"),
        mode: AgentMode::Subagent,
        default_description: "Compiles raw sources into wiki/sources/ and creates/patches entity & concept pages.",
        default_prompt: WIKI_INGESTOR_PROMPT,
        default_steps: 35,
        default_allowed_tools: &[
            "skill",
            "wiki_tree",
            "wiki_get",
            "wiki_multi_get",
            "wiki_query",
            "wiki_metadata",
            "wiki_ingest_source",
            "wiki_write_file",
            "wiki_patch_file",
        ],
        available_tools: &[
            "skill",
            "wiki_tree",
            "wiki_get",
            "wiki_multi_get",
            "wiki_query",
            "wiki_metadata",
            "wiki_ingest_source",
            "wiki_write_file",
            "wiki_patch_file",
        ],
        default_allowed_skills: &["wiki-taxonomy", "wiki-ingest"],
        available_skills: &["wiki-taxonomy", "wiki-ingest", "wiki-query"],
        default_subagents: &[],
    },
    AgentSpec {
        id: "wiki_curator",
        flow_id: "wiki",
        parent_id: Some("wiki_root"),
        mode: AgentMode::Subagent,
        default_description: "Creates topic syntheses and repairs structural issues via wiki_lint.",
        default_prompt: WIKI_CURATOR_PROMPT,
        default_steps: 35,
        default_allowed_tools: &[
            "skill",
            "wiki_tree",
            "wiki_get",
            "wiki_multi_get",
            "wiki_query",
            "wiki_file_synthesis",
            "wiki_write_file",
            "wiki_patch_file",
            "wiki_delete_file",
            "wiki_lint",
            "wiki_schema",
        ],
        available_tools: &[
            "skill",
            "wiki_tree",
            "wiki_get",
            "wiki_multi_get",
            "wiki_query",
            "wiki_file_synthesis",
            "wiki_write_file",
            "wiki_patch_file",
            "wiki_delete_file",
            "wiki_lint",
            "wiki_schema",
        ],
        default_allowed_skills: &["wiki-taxonomy", "wiki-query", "wiki-lint"],
        available_skills: &["wiki-taxonomy", "wiki-query", "wiki-lint", "wiki-ingest"],
        default_subagents: &[],
    },
];

pub fn find_agent_spec(id: &str) -> Option<&'static AgentSpec> {
    AGENT_SPECS.iter().find(|s| s.id == id)
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct AgentConfigOverride {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_ref: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub steps: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allowed_tools: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allowed_skills: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allowed_subagents: Option<Vec<String>>,
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

pub fn effective_settings_dir(config: &crate::config::AppConfig) -> std::path::PathBuf {
    if config.orchestrator.enabled
        || config.orchestrator.store_dir != Path::new("data/orchestrator-store")
    {
        return config.orchestrator.store_dir.clone();
    }
    if let Some(db_path) = config.database.url.strip_prefix("sqlite://")
        && let Some(parent) = Path::new(db_path).parent()
        && !parent.as_os_str().is_empty()
    {
        return parent.to_path_buf();
    }
    config.orchestrator.store_dir.clone()
}

pub fn load_persisted_settings(store_dir: &Path) -> Option<PersistedBackgroundSettings> {
    let path = store_dir.join(AGENT_SETTINGS_FILE);
    let raw = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&raw).ok()
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

#[derive(Clone, Debug)]
pub struct ResolvedLlmProvider {
    pub provider_ref: String,
    pub name: String,
    pub source: &'static str,
    pub adapter: String,
    pub model: String,
    pub base_url: String,
    pub api_key: Option<String>,
    pub timeout_ms: u64,
    pub max_tokens: u32,
    pub context_limit_tokens: usize,
    pub reasoning_effort: Option<String>,
    pub temperature_percent: Option<u16>,
}

impl ResolvedLlmProvider {
    pub fn to_json_summary(&self) -> Value {
        json!({
            "ref": self.provider_ref,
            "name": self.name,
            "source": self.source,
            "adapter": self.adapter,
            "model": self.model,
            "base_url": self.base_url,
            "timeout_ms": self.timeout_ms,
            "max_tokens": self.max_tokens,
            "context_limit_tokens": self.context_limit_tokens,
            "reasoning_effort": self.reasoning_effort,
            "temperature_percent": self.temperature_percent,
        })
    }

    pub fn to_orchestrator_provider(&self) -> LlmProviderConfig {
        let auth = match &self.api_key {
            Some(key) if !key.trim().is_empty() => {
                ProviderAuth::ApiKey(SecretString::new(key.clone()))
            }
            _ => ProviderAuth::None,
        };
        LlmProviderConfig {
            id: self.provider_ref.clone(),
            backend: LlmBackend::OpenAiCompatible,
            endpoint: Some(self.base_url.clone()),
            auth,
            request_timeout: Duration::from_millis(self.timeout_ms.max(1_000)),
        }
    }
}

pub fn default_orchestrator_provider(cfg: &OrchestratorSectionConfig) -> ResolvedLlmProvider {
    ResolvedLlmProvider {
        provider_ref: DEFAULT_ORCHESTRATOR_PROVIDER_REF.to_owned(),
        name: cfg.model.clone(),
        source: "config",
        adapter: "openai".to_owned(),
        model: cfg.model.clone(),
        base_url: cfg.endpoint.clone(),
        api_key: (!cfg.api_key.trim().is_empty()).then(|| cfg.api_key.clone()),
        timeout_ms: cfg.request_timeout_ms,
        max_tokens: cfg.max_output_tokens,
        context_limit_tokens: cfg.context_limit_tokens,
        reasoning_effort: None,
        temperature_percent: None,
    }
}

pub async fn resolve_llm_provider_catalog(state: &AppState) -> Vec<ResolvedLlmProvider> {
    let orch_cfg = &state.config.orchestrator;
    let mut out = vec![default_orchestrator_provider(orch_cfg)];

    // 1. Deployment config LLM instances (`config:llm:<key>`)
    let mut config_keys: Vec<&String> = state.config.providers.llm.instances.keys().collect();
    config_keys.sort();
    for key in config_keys {
        let Some(instance) = state.config.providers.llm.instances.get(key) else {
            continue;
        };
        let provider_ref = format!("config:llm:{key}");
        match instance {
            LlmInstanceConfig::Openai(openai) => {
                out.push(ResolvedLlmProvider {
                    provider_ref,
                    name: format!("{key} ({})", openai.model),
                    source: "config",
                    adapter: "openai".to_owned(),
                    model: openai.model.clone(),
                    base_url: openai.base_url.to_string(),
                    api_key: (!openai.api_key.expose().trim().is_empty())
                        .then(|| openai.api_key.expose().to_owned()),
                    timeout_ms: openai.timeout_ms,
                    max_tokens: openai.max_tokens.unwrap_or(orch_cfg.max_output_tokens),
                    context_limit_tokens: openai
                        .context_limit_tokens
                        .map(|v| v as usize)
                        .unwrap_or(orch_cfg.context_limit_tokens),
                    reasoning_effort: openai.reasoning_effort.clone(),
                    temperature_percent: openai.temperature_percent,
                });
            }
            LlmInstanceConfig::OpenCodeFree(free) => {
                out.push(ResolvedLlmProvider {
                    provider_ref,
                    name: format!("{key} ({})", free.model),
                    source: "config",
                    adapter: "opencode_free".to_owned(),
                    model: free.model.clone(),
                    base_url: crate::providers::llm::opencode_free::BASE_URL.to_owned(),
                    api_key: Some(crate::providers::llm::opencode_free::PUBLIC_KEY.to_owned()),
                    timeout_ms: free.timeout_ms,
                    max_tokens: free.max_tokens.unwrap_or(orch_cfg.max_output_tokens),
                    context_limit_tokens: free
                        .context_limit_tokens
                        .map(|v| v as usize)
                        .unwrap_or(orch_cfg.context_limit_tokens),
                    reasoning_effort: free.reasoning_effort.clone(),
                    temperature_percent: free.temperature_percent,
                });
            }
            #[cfg(feature = "qualification-providers")]
            LlmInstanceConfig::QualificationLlm(_) => {}
        }
    }

    // 2. Database LLM providers (`providers` table where `type='llm' AND enabled=1`)
    if let Some(database) = &state.database
        && let Ok(rows) = sqlx::query_as::<_, (String, String, String, String, Option<String>)>(
            "SELECT key, name, adapter, config_json, credential_json FROM providers WHERE type='llm' AND enabled=1 ORDER BY id",
        )
        .fetch_all(database.pool())
        .await
    {
        for (key, name, adapter, config_json, credential_json) in rows {
            let Ok(parsed) = serde_json::from_str::<Value>(&config_json) else {
                continue;
            };
            let Some(model) = parsed
                .get("model")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_owned)
            else {
                continue;
            };
            let timeout_ms = parsed
                .get("timeout_ms")
                .and_then(Value::as_u64)
                .unwrap_or(orch_cfg.request_timeout_ms);
            let max_tokens = parsed
                .get("max_tokens")
                .and_then(Value::as_u64)
                .and_then(|v| u32::try_from(v).ok())
                .unwrap_or(orch_cfg.max_output_tokens);
            let context_limit_tokens = parsed
                .get("context_limit_tokens")
                .and_then(Value::as_u64)
                .and_then(|v| usize::try_from(v).ok())
                .unwrap_or(orch_cfg.context_limit_tokens);
            let reasoning_effort = parsed
                .get("reasoning_effort")
                .and_then(Value::as_str)
                .map(str::to_owned);
            let temperature_percent = parsed
                .get("temperature_percent")
                .and_then(Value::as_u64)
                .and_then(|v| u16::try_from(v).ok());

            let (base_url, api_key) = match adapter.as_str() {
                "openai" => {
                    let Some(url) = parsed
                        .get("base_url")
                        .and_then(Value::as_str)
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                        .map(str::to_owned)
                    else {
                        continue;
                    };
                    let secret_ref = crate::database::credentials::reference(
                        &format!("provider:{key}"),
                        credential_json.as_deref(),
                        crate::database::secrets::provider_secret_env(&key, &adapter),
                    );
                    let resolved_key = secret_ref
                        .and_then(|r| crate::database::secrets::SecretRef::parse(r).ok())
                        .and_then(|s| state.secret_resolver.resolve(&s).ok())
                        .map(|v| v.expose().to_owned());
                    (url, resolved_key)
                }
                "opencode_free" => (
                    crate::providers::llm::opencode_free::BASE_URL.to_owned(),
                    Some(crate::providers::llm::opencode_free::PUBLIC_KEY.to_owned()),
                ),
                _ => continue,
            };

            out.push(ResolvedLlmProvider {
                provider_ref: key,
                name,
                source: "database",
                adapter,
                model,
                base_url,
                api_key,
                timeout_ms,
                max_tokens,
                context_limit_tokens,
                reasoning_effort,
                temperature_percent,
            });
        }
    }

    out
}

struct ResolvedAgentSettings<'a> {
    spec: &'static AgentSpec,
    enabled: bool,
    explicit_ref: Option<String>,
    resolved_provider: ResolvedLlmProvider,
    steps: u32,
    allowed_tools: Vec<String>,
    allowed_skills: Vec<String>,
    allowed_subagents: Vec<String>,
    description: &'a str,
    system_prompt: &'a str,
}

fn resolve_effective_agents<'a>(
    cfg: &OrchestratorSectionConfig,
    settings: &'a PersistedBackgroundSettings,
    catalog: &[ResolvedLlmProvider],
) -> Vec<ResolvedAgentSettings<'a>> {
    let fallback = catalog
        .first()
        .cloned()
        .unwrap_or_else(|| default_orchestrator_provider(cfg));
    let find_provider = |pref: &str| -> ResolvedLlmProvider {
        catalog
            .iter()
            .find(|p| p.provider_ref == pref)
            .cloned()
            .unwrap_or_else(|| fallback.clone())
    };

    AGENT_SPECS
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
            let resolved_provider = find_provider(effective_ref);
            let steps = ovr
                .and_then(|o| o.steps)
                .unwrap_or(spec.default_steps)
                .clamp(1, 100);
            let allowed_skills: Vec<String> = match ovr.and_then(|o| o.allowed_skills.as_ref()) {
                Some(list) => list
                    .iter()
                    .filter(|s| spec.available_skills.contains(&s.as_str()))
                    .cloned()
                    .collect(),
                None => spec
                    .default_allowed_skills
                    .iter()
                    .map(|s| (*s).to_owned())
                    .collect(),
            };
            let allowed_tools: Vec<String> = if spec.mode == AgentMode::Primary {
                vec!["subagent".to_owned()]
            } else {
                let mut tools: Vec<String> = match ovr.and_then(|o| o.allowed_tools.as_ref()) {
                    Some(list) => list
                        .iter()
                        .filter(|t| spec.available_tools.contains(&t.as_str()))
                        .cloned()
                        .collect(),
                    None => spec
                        .default_allowed_tools
                        .iter()
                        .map(|t| (*t).to_owned())
                        .collect(),
                };
                if !allowed_skills.is_empty() && !tools.iter().any(|t| t == "skill") {
                    tools.insert(0, "skill".to_owned());
                }
                tools
            };
            let allowed_subagents: Vec<String> = if spec.mode == AgentMode::Primary {
                spec.default_subagents
                    .iter()
                    .filter(|sub_id| {
                        settings
                            .agents
                            .get(**sub_id)
                            .is_none_or(|sub_ovr| sub_ovr.enabled)
                    })
                    .map(|s| (*s).to_owned())
                    .collect()
            } else {
                Vec::new()
            };
            let description = ovr
                .and_then(|o| o.description.as_deref())
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .unwrap_or(spec.default_description);
            let system_prompt = ovr
                .and_then(|o| o.system_prompt.as_deref())
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .unwrap_or(spec.default_prompt);

            ResolvedAgentSettings {
                spec,
                enabled,
                explicit_ref,
                resolved_provider,
                steps,
                allowed_tools,
                allowed_skills,
                allowed_subagents,
                description,
                system_prompt,
            }
        })
        .collect()
}

pub fn build_agent_definitions(
    cfg: &OrchestratorSectionConfig,
    settings: &PersistedBackgroundSettings,
    catalog: &[ResolvedLlmProvider],
) -> (Vec<LlmProviderConfig>, Vec<AgentDefinition>) {
    let providers: Vec<LlmProviderConfig> = catalog
        .iter()
        .map(ResolvedLlmProvider::to_orchestrator_provider)
        .collect();

    let defs = resolve_effective_agents(cfg, settings, catalog)
        .into_iter()
        .map(|eff| {
            let resolved = &eff.resolved_provider;
            let mut def = AgentDefinition::new(eff.spec.id, &resolved.provider_ref, &resolved.model)
                .with_mode(eff.spec.mode)
                .with_description(eff.description)
                .with_prompt_path(cfg.prompts_dir.join(format!("{}.md", eff.spec.id)))
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
            }
            if eff.spec.mode == AgentMode::Primary {
                def = def.with_allowed_subagents(eff.allowed_subagents);
            }
            def
        })
        .collect();

    (providers, defs)
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

pub fn project_agents_json(
    cfg: &OrchestratorSectionConfig,
    settings: &PersistedBackgroundSettings,
    catalog: &[ResolvedLlmProvider],
) -> Vec<Value> {
    resolve_effective_agents(cfg, settings, catalog)
        .into_iter()
        .map(|eff| {
            json!({
                "id": eff.spec.id,
                "flow_id": eff.spec.flow_id,
                "parent_id": eff.spec.parent_id,
                "mode": match eff.spec.mode {
                    AgentMode::Primary => "primary",
                    AgentMode::Subagent => "subagent",
                    AgentMode::All => "all",
                },
                "enabled": eff.enabled,
                "provider_ref": eff.explicit_ref,
                "effective_provider_ref": eff.resolved_provider.provider_ref,
                "effective_model": eff.resolved_provider.model,
                "steps": eff.steps,
                "default_steps": eff.spec.default_steps,
                "allowed_tools": eff.allowed_tools,
                "default_allowed_tools": eff.spec.default_allowed_tools,
                "available_tools": eff.spec.available_tools,
                "allowed_skills": eff.allowed_skills,
                "default_allowed_skills": eff.spec.default_allowed_skills,
                "available_skills": eff.spec.available_skills,
                "allowed_subagents": eff.allowed_subagents,
                "description": eff.description,
                "default_description": eff.spec.default_description,
                "system_prompt": eff.system_prompt,
                "default_system_prompt": eff.spec.default_prompt,
            })
        })
        .collect()
}

pub fn validate_agent_overrides(
    orchestrator_provider: &str,
    worker_provider: &str,
    overrides: &BTreeMap<String, AgentConfigOverride>,
    catalog: &[ResolvedLlmProvider],
) -> Result<(), &'static str> {
    let valid_refs: HashSet<&str> = catalog.iter().map(|p| p.provider_ref.as_str()).collect();
    if !valid_refs.contains(orchestrator_provider.trim())
        || !valid_refs.contains(worker_provider.trim())
    {
        return Err("invalid_provider");
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
            if !trimmed.is_empty() && !valid_refs.contains(trimmed) {
                return Err("invalid_provider");
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
        if let Some(ref skills) = ovr.allowed_skills
            && skills
                .iter()
                .any(|s| !spec.available_skills.contains(&s.as_str()))
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
    let Some(orch) = state.orchestrator.as_ref() else {
        return Ok(());
    };
    let catalog = resolve_llm_provider_catalog(state).await;
    let (providers, defs) = build_agent_definitions(&state.config.orchestrator, &settings, &catalog);
    orch.reconfigure(providers, defs)
        .await
        .map_err(|e| e.to_string())
}
