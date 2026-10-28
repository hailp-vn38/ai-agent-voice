//! Canonical defaults for Flow 1 (`daily_root`) & Flow 2 (`wiki_root`) Root Agents
//! and their 7 specialist Subagents.

use orchestrator_agent_rs::AgentMode;

use crate::services::task_flow::workspace_seed::{
    DAILY_ROOT_PROMPT, MEDIA_CREATOR_PROMPT, REMINDER_PLANNER_PROMPT, REPORT_STAGER_PROMPT,
    SOURCE_COLLECTOR_PROMPT, WIKI_CURATOR_PROMPT, WIKI_INGESTOR_PROMPT, WIKI_RESEARCHER_PROMPT,
    WIKI_ROOT_PROMPT,
};

/// Default user-facing Root Agent ID for Flow 1 (`daily_root`).
pub const DEFAULT_USER_ROOT_AGENT_ID: &str = "daily_root";

/// Default automated Wiki pipeline Root Agent ID for Flow 2 (`wiki_root`).
pub const WIKI_AUTOFLOW_ROOT_AGENT_ID: &str = "wiki_root";

#[derive(Clone, Copy, Debug)]
pub struct AgentSpec {
    pub id: &'static str,
    pub flow_id: &'static str,
    pub parent_id: Option<&'static str>,
    pub mode: AgentMode,
    /// Whether user/Voice tools (`task_create`, `reminder_create`) may directly target this root agent.
    /// `true` for Flow 1 (`daily_root`), `false` for internal Flow 2 (`wiki_root`) and all subagents.
    pub user_routable: bool,
    pub default_description: &'static str,
    pub default_prompt: &'static str,
    pub default_steps: u32,
    pub default_allowed_tools: &'static [&'static str],
    pub available_tools: &'static [&'static str],
    pub default_allowed_skills: &'static [&'static str],
    pub suggested_skills: &'static [&'static str],
    pub default_subagents: &'static [&'static str],
}

impl AgentSpec {
    pub const fn allowed_capability_tags(&self) -> &'static [&'static str] {
        match self.id.as_bytes() {
            b"media_creator" => &["chat", "reasoning", "image", "video"],
            _ => &["chat", "reasoning"],
        }
    }

    /// Maps media/specialized tools on this agent to their required provider capability tag
    /// (e.g. `"image_create"` -> `"image"`, `"video_create"` -> `"video"`, `"tts_synthesize"` -> `"tts"`).
    pub const fn tool_capability_tags(&self) -> &'static [(&'static str, &'static str)] {
        match self.id.as_bytes() {
            b"media_creator" => &[
                ("image_create", "image"),
                ("video_create", "video"),
                ("tts_synthesize", "tts"),
            ],
            _ => &[],
        }
    }

    pub fn tool_required_tag(&self, tool_name: &str) -> Option<&'static str> {
        self.tool_capability_tags()
            .iter()
            .find(|(tool, _)| *tool == tool_name)
            .map(|(_, tag)| *tag)
    }
}

const SOURCE_COLLECTOR_TOOLS: &[&str] = &[
    "skill",
    "anydoc_inspect_pdf",
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
];

const WIKI_RESEARCHER_TOOLS: &[&str] = &[
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
];

const REPORT_STAGER_TOOLS: &[&str] = &[
    "skill",
    "wiki_tree",
    "wiki_get",
    "add_to_sources",
    "markdown_create",
    "markdown_read",
    "chart_write",
    "chart_get",
];

const REMINDER_PLANNER_TOOLS: &[&str] = &[
    "skill",
    "reminder_create",
    "reminder_list",
    "reminder_complete",
    "reminder_delete",
];

const WIKI_INGESTOR_TOOLS: &[&str] = &[
    "skill",
    "wiki_tree",
    "wiki_get",
    "wiki_multi_get",
    "wiki_query",
    "wiki_metadata",
    "wiki_ingest_source",
    "wiki_write_file",
    "wiki_patch_file",
    "chart_write",
    "chart_get",
];

