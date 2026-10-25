//! Unified Background Task, Followup, Schedule/Reminder & Karpathy LLM Wiki Dual-Flow service
//! powered by [`orchestrator_flow_rs`] (`orchestrator_task_rs`).

mod projections;
mod voice;
mod workspace_seed;

#[cfg(test)]
mod tests;

pub use projections::{project_task_events, project_task_to_admin_job};
pub use voice::{
    TASK_SUITE_TOOL_NAMES, VoiceTaskSubmit, is_task_suite_tool_name, voice_rejection,
};
pub use workspace_seed::{
    DAILY_ROOT_PROMPT, DAILY_SYNTHESIS_STAGING_SKILL, DAILY_TASK_REMINDER_SKILL,
    REMINDER_PLANNER_PROMPT, REPORT_STAGER_PROMPT, WIKI_RESEARCHER_PROMPT, WIKI_ROOT_PROMPT,
};

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use llm_wiki::{AdaptiveReadBudget, WikiEngine, WikiError};
use orchestrator_agent_rs::{
    AgentDefinition, AgentMode, LlmBackend, LlmConfig, LlmProviderConfig, Orchestrator,
    OrchestratorConfig, OrchestratorError, ProviderAuth, SecretString,
};
use orchestrator_flow_rs::{
    CreateTaskRequest, TaskConfig, TaskError, TaskManager, TaskRuntimeConfig, TaskSource,
    TaskView, WikiAutoFlowBinding, WorkspaceToolKit,
};
use thiserror::Error;
use tokio::sync::Notify;

use crate::config::{AppConfig, OrchestratorSectionConfig, WikiSectionConfig};
use workspace_seed::ensure_prompts_and_skills;

#[derive(Debug, Error)]
pub enum TaskFlowError {
    #[error("wiki error: {0}")]
    Wiki(#[from] WikiError),
    #[error("orchestrator error: {0}")]
    Orchestrator(#[from] OrchestratorError),
    #[error("task error: {0}")]
    Task(#[from] TaskError),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
}

impl TaskFlowError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::Task(err) => err.code().as_str(),
            Self::Wiki(_) => "knowledge_unavailable",
            Self::Orchestrator(_) => "orchestrator_error",
            Self::Io(_) => "io_error",
        }
    }
}

#[derive(Clone)]
pub struct TaskFlowRuntime {
    pub wiki_engine: Option<Arc<WikiEngine>>,
    pub orchestrator: Option<Arc<Orchestrator>>,
    pub task_manager: Option<Arc<TaskManager>>,
    pub autoflow_notify: Arc<Notify>,
}

pub fn init_wiki_engine(
    cfg: &WikiSectionConfig,
) -> Result<Option<Arc<WikiEngine>>, TaskFlowError> {
    if !cfg.enabled {
        return Ok(None);
    }
    open_wiki_engine(cfg).map(Some)
}

fn open_wiki_engine(cfg: &WikiSectionConfig) -> Result<Arc<WikiEngine>, TaskFlowError> {
    std::fs::create_dir_all(&cfg.vault_dir)?;
    let engine = Arc::new(WikiEngine::open(&cfg.vault_dir)?);
    engine.apply_template(cfg.template)?;
    let _ = engine.sync_index();
    let _ = engine.rebuild_or_sync_index_md();
    Ok(engine)
}

/// Initializes `WikiEngine`, `Orchestrator`, and `TaskManager` (`orchestrator_flow_rs`)
/// from [`AppConfig`].
pub async fn init_from_app_config(config: &AppConfig) -> Result<TaskFlowRuntime, TaskFlowError> {
    let autoflow_notify = Arc::new(Notify::new());
    let mut wiki_engine = init_wiki_engine(&config.wiki)?;

    if !config.orchestrator.enabled {
        return Ok(TaskFlowRuntime {
            wiki_engine,
            orchestrator: None,
            task_manager: None,
            autoflow_notify,
        });
    }

    let engine = match wiki_engine.clone() {
        Some(engine) => engine,
        None => {
            let opened = open_wiki_engine(&config.wiki)?;
            wiki_engine = Some(opened.clone());
            opened
        }
    };

    let (orchestrator, task_manager) = init_orchestrator_and_task_manager(
        &config.orchestrator,
        &config.wiki,
        engine,
        autoflow_notify.clone(),
    )
    .await?;

    Ok(TaskFlowRuntime {
        wiki_engine,
        orchestrator: Some(orchestrator),
        task_manager: Some(task_manager),
        autoflow_notify,
    })
}

