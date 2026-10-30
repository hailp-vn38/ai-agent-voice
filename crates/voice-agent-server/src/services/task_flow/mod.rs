//! Unified Background Task, Followup, Schedule/Reminder & Karpathy LLM Wiki Dual-Flow service
//! powered by [`orchestrator_flow_rs`] (`orchestrator_task_rs`).

pub mod agent_settings;
pub mod charts;
mod projections;
pub mod skills;
mod voice;
mod workspace_seed;

#[cfg(test)]
mod tests;
#[cfg(test)]
mod tests_media;
#[cfg(test)]
mod tests_providers;

pub(crate) use agent_settings::install_server_media_runtime;
pub use agent_settings::{
    AGENT_SPECS, AgentConfigOverride, AgentSpec, CUSTOM_DAILY_SUBAGENT_TOOLS,
    CUSTOM_WIKI_SUBAGENT_TOOLS, CatalogAsrTranscriberBackend, CatalogImageGeneratorBackend,
    CatalogMediaBackend, CatalogTtsSynthesizerBackend, CatalogVideoGeneratorBackend,
    CreateCustomSubagentInput, DEFAULT_ORCHESTRATOR_PROVIDER_REF, DEFAULT_USER_ROOT_AGENT_ID,
    PersistedBackgroundSettings, ResolvedLlmProvider, SubagentMutationError,
    WIKI_AUTOFLOW_ROOT_AGENT_ID, build_agent_definitions, create_custom_subagent,
    custom_subagent_flow_policy, default_orchestrator_provider, delete_custom_subagent,
    find_agent_spec, find_tool_owner_spec, is_user_routable_root_agent, load_persisted_settings,
    normalize_user_target_agent, project_agents_json, resolve_llm_provider_catalog,
    save_persisted_settings, sync_orchestrator_from_state, valid_custom_subagent_id,
    validate_agent_overrides, validate_agent_overrides_with_skills, write_agent_prompts,
};
pub use charts::{ChartDocument, ChartError, chart_host_tools, read_chart_json_for_admin};
pub use projections::{
    project_artifact_catalog_page, project_task_events, project_task_to_admin_job,
};
pub use voice::{TASK_SUITE_TOOL_NAMES, VoiceTaskSubmit, is_task_suite_tool_name, voice_rejection};
pub use workspace_seed::{
    BUILTIN_SKILLS, DAILY_ROOT_PROMPT, DAILY_SYNTHESIS_STAGING_SKILL, DAILY_TASK_REMINDER_SKILL,
    MEDIA_CREATOR_PROMPT, MEDIA_PROCESSOR_PROMPT, REMINDER_PLANNER_PROMPT, REPORT_STAGER_PROMPT,
    SOURCE_COLLECTOR_PROMPT, TRADING_CHART_SKILL, WIKI_CURATOR_PROMPT, WIKI_INGEST_SKILL,
    WIKI_INGESTOR_PROMPT, WIKI_LINT_SKILL, WIKI_QUERY_SKILL, WIKI_RESEARCHER_PROMPT,
    WIKI_ROOT_PROMPT, WIKI_TAXONOMY_SKILL,
};

use std::sync::Arc;
use std::time::Duration;

use llm_wiki::{AdaptiveReadBudget, WikiEngine, WikiError};
use orchestrator_agent_rs::{
    HostTool, LlmConfig, Orchestrator, OrchestratorConfig, OrchestratorError, ToolContext,
    ToolDescriptor, ToolError, ToolOutcome,
};
use orchestrator_flow_rs::{
    CreateTaskRequest, TaskConfig, TaskError, TaskManager, TaskRuntimeConfig, TaskSource, TaskView,
    WikiAutoFlowBinding, WorkspaceToolKit,
};
use serde_json::Value;
use thiserror::Error;
use tokio::sync::Notify;

use crate::config::{AppConfig, OrchestratorSectionConfig, WikiSectionConfig};
use crate::tools::knowledge_mcp::normalize_wiki_tool_arguments;
use workspace_seed::ensure_prompts_and_skills;

struct NormalizedWikiHostTool {
    inner: Arc<dyn HostTool>,
}

#[async_trait::async_trait]
impl HostTool for NormalizedWikiHostTool {
    fn descriptor(&self) -> ToolDescriptor {
        self.inner.descriptor()
    }

