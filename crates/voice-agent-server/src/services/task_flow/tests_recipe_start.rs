use std::{
    path::PathBuf,
    sync::{Arc, OnceLock},
    time::Duration,
};

use async_trait::async_trait;
use orchestrator_agent_rs::{
    AgentDefinition, AgentMode, HostTool, LlmBackend, LlmConfig, LlmProviderConfig, ModelRole,
    Orchestrator, OrchestratorConfig, ProviderAuth, RequestLimits, SecretString, ToolContext,
    ToolDescriptor, ToolError, ToolOutcome, test_support::ScriptedGateway,
};
use orchestrator_flow_rs::{
    CreateTaskRequest, RecipeDecision, RecipeMetadata, RecipeProgress, RecipeStepOutline,
    RecipeValidationError, TaskConfig, TaskFilter, TaskManager, TaskRecipe, TaskScope, TaskSource,
};
use serde_json::{Value, json};

use super::agent_settings::find_agent_spec;
use super::recipe_start::TaskRecipeStartTool;

const OWNER: &str = "recipe-test-owner";
const WORKSPACE: &str = "recipe-test-workspace";

#[test]
fn recipe_start_is_only_an_available_tool_on_daily_root_and_describes_catalog_params() {
    let daily = find_agent_spec("daily_root").expect("daily root spec");
    assert!(daily.available_tools.contains(&"recipe_start"));
    for agent in super::AGENT_SPECS
        .iter()
        .filter(|agent| agent.id != "daily_root")
    {
        assert!(
            !agent.available_tools.contains(&"recipe_start"),
            "recipe_start must not be available on {}",
            agent.id
        );
    }

    let tool = TaskRecipeStartTool::new(vec![NoopRecipe.metadata()], Arc::new(OnceLock::new()));
    let descriptor = tool.descriptor();
    assert_eq!(descriptor.name, "recipe_start");
    assert_eq!(
        descriptor.effect,
        orchestrator_agent_rs::ToolEffect::Mutating
    );
    assert!(descriptor.description.contains("noop (No-op recipe v1)"));
    assert!(descriptor.description.contains("label: string, required"));
}

struct NoopRecipe;

impl TaskRecipe for NoopRecipe {
    fn metadata(&self) -> RecipeMetadata {
        RecipeMetadata {
            id: "noop".into(),
            version: 1,
            title: "No-op recipe".into(),
            description: "Completes without external work.".into(),
            params_schema: json!({
                "type": "object",
                "properties": { "label": { "type": "string", "title": "Label" } },
                "required": ["label"]
            }),
            outline: Vec::<RecipeStepOutline>::new(),
            max_steps: 1,
        }
    }

    fn validate_params(&self, params: &Value) -> Result<Value, RecipeValidationError> {
        orchestrator_flow_rs::validate_recipe_params(&self.metadata().params_schema, params)
    }

    fn next(
        &self,
        _params: &Value,
        _transcript: &[orchestrator_agent_rs::ModelMessage],
    ) -> RecipeDecision {
        RecipeDecision::Complete("done".into())
    }

    fn progress(
        &self,
        _params: &Value,
        _transcript: &[orchestrator_agent_rs::ModelMessage],
    ) -> RecipeProgress {
        RecipeProgress {
            current_step: None,
            completed_steps: Vec::new(),
            completed_items: None,
            total_items: None,
            summary: Some("done".into()),
            stop_reason: None,
            failed_step: None,
        }
    }
}

struct ReplaySameInvocation {
    inner: Arc<dyn HostTool>,
}

#[async_trait]
impl HostTool for ReplaySameInvocation {
    fn descriptor(&self) -> ToolDescriptor {
        self.inner.descriptor()
    }

    async fn execute(&self, ctx: ToolContext, args: Value) -> Result<ToolOutcome, ToolError> {
        let first = self.inner.execute(ctx.clone(), args.clone()).await?;
        let second = self.inner.execute(ctx, args).await?;
        assert_eq!(
            first.content, second.content,
            "same invocation reuses its result"
        );
        Ok(second)
    }
}

struct Harness {
    root: PathBuf,
    manager: Arc<TaskManager>,
    gateway: Arc<ScriptedGateway>,
}