const WIKI_CURATOR_TOOLS: &[&str] = &[
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
    "chart_write",
    "chart_get",
];

pub const AGENT_SPECS: &[AgentSpec] = &[
    // === Flow 1: Information Collection & Multi-Functional Assistant (`daily_root`) ===
    AgentSpec {
        id: "daily_root",
        flow_id: "daily",
        parent_id: None,
        mode: AgentMode::Primary,
        user_routable: true,
        default_description: "Interactive & background multi-functional assistant: inspects and summarizes external sources (YouTube videos/playlists, PDFs, Web pages), researches the Wiki/Web, creates reports, generates images/TTS artifacts, manages reminders, and ONLY saves files into sources/ when explicitly requested by the user.",
        default_prompt: DAILY_ROOT_PROMPT,
        default_steps: 20,
        default_allowed_tools: &["subagent"],
        available_tools: &["subagent"],
        default_allowed_skills: &[],
        suggested_skills: &[],
        default_subagents: &[
            "source_collector",
            "wiki_researcher",
            "report_stager",
            "media_creator",
            "reminder_planner",
        ],
    },
    AgentSpec {
        id: "source_collector",
        flow_id: "daily",
        parent_id: Some("daily_root"),
        mode: AgentMode::Subagent,
        user_routable: false,
        default_description: "Inspects external sources using read-only extraction tools (YouTube videos/playlists via yt_list_playlist/yt_fetch_transcript, PDF via anydoc_inspect_pdf, or Web via fetch_html/html_*) to return structured summaries, deep analyses, and visual keypoints directly, OR calls the unified add_to_sources tool to save into sources/<domain>/<topic>/<slug>.md ONLY when the user explicitly requests saving/adding to sources/ or the Wiki.",
        default_prompt: SOURCE_COLLECTOR_PROMPT,
        default_steps: 35,
        default_allowed_tools: SOURCE_COLLECTOR_TOOLS,
        available_tools: SOURCE_COLLECTOR_TOOLS,
        default_allowed_skills: &["wiki-taxonomy"],
        suggested_skills: &["wiki-taxonomy", "wiki-ingest"],
        default_subagents: &[],
    },
    AgentSpec {
        id: "wiki_researcher",
        flow_id: "daily",
        parent_id: Some("daily_root"),
        mode: AgentMode::Subagent,
        user_routable: false,
        default_description: "Read-only specialist for querying and reading the LLM Wiki and inspecting web pages to return structured summaries, deep analyses with citations, and visual keypoints.",
        default_prompt: WIKI_RESEARCHER_PROMPT,
        default_steps: 12,
        default_allowed_tools: WIKI_RESEARCHER_TOOLS,
        available_tools: WIKI_RESEARCHER_TOOLS,
        default_allowed_skills: &["wiki-query"],
        suggested_skills: &["wiki-query", "wiki-taxonomy"],
        default_subagents: &[],
    },
    AgentSpec {
        id: "report_stager",
        flow_id: "daily",
        parent_id: Some("daily_root"),
        mode: AgentMode::Subagent,
        user_routable: false,
        default_description: "Writes standalone Markdown reports via markdown_create (default), or stages synthesized discussions into sources/<domain>/<topic>/ via add_to_sources ONLY when explicitly asked to save/add to sources/ or the Wiki.",
        default_prompt: REPORT_STAGER_PROMPT,
        default_steps: 12,
        default_allowed_tools: REPORT_STAGER_TOOLS,
        available_tools: REPORT_STAGER_TOOLS,
        default_allowed_skills: &["wiki-taxonomy", "daily-synthesis-staging", "trading-chart"],
        suggested_skills: &[
            "wiki-taxonomy",
            "daily-synthesis-staging",
            "trading-chart",
            "wiki-query",
        ],
        default_subagents: &[],
    },
    AgentSpec {
        id: "media_creator",
        flow_id: "daily",
        parent_id: Some("daily_root"),
        mode: AgentMode::Subagent,
        user_routable: false,
        default_description: "Generates PNG illustrations and SVG vector diagrams/infographics via image_create (with agent-selected or default dimensions), MP4 videos via video_create, concatenates multi-clip MP4 videos into a continuous primary video via video_concat, synthesizes TTS audio via tts_synthesize, and publishes structured Task Artifacts via artifact_create.",
        default_prompt: MEDIA_CREATOR_PROMPT,
        default_steps: 12,
        default_allowed_tools: &[
            "image_create",
            "video_create",
            "video_concat",
            "tts_synthesize",
            "artifact_create",
        ],
        available_tools: &[
            "skill",
            "image_create",
            "video_create",
            "video_concat",
            "tts_synthesize",
            "artifact_create",
        ],
        default_allowed_skills: &[],
        suggested_skills: &["daily-synthesis-staging"],
        default_subagents: &[],
    },
    AgentSpec {
        id: "reminder_planner",
        flow_id: "daily",
        parent_id: Some("daily_root"),
        mode: AgentMode::Subagent,
        user_routable: false,
        default_description: "Creates, lists, completes, and deletes user reminders and scheduled tasks.",
        default_prompt: REMINDER_PLANNER_PROMPT,
        default_steps: 10,
        default_allowed_tools: REMINDER_PLANNER_TOOLS,
        available_tools: REMINDER_PLANNER_TOOLS,
        default_allowed_skills: &["daily-task-reminder"],
        suggested_skills: &["daily-task-reminder"],
        default_subagents: &[],
    },
    // === Flow 2: Automated Karpathy LLM Wiki Pipeline (`wiki_root`) ===
    AgentSpec {
        id: "wiki_root",
        flow_id: "wiki",
        parent_id: None,
        mode: AgentMode::Primary,
        user_routable: false,
        default_description: "Automated background Wiki pipeline coordinator triggered when new files arrive in sources/: ingests new files from sources/, updates/patches Wiki pages, synthesizes topics, and runs wiki_lint.",
        default_prompt: WIKI_ROOT_PROMPT,
        default_steps: 40,
        default_allowed_tools: &["subagent"],
        available_tools: &["subagent"],
        default_allowed_skills: &[],
        suggested_skills: &[],
        default_subagents: &["wiki_ingestor", "wiki_curator"],
    },
    AgentSpec {
        id: "wiki_ingestor",
        flow_id: "wiki",
        parent_id: Some("wiki_root"),
        mode: AgentMode::Subagent,
        user_routable: false,
        default_description: "Compiles raw sources into wiki/sources/ and creates/patches entity & concept pages.",
        default_prompt: WIKI_INGESTOR_PROMPT,
        default_steps: 35,
        default_allowed_tools: WIKI_INGESTOR_TOOLS,
        available_tools: WIKI_INGESTOR_TOOLS,
        default_allowed_skills: &["wiki-taxonomy", "wiki-ingest", "trading-chart"],
        suggested_skills: &["wiki-taxonomy", "wiki-ingest", "trading-chart", "wiki-query"],
        default_subagents: &[],
    },
    AgentSpec {
        id: "wiki_curator",
        flow_id: "wiki",
        parent_id: Some("wiki_root"),
        mode: AgentMode::Subagent,
        user_routable: false,
        default_description: "Creates topic syntheses and repairs structural issues via wiki_lint.",
        default_prompt: WIKI_CURATOR_PROMPT,
        default_steps: 35,
        default_allowed_tools: WIKI_CURATOR_TOOLS,
        available_tools: WIKI_CURATOR_TOOLS,
        default_allowed_skills: &["wiki-taxonomy", "wiki-query", "wiki-lint", "trading-chart"],
        suggested_skills: &[
            "wiki-taxonomy",
            "wiki-query",
            "wiki-lint",
            "trading-chart",
            "wiki-ingest",
        ],
        default_subagents: &[],
    },
];