async fn init_orchestrator_and_task_manager(
    cfg: &OrchestratorSectionConfig,
    wiki_cfg: &WikiSectionConfig,
    engine: Arc<WikiEngine>,
    autoflow_notify: Arc<Notify>,
) -> Result<(Arc<Orchestrator>, Arc<TaskManager>), TaskFlowError> {
    std::fs::create_dir_all(&cfg.store_dir)?;
    std::fs::create_dir_all(&cfg.tasks_store_dir)?;
    std::fs::create_dir_all(&cfg.reports_dir)?;
    std::fs::create_dir_all(&cfg.work_dir)?;

    let resolved_skills_dir = cfg.skills_dir.as_ref().map(|dir| {
        if dir.exists() {
            dir.clone()
        } else {
            let workspace_candidate = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../..")
                .join(dir);
            if workspace_candidate.exists() {
                workspace_candidate
            } else {
                dir.clone()
            }
        }
    });

    ensure_prompts_and_skills(&cfg.prompts_dir, resolved_skills_dir.as_deref())?;

    let read_budget = AdaptiveReadBudget::for_context_limit(cfg.context_limit_tokens);
    let toolkit = WorkspaceToolKit::new(
        engine.clone(),
        autoflow_notify.clone(),
        &cfg.reports_dir,
        cfg.work_dir.join("pdf"),
        cfg.work_dir.join("yt"),
    )
    .with_html_work_dir(cfg.work_dir.join("html"))
    .with_read_budget(read_budget);

    // Phase 1: Prepare `TaskManager` (`tasks.db` + `tasks.owner.lock` + `HostTool` builders)
    let runtime_cfg = TaskRuntimeConfig {
        scheduler_tick: Duration::from_secs(5),
        autoflow_poll_interval: Duration::from_secs(30),
        event_capacity: 2048,
        ..Default::default()
    };
    let task_cfg = TaskConfig::new(&cfg.tasks_store_dir)
        .with_wiki_vault_dir(&wiki_cfg.vault_dir)
        .with_default_timezone(&cfg.default_timezone)
        .with_default_agent("daily_root")
        .with_runtime(runtime_cfg);
    let task_init = TaskManager::prepare(task_cfg).await?;

    // Build unified `Orchestrator` with Flow 2 (`daily_root`) & Flow 1 (`wiki_root`)
    let provider_id = "default_openai";
    let llm_cfg = LlmConfig {
        providers: vec![LlmProviderConfig {
            id: provider_id.into(),
            backend: LlmBackend::OpenAiCompatible,
            endpoint: Some(cfg.endpoint.clone()),
            auth: if cfg.api_key.trim().is_empty() {
                ProviderAuth::None
            } else {
                ProviderAuth::ApiKey(SecretString::new(cfg.api_key.clone()))
            },
            request_timeout: Duration::from_millis(cfg.request_timeout_ms),
        }],
    };

    let prompts = &cfg.prompts_dir;
    let model = &cfg.model;
    let ctx_limit = cfg.context_limit_tokens;
    let max_out = cfg.max_output_tokens;

    let agents = vec![
        // === Flow 2: Interactive & Daily Background Assistant (`daily_root`) ===
        AgentDefinition::new("daily_root", provider_id, model)
            .with_mode(AgentMode::Primary)
            .with_description(
                "Interactive & background daily assistant: researches Wiki/Web, creates reports, stages knowledge to sources/, and manages reminders.",
            )
            .with_prompt_path(prompts.join("daily_root.md"))
            .with_context_limit_tokens(ctx_limit)
            .with_max_output_tokens(max_out)
            .with_allowed_tools(["subagent"])
            .with_allowed_subagents(["wiki_researcher", "report_stager", "reminder_planner"])
            .with_steps(15),
        AgentDefinition::new("wiki_researcher", provider_id, model)
            .with_mode(AgentMode::Subagent)
            .with_description(
                "Read-only specialist for querying and reading the LLM Wiki and inspecting web pages.",
            )
            .with_prompt_path(prompts.join("wiki_researcher.md"))
            .with_context_limit_tokens(ctx_limit)
            .with_max_output_tokens(max_out)
            .with_allowed_tools([
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
            ])
            .with_allowed_skills(["wiki-query"])
            .with_steps(12),
        AgentDefinition::new("report_stager", provider_id, model)
            .with_mode(AgentMode::Subagent)
            .with_description(
                "Synthesizes discussions into sources/<domain>/<topic>/ via add_to_sources or writes standalone reports via markdown_create.",
            )
            .with_prompt_path(prompts.join("report_stager.md"))
            .with_context_limit_tokens(ctx_limit)
            .with_max_output_tokens(max_out)
            .with_allowed_tools([
                "skill",
                "wiki_tree",
                "wiki_get",
                "add_to_sources",
                "markdown_create",
                "markdown_read",
            ])
            .with_allowed_skills(["wiki-taxonomy", "daily-synthesis-staging"])
            .with_steps(12),
        AgentDefinition::new("reminder_planner", provider_id, model)
            .with_mode(AgentMode::Subagent)
            .with_description(
                "Creates, lists, completes, and deletes user reminders and scheduled tasks.",
            )
            .with_prompt_path(prompts.join("reminder_planner.md"))
            .with_context_limit_tokens(ctx_limit)
            .with_max_output_tokens(max_out)
            .with_allowed_tools([
                "skill",
                "reminder_create",
                "reminder_list",
                "reminder_complete",
                "reminder_delete",
            ])
            .with_allowed_skills(["daily-task-reminder"])
            .with_steps(10),
        // === Flow 1: Background Karpathy LLM Wiki Pipeline (`wiki_root`) ===
        AgentDefinition::new("wiki_root", provider_id, model)
            .with_mode(AgentMode::Primary)
            .with_description(
                "Background Wiki pipeline coordinator: collects external sources (YouTube playlists, PDFs, Web pages), ingests new files from sources/, updates/patches Wiki pages, synthesizes topics, and runs wiki_lint.",
            )
            .with_prompt_path(prompts.join("wiki_root.md"))
            .with_context_limit_tokens(ctx_limit)
            .with_max_output_tokens(max_out)
            .with_allowed_tools(["subagent"])
            .with_allowed_subagents(["source_collector", "wiki_ingestor", "wiki_curator"])
            .with_steps(40),
        AgentDefinition::new("source_collector", provider_id, model)
            .with_mode(AgentMode::Subagent)
            .with_description(
                "Inspects external sources (YouTube playlist, PDF via anydoc, or Web via fetch_html/html_*), extracts detailed transcripts/markdown/metadata, classifies topic hierarchy (<domain>/<topic>), and saves raw Markdown files into sources/<domain>/<topic>/<slug>.md.",
            )
            .with_prompt_path(prompts.join("source_collector.md"))
            .with_context_limit_tokens(ctx_limit)
            .with_max_output_tokens(max_out)
            .with_allowed_tools([
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
            ])
            .with_allowed_skills(["wiki-taxonomy"])
            .with_steps(35),
        AgentDefinition::new("wiki_ingestor", provider_id, model)
            .with_mode(AgentMode::Subagent)
            .with_description(
                "Compiles raw sources into wiki/sources/ and creates/patches entity & concept pages.",
            )
            .with_prompt_path(prompts.join("wiki_ingestor.md"))
            .with_context_limit_tokens(ctx_limit)
            .with_max_output_tokens(max_out)
            .with_allowed_tools([
                "skill",
                "wiki_tree",
                "wiki_get",
                "wiki_multi_get",
                "wiki_query",
                "wiki_metadata",
                "wiki_ingest_source",
                "wiki_write_file",
                "wiki_patch_file",
            ])
            .with_allowed_skills(["wiki-taxonomy", "wiki-ingest"])
            .with_steps(35),
        AgentDefinition::new("wiki_curator", provider_id, model)
            .with_mode(AgentMode::Subagent)
            .with_description("Creates topic syntheses and repairs structural issues via wiki_lint.")
            .with_prompt_path(prompts.join("wiki_curator.md"))
            .with_context_limit_tokens(ctx_limit)
            .with_max_output_tokens(max_out)
            .with_allowed_tools([
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
            ])
            .with_allowed_skills(["wiki-taxonomy", "wiki-query", "wiki-lint"])
            .with_steps(35),
    ];

    let mut orch_cfg = OrchestratorConfig::new(llm_cfg, agents, &cfg.store_dir)
        .with_project_instructions(engine.config().schema_path());
    if let Some(ref skills_dir) = resolved_skills_dir {
        orch_cfg = orch_cfg.with_skills_dir(skills_dir);
    }
    orch_cfg.runtime.snapshot_refresh = Duration::from_secs(1);
    orch_cfg.runtime.context_limit_tokens = ctx_limit;
    orch_cfg.runtime.compaction_threshold_percent = 85;
    orch_cfg.runtime.compaction_keep_recent = 6;
    orch_cfg.runtime.tool_output_cap_bytes =
        (read_budget.max_window_chars * 3 / 2).max(24 * 1024);
    orch_cfg.runtime.max_concurrent_root_runs = cfg.max_concurrent_root_runs;

    let mut builder = Orchestrator::builder(orch_cfg);
    for tool in toolkit.all_host_tools(&task_init) {
        builder = builder.register_tool(tool);
    }
    let orchestrator = Arc::new(builder.build().await?);

    // Phase 2: Bind `Orchestrator` + `WikiAutoFlowBinding` and start `TaskManager`
    let task_manager = task_init
        .with_orchestrator(orchestrator.clone())
        .with_wiki_autoflow(WikiAutoFlowBinding::for_agent(
            engine,
            autoflow_notify,
            "wiki_root",
        ))
        .start()
        .await?;

    Ok((orchestrator, task_manager))
}