impl Harness {
    async fn new(replay: bool) -> Self {
        let root = std::env::temp_dir().join(format!("recipe-start-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let daily_prompt = root.join("daily_root.md");
        let recipe_prompt = root.join("recipe.md");
        std::fs::write(&daily_prompt, "daily test").unwrap();
        std::fs::write(&recipe_prompt, "recipe test").unwrap();

        let init = TaskManager::prepare(
            TaskConfig::new(root.join("tasks")).with_default_agent("daily_root"),
        )
        .await
        .unwrap()
        .register_recipe(Arc::new(NoopRecipe))
        .unwrap();
        let runtime = init.recipe_provider_runtime().unwrap();
        let metadata = NoopRecipe.metadata();
        let slot = Arc::new(OnceLock::new());
        let start_tool: Arc<dyn HostTool> = Arc::new(TaskRecipeStartTool::new(
            vec![metadata.clone()],
            slot.clone(),
        ));
        let registered_tool: Arc<dyn HostTool> = if replay {
            Arc::new(ReplaySameInvocation { inner: start_tool })
        } else {
            start_tool
        };
        let agents = vec![
            AgentDefinition::new("daily_root", "test", "test-model")
                .with_mode(AgentMode::Primary)
                .with_prompt_path(daily_prompt)
                .with_allowed_tools(["recipe_start"]),
            AgentDefinition::new(
                format!("recipe_{}", metadata.id),
                "recipe",
                format!("{}@{}", metadata.id, metadata.version),
            )
            .with_mode(AgentMode::Primary)
            .with_prompt_path(recipe_prompt),
        ];
        let gateway = ScriptedGateway::new();
        let orchestrator = Arc::new(
            Orchestrator::builder(OrchestratorConfig::new(
                LlmConfig {
                    providers: vec![LlmProviderConfig {
                        id: "test".into(),
                        backend: LlmBackend::OpenAi,
                        endpoint: None,
                        auth: ProviderAuth::ApiKey(SecretString::new("test")),
                        limits: RequestLimits::uniform(Duration::from_secs(5)),
                    }],
                },
                agents,
                root.join("orchestrator"),
            ))
            .with_gateway(gateway.clone())
            .with_provider_runtime(runtime)
            .register_tool(registered_tool)
            .build()
            .await
            .unwrap(),
        );
        let manager = init.with_orchestrator(orchestrator).start().await.unwrap();
        slot.set(manager.clone())
            .unwrap_or_else(|_| panic!("set manager slot"));
        Self {
            root,
            manager,
            gateway,
        }
    }

    async fn ask_recipe(&self, params: Value) {
        self.gateway.push_tool_calls(
            "",
            vec![(
                "recipe-start-call",
                "recipe_start",
                json!({
                    "recipe_id": "noop",
                    "params": params
                }),
            )],
        );
        self.gateway.push_text("started");
        self.manager
            .create_task_scoped(
                &TaskScope::new(OWNER, WORKSPACE),
                CreateTaskRequest::new("Caller", "start the recipe").with_source(TaskSource::Voice),
            )
            .await
            .unwrap();
    }

    async fn task_count(&self) -> usize {
        self.manager
            .list_tasks_scoped(
                &TaskScope::new(OWNER, WORKSPACE),
                &TaskFilter {
                    limit: Some(10),
                    ..Default::default()
                },
            )
            .await
            .unwrap()
            .items
            .len()
    }

    fn tool_result(&self) -> Option<Value> {
        self.gateway
            .requests()
            .iter()
            .flat_map(|request| &request.messages)
            .find(|message| {
                message.role == ModelRole::Tool
                    && message.tool_name.as_deref() == Some("recipe_start")
            })
            .and_then(|message| serde_json::from_str(&message.text).ok())
    }

    async fn wait_for_tool_result(&self) -> Value {
        for _ in 0..200 {
            if let Some(result) = self.tool_result() {
                return result;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        panic!("timed out waiting for recipe_start result")
    }

    async fn wait_for_tasks(&self, expected: usize) {
        for _ in 0..200 {
            if self.task_count().await == expected {
                return;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        panic!("timed out waiting for {expected} tasks");
    }

    async fn cleanup(self) {
        let _ = self.manager.shutdown().await;
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

#[tokio::test]
async fn recipe_start_creates_recipe_task_from_daily_root() {
    let harness = Harness::new(false).await;
    harness.ask_recipe(json!({ "label": "demo" })).await;
    harness.wait_for_tasks(2).await;
    let result = harness.wait_for_tool_result().await;
    assert_eq!(result["created"], true);
    assert!(result["task_id"].as_str().is_some_and(|id| !id.is_empty()));
    harness.cleanup().await;
}

#[tokio::test]
async fn recipe_start_invalid_params_returns_validation_path_without_creating_task() {
    let harness = Harness::new(false).await;
    harness.ask_recipe(json!({})).await;
    harness.wait_for_tasks(1).await;
    let result = harness.wait_for_tool_result().await;
    assert_eq!(result["created"], false);
    assert_eq!(result["validation_path"], "params.label");
    harness.cleanup().await;
}

#[tokio::test]
async fn recipe_start_replay_of_one_tool_call_does_not_create_a_second_task() {
    let harness = Harness::new(true).await;
    harness.ask_recipe(json!({ "label": "demo" })).await;
    harness.wait_for_tasks(2).await;
    assert_eq!(harness.wait_for_tool_result().await["created"], true);
    harness.cleanup().await;
}
