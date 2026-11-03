use std::sync::{Arc, OnceLock};

use async_trait::async_trait;
use orchestrator_agent_rs::{
    HostTool, OperationUpdate, ToolContext, ToolDescriptor, ToolEffect, ToolError, ToolOperation,
    ToolOperationStatus, ToolOutcome,
};
use orchestrator_flow_rs::{
    CreateTaskRequest, RecipeMetadata, TaskManager, TaskRecipe, TaskRecipeRequest, TaskScope,
    TaskSource,
};
use serde_json::{Value, json};

use super::recipe_validation_error;

const TOOL_NAME: &str = "recipe_start";
const OPERATION_REF: &str = "task_recipe_start";

pub(super) struct TaskRecipeStartTool {
    recipes: Vec<Arc<dyn TaskRecipe>>,
    manager: Arc<OnceLock<Arc<TaskManager>>>,
}

impl TaskRecipeStartTool {
    pub(super) fn new(
        recipes: Vec<Arc<dyn TaskRecipe>>,
        manager: Arc<OnceLock<Arc<TaskManager>>>,
    ) -> Self {
        Self { recipes, manager }
    }

    /// Read on each use: a recipe schema may carry live options such as Wiki domains.
    fn metadata(&self) -> Vec<RecipeMetadata> {
        self.recipes
            .iter()
            .map(|recipe| recipe.metadata())
            .collect()
    }

    fn descriptor_description(&self) -> String {
        let mut description = String::from(
            "Creates a new Task using one of the server-owned recipes. Choose a matching recipe and pass its validated parameters. Available recipes:",
        );
        for recipe in self.metadata() {
            description.push_str(&format!(
                "\n- {} ({} v{}): {} Parameters: {}",
                recipe.id,
                recipe.title,
                recipe.version,
                recipe.description,
                parameter_summary(&recipe)
            ));
        }
        description
    }

    async fn run(&self, ctx: ToolContext, args: Value) -> Result<ToolOutcome, ToolError> {
        let recipe_id = args
            .get("recipe_id")
            .and_then(Value::as_str)
            .filter(|id| !id.trim().is_empty())
            .ok_or_else(|| ToolError {
                code: "invalid_arguments".into(),
                message: "recipe_id is required".into(),
            })?;
        let params = args.get("params").cloned().unwrap_or_else(|| json!({}));
        if !self.metadata().iter().any(|recipe| recipe.id == recipe_id) {
            return Ok(outcome(
                &ctx,
                json!({
                    "created": false,
                    "isError": true,
                    "error": { "code": "invalid_input", "message": format!("recipe '{recipe_id}' is not registered") },
                    "validation_path": "recipe_id"
                }),
            ));
        }
        let request_json = serde_json::to_string(&json!({
            "recipe_id": recipe_id,
            "params": params,
        }))
        .map_err(|error| ToolError {
            code: "invalid_arguments".into(),
            message: error.to_string(),
        })?;

        if let Some(prior) = ctx.operation().current().await? {
            return replay_result(prior, &request_json, &ctx);
        }

        let manager = self.manager.get().ok_or_else(|| ToolError {
            code: "runtime_unavailable".into(),
            message: "task manager is not ready".into(),
        })?;
        let operation = ctx
            .operation()
            .begin(ctx.operation().now_secs() + 24 * 60 * 60, OPERATION_REF)
            .await?;
        if operation.request_json.is_some() || operation.status != ToolOperationStatus::Submitting {
            return replay_result(operation, &request_json, &ctx);
        }
        ctx.operation()
            .advance(OperationUpdate {
                request_json: Some(request_json.clone()),
                ..Default::default()
            })
            .await?;

        // An empty title lets TaskManager name the Task from the recipe's validated parameters.
        let task_request = CreateTaskRequest {
            title: String::new(),
            prompt: String::new(),
            target_agent: None,
            source: TaskSource::Voice,
            schedule_id: None,
            existing_session_id: None,
            related_wiki_links: Vec::new(),
            recipe: Some(TaskRecipeRequest {
                id: recipe_id.to_owned(),
                params,
            }),
            input_artifacts: Vec::new(),
        };
        let scope = TaskScope::from_tool_scope(ctx.scope());
        match manager.create_task_scoped(&scope, task_request).await {
            Ok(task) => {
                let result = json!({ "created": true, "task_id": task.id });
                persist_result(&ctx, ToolOperationStatus::Committed, &request_json, &result)
                    .await?;
                Ok(outcome(&ctx, result))
            }
            Err(error) => {
                let Some((path, _)) = recipe_validation_error(&error) else {
                    return Err(ToolError {
                        code: error.code().as_str().to_owned(),
                        message: error.message().to_owned(),
                    });
                };
                let result = json!({
                    "created": false,
                    "isError": true,
                    "error": { "code": error.code().as_str(), "message": error.message() },
                    "validation_path": path,
                });
                persist_result(&ctx, ToolOperationStatus::Failed, &request_json, &result).await?;
                Ok(outcome(&ctx, result))
            }
        }
    }
}