/// Triggers an explicit Wiki ingestion/curation task for a source via [`TaskManager`].
/// If a `wiki_root` autoflow task is already queued or running, returns that active task
/// instead of spawning a competing duplicate run.
pub async fn submit_wiki_source_analysis(
    task_manager: &TaskManager,
    source_id: &str,
) -> Result<TaskView, TaskFlowError> {
    let active_filter = orchestrator_flow_rs::TaskFilter {
        agent_id: Some("wiki_root".to_string()),
        limit: Some(10),
        ..Default::default()
    };
    if let Ok(page) = task_manager.list_all_tasks(&active_filter).await
        && let Some(active) = page
            .items
            .into_iter()
            .find(|t| !t.status.is_terminal())
    {
        return Ok(active);
    }

    let title = format!("Phân tích & Ingest nguồn Wiki: {source_id}");
    let prompt = task_manager
        .wiki_engine()
        .and_then(|engine| engine.build_next_autoflow_prompt(None).ok().flatten())
        .map(|canonical| {
            format!(
                "{canonical}\n\n[PRIORITY SOURCE FOCUS]: Ưu tiên kiểm tra và ingest tài liệu `{source_id}`."
            )
        })
        .unwrap_or_else(|| {
            format!(
                "[AUTO-FLOW: NEW SOURCES DETECTED]\n\
                 Hãy điều phối subagent `wiki_ingestor` để ingest tài liệu `{source_id}` từ `sources/` sang `wiki/`, \
                 sau đó gọi `wiki_curator` để tổng hợp Synthesis và chạy `wiki_lint`."
            )
        });
    let req = CreateTaskRequest::new(title, prompt)
        .with_target("wiki_root")
        .with_source(TaskSource::Autoflow);
    Ok(task_manager.create_task(req).await?)
}