pub fn find_agent_spec(id: &str) -> Option<&'static AgentSpec> {
    AGENT_SPECS.iter().find(|s| s.id == id)
}

/// Tool pool available to Custom Sub-Agents assigned to Flow 1 (`daily_root`).
pub const CUSTOM_DAILY_SUBAGENT_TOOLS: &[&str] = &[
    "skill",
    "anydoc_inspect_pdf",
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
    "wiki_query",
    "wiki_get",
    "wiki_multi_get",
    "wiki_metadata",
    "wiki_status",
    "wiki_graph_neighbors",
    "markdown_create",
    "markdown_read",
    "chart_write",
    "chart_get",
    "artifact_create",
    "reminder_create",
    "reminder_list",
    "reminder_complete",
    "reminder_delete",
];

/// Tool pool available to Custom Sub-Agents assigned to Flow 2 (`wiki_root`).
/// Strictly limited to Wiki compilation, curation, and lint tools (no external collectors).
pub const CUSTOM_WIKI_SUBAGENT_TOOLS: &[&str] = &[
    "skill",
    "wiki_tree",
    "wiki_get",
    "wiki_multi_get",
    "wiki_query",
    "wiki_metadata",
    "wiki_status",
    "wiki_graph_neighbors",
    "wiki_ingest_source",
    "wiki_file_synthesis",
    "wiki_write_file",
    "wiki_patch_file",
    "wiki_delete_file",
    "wiki_lint",
    "wiki_schema",
    "chart_write",
    "chart_get",
];