    async fn execute(&self, ctx: ToolContext, mut args: Value) -> Result<ToolOutcome, ToolError> {
        let name = self.inner.descriptor().name;
        normalize_wiki_tool_arguments(&name, &mut args);
        self.inner.execute(ctx, args).await
    }
}

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

pub fn init_wiki_engine(cfg: &WikiSectionConfig) -> Result<Option<Arc<WikiEngine>>, TaskFlowError> {
    if !cfg.enabled && !cfg.mcp.enabled {
        return Ok(None);
    }
    open_wiki_engine(cfg).map(Some)
}

pub(crate) fn open_wiki_engine(cfg: &WikiSectionConfig) -> Result<Arc<WikiEngine>, TaskFlowError> {
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
    let initial_catalog = vec![default_orchestrator_provider(&config.orchestrator)];
    init_from_app_config_with_catalog(config, &initial_catalog).await
}

/// Initializes `WikiEngine`, `Orchestrator`, and `TaskManager` with an already-resolved
/// LLM provider catalog so startup recovery has access to all configured and DB providers.
pub async fn init_from_app_config_with_catalog(
    config: &AppConfig,
    catalog: &[ResolvedLlmProvider],
) -> Result<TaskFlowRuntime, TaskFlowError> {
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
        catalog,
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
    catalog: &[ResolvedLlmProvider],
) -> Result<(Arc<Orchestrator>, Arc<TaskManager>), TaskFlowError> {
    std::fs::create_dir_all(&cfg.store_dir)?;
    std::fs::create_dir_all(&cfg.tasks_store_dir)?;
    std::fs::create_dir_all(&cfg.reports_dir)?;
    std::fs::create_dir_all(&cfg.work_dir)?;

    ensure_prompts_and_skills(&cfg.prompts_dir, cfg.skills_dir.as_deref())?;

    let read_budget = AdaptiveReadBudget::for_context_limit(cfg.context_limit_tokens);
    let media_backend = Arc::new(CatalogMediaBackend::new(&cfg.store_dir, catalog.to_vec()));
    let toolkit = WorkspaceToolKit::new(
        engine.clone(),
        autoflow_notify.clone(),
        &cfg.reports_dir,
        cfg.work_dir.join("pdf"),
        cfg.work_dir.join("yt"),
    )
    .with_html_work_dir(cfg.work_dir.join("html"))
    .with_read_budget(read_budget)
    .with_image_backend(media_backend.clone())
    .with_video_backend(media_backend.clone())
    .with_tts_backend(media_backend.clone())
    .with_asr_backend(media_backend);

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

    let persisted = load_persisted_settings(&cfg.store_dir).unwrap_or_default();
    let _ = write_agent_prompts(&cfg.prompts_dir, &persisted);
    let (providers, agents) = build_agent_definitions(cfg, &persisted, catalog);
    let llm_cfg = LlmConfig { providers };
    let ctx_limit = cfg.context_limit_tokens;

    let mut orch_cfg = OrchestratorConfig::new(llm_cfg, agents, &cfg.store_dir)
        .with_project_instructions(engine.config().schema_path());
    if let Some(ref skills_dir) = cfg.skills_dir {
        orch_cfg = orch_cfg.with_skills_dir(skills_dir);
    }
    orch_cfg.runtime.snapshot_refresh = Duration::from_secs(1);
    orch_cfg.runtime.context_limit_tokens = ctx_limit;
    orch_cfg.runtime.compaction_threshold_percent = 85;
    orch_cfg.runtime.compaction_keep_recent = 6;
    orch_cfg.runtime.tool_output_cap_bytes = (read_budget.max_window_chars * 3 / 2).max(24 * 1024);
    orch_cfg.runtime.max_concurrent_root_runs = cfg.max_concurrent_root_runs;

    let mut builder = Orchestrator::builder(orch_cfg);
    for tool in toolkit.all_host_tools(&task_init) {
        builder = builder.register_tool(Arc::new(NormalizedWikiHostTool { inner: tool }));
    }
    for tool in chart_host_tools(&wiki_cfg.vault_dir) {
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
        && let Some(active) = page.items.into_iter().find(|t| !t.status.is_terminal())
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
