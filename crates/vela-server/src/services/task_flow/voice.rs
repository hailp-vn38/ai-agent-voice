//! Voice Session adapter for [`TaskToolSuite`] (`orchestrator_flow_rs`).

use std::sync::Arc;
use std::time::Duration;

use orchestrator_flow_rs::{
    AgentCatalogEntry, DEFAULT_OWNER_ID, DEFAULT_WORKSPACE_ID, TargetAgent, TaskManager, TaskScope,
    TaskToolSpec, TaskToolSuite, build_tool_specs_for_agents,
};
use serde_json::{Value, json};

use crate::providers::llm::ToolDefinition;

/// All tool names exposed by [`TaskToolSuite`] to Voice Sessions.
pub const TASK_SUITE_TOOL_NAMES: &[&str] = &[
    "task_create",
    "task_followup",
    "task_list",
    "task_detail",
    "task_cancel",
    "reminder_create",
    "reminder_list",
    "reminder_complete",
    "reminder_delete",
];

pub fn is_task_suite_tool_name(name: &str) -> bool {
    TASK_SUITE_TOOL_NAMES.contains(&name)
}

/// Handle passed to an admitted Voice Session so it can advertise and execute
/// [`TaskToolSuite`] tools (`task_create`, `task_followup`, `task_list`, `task_detail`,
/// `task_cancel`, `reminder_create`, `reminder_list`, `reminder_complete`, `reminder_delete`)
/// under the session's immutable requester [`TaskScope`].
#[derive(Clone)]
pub struct VoiceTaskSubmit {
    suite: Option<TaskToolSuite>,
    scope: TaskScope,
}

impl VoiceTaskSubmit {
    pub fn admitted(
        task_manager: Option<&Arc<TaskManager>>,
        device_db_id: i64,
        agent_id: i64,
    ) -> Option<Self> {
        let owner_id = format!("device:{device_db_id}");
        let workspace_id = format!("agent:{agent_id}");
        task_manager.map(|tm| Self {
            suite: Some(tm.voice_tool_suite()),
            scope: TaskScope::new(owner_id, workspace_id),
        })
    }

    /// Creates an unbacked handle for unit tests that do not boot a full `TaskManager`.
    pub fn unbacked_for_test() -> Self {
        Self {
            suite: None,
            scope: TaskScope::new(DEFAULT_OWNER_ID, DEFAULT_WORKSPACE_ID),
        }
    }

    pub fn scope(&self) -> &TaskScope {
        &self.scope
    }

    pub fn tool_definitions(&self) -> Vec<ToolDefinition> {
        let specs: Vec<TaskToolSpec> = match &self.suite {
            Some(suite) => {
                let user_facing_agents: Vec<AgentCatalogEntry> = suite
                    .available_agents()
                    .into_iter()
                    .filter(|entry| super::is_user_routable_root_agent(&entry.target.agent_id))
                    .collect();
                if user_facing_agents.is_empty() {
                    suite.tool_specs()
                } else {
                    build_tool_specs_for_agents(&user_facing_agents)
                }
            }
            None => fallback_tool_specs(),
        };
        specs
            .into_iter()
            .filter(|spec| is_task_suite_tool_name(&spec.name))
            .map(|spec| ToolDefinition {
                name: spec.name,
                description: spec.description,
                parameters: spec.parameters,
            })
            .collect()
    }

    pub async fn execute(&self, tool_name: &str, mut arguments: Value, budget: Duration) -> Value {
        if !is_task_suite_tool_name(tool_name) {
            return voice_rejection("invalid_arguments");
        }
        let Some(ref suite) = self.suite else {
            return voice_rejection("runtime_unavailable");
        };
        if matches!(tool_name, "task_create" | "reminder_create")
            && let Some(obj) = arguments.as_object_mut()
            && let Some(raw_target) = obj.get("target_agent").and_then(Value::as_str)
        {
            let normalized = super::normalize_user_target_agent(raw_target);
            if normalized != raw_target {
                obj.insert(
                    "target_agent".to_owned(),
                    Value::String(normalized.to_owned()),
                );
            }
        }
        match tokio::time::timeout(
            budget,
            suite.execute_raw_scoped(&self.scope, tool_name, arguments),
        )
        .await
        {
            Ok(Ok(val)) => val,
            Ok(Err(err)) => voice_rejection(err.code().as_str()),
            Err(_) => voice_rejection("submission_outcome_unknown"),
        }
    }
}

pub fn voice_rejection(code: &str) -> Value {
    json!({
        "created": false,
        "isError": true,
        "error": { "code": code }
    })
}

fn fallback_tool_specs() -> Vec<TaskToolSpec> {
    let default_root = super::DEFAULT_USER_ROOT_AGENT_ID;
    let description = super::find_agent_spec(default_root)
        .map(|spec| spec.default_description)
        .unwrap_or("Interactive & background multi-functional assistant.");
    let default_agents = [AgentCatalogEntry {
        target: TargetAgent::new("default", default_root),
        description: description.into(),
    }];
    build_tool_specs_for_agents(&default_agents)
}