#[async_trait]
impl HostTool for TaskRecipeStartTool {
    fn descriptor(&self) -> ToolDescriptor {
        ToolDescriptor {
            name: TOOL_NAME.into(),
            description: self.descriptor_description(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "recipe_id": {"type": "string", "minLength": 1},
                    "params": {"type": "object"}
                },
                "required": ["recipe_id", "params"],
                "additionalProperties": false
            }),
            effect: ToolEffect::Mutating,
        }
    }

    async fn execute(&self, ctx: ToolContext, args: Value) -> Result<ToolOutcome, ToolError> {
        self.run(ctx, args).await
    }
}

fn parameter_summary(recipe: &RecipeMetadata) -> String {
    let Some(properties) = recipe
        .params_schema
        .get("properties")
        .and_then(Value::as_object)
    else {
        return "no parameters".into();
    };
    if properties.is_empty() {
        return "no parameters".into();
    }
    let required = recipe.params_schema["required"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .collect::<Vec<_>>();
    properties
        .iter()
        .map(|(name, schema)| {
            let kind = schema["type"].as_str().unwrap_or("value");
            let requiredness = if required.contains(&name.as_str()) {
                "required"
            } else {
                "optional"
            };
            let mut details = vec![format!("{name}: {kind}, {requiredness}")];
            for field in [
                "title",
                "description",
                "format",
                "default",
                "minimum",
                "maximum",
                "enum",
            ] {
                if let Some(value) = schema.get(field).filter(|value| !value.is_null()) {
                    details.push(format!("{field}={value}"));
                }
            }
            details.join("; ")
        })
        .collect::<Vec<_>>()
        .join(", ")
}

async fn persist_result(
    ctx: &ToolContext,
    status: ToolOperationStatus,
    request_json: &str,
    result: &Value,
) -> Result<(), ToolError> {
    ctx.operation()
        .advance(OperationUpdate {
            status: Some(status),
            request_json: Some(request_json.to_owned()),
            result_json: Some(result.to_string()),
            ..Default::default()
        })
        .await
}

fn replay_result(
    prior: ToolOperation,
    request_json: &str,
    ctx: &ToolContext,
) -> Result<ToolOutcome, ToolError> {
    if prior
        .request_json
        .as_deref()
        .is_some_and(|stored| stored != request_json)
    {
        return Ok(outcome(
            ctx,
            json!({
                "created": false,
                "isError": true,
                "error": { "code": "idempotency_key_conflict", "message": "tool call arguments changed during replay" }
            }),
        ));
    }
    if let Some(result) = prior
        .result_json
        .as_deref()
        .and_then(|stored| serde_json::from_str::<Value>(stored).ok())
        && matches!(
            prior.status,
            ToolOperationStatus::Committed | ToolOperationStatus::Failed
        )
    {
        return Ok(outcome(ctx, result));
    }
    Ok(outcome(
        ctx,
        json!({
            "created": false,
            "isError": true,
            "error": { "code": "TOOL_OUTCOME_UNKNOWN", "message": "task creation may still be in progress; do not retry with a new tool call" }
        }),
    ))
}

fn outcome(ctx: &ToolContext, result: Value) -> ToolOutcome {
    ToolOutcome {
        call_id: ctx.scope().tool_call_id.clone(),
        content: result.to_string(),
        metadata: result,
    }
}