/// Returns `(parent_root_id, available_tools)` for a valid Custom Sub-Agent `flow_id`.
pub fn custom_subagent_flow_policy(
    flow_id: &str,
) -> Option<(&'static str, &'static [&'static str])> {
    match flow_id {
        "daily" => Some(("daily_root", CUSTOM_DAILY_SUBAGENT_TOOLS)),
        "wiki" => Some(("wiki_root", CUSTOM_WIKI_SUBAGENT_TOOLS)),
        _ => None,
    }
}

/// Validates that `id` is a legal Custom Sub-Agent identifier (`^[a-z0-9][a-z0-9_-]{0,63}$`)
/// and does not collide with any Built-in Agent or reserved identifier.
pub fn valid_custom_subagent_id(id: &str) -> bool {
    let bytes = id.as_bytes();
    if bytes.is_empty() || bytes.len() > 64 {
        return false;
    }
    if !matches!(bytes[0], b'a'..=b'z' | b'0'..=b'9') {
        return false;
    }
    if !bytes
        .iter()
        .all(|b| matches!(b, b'a'..=b'z' | b'0'..=b'9' | b'_' | b'-'))
    {
        return false;
    }
    if find_agent_spec(id).is_some() {
        return false;
    }
    !matches!(id, "subagent" | "skill" | "default" | "all")
}

/// Returns whether a root agent is user/Voice-routable (`spec.user_routable`).
/// Unknown custom test agent IDs default to `true`.
pub fn is_user_routable_root_agent(agent_id: &str) -> bool {
    find_agent_spec(agent_id).is_none_or(|spec| spec.user_routable)
}

/// Normalizes a user/Voice `target_agent` parameter (`"agent_id"` or `"flow_id/agent_id"`).
/// If the requested target resolves to a non-user-routable agent (such as internal Flow 2 `wiki_root`),
/// redirects it to [`DEFAULT_USER_ROOT_AGENT_ID`] (`"daily_root"`).
pub fn normalize_user_target_agent(raw: &str) -> &str {
    let trimmed = raw.trim();
    let agent_id = trimmed
        .rsplit_once('/')
        .map(|(_, agent)| agent.trim())
        .unwrap_or(trimmed);
    if find_agent_spec(agent_id).is_some_and(|spec| !spec.user_routable) {
        DEFAULT_USER_ROOT_AGENT_ID
    } else {
        trimmed
    }
}
