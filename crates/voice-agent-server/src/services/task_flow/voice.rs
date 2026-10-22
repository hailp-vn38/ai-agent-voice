//! Voice Session adapter for [`TaskToolSuite`] (`orchestrator_flow_rs`).

use std::sync::Arc;
use std::time::Duration;

use orchestrator_flow_rs::{
    AgentCatalogEntry, DEFAULT_OWNER_ID, DEFAULT_WORKSPACE_ID, TargetAgent, TaskManager,
    TaskScope, TaskToolSpec, TaskToolSuite,
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
            Some(suite) => suite.tool_specs(),
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

    pub async fn execute(&self, tool_name: &str, arguments: Value, budget: Duration) -> Value {
        let Some(ref suite) = self.suite else {
            return voice_rejection("runtime_unavailable");
        };
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
    let default_agents = vec![
        AgentCatalogEntry {
            target: TargetAgent::new("default", "daily_root"),
            description: "Interactive & background daily assistant: researches Wiki, creates reports, stages knowledge to sources/, and manages reminders.".into(),
        },
        AgentCatalogEntry {
            target: TargetAgent::new("default", "wiki_root"),
            description: "Background Wiki pipeline coordinator: collects external sources, ingests new files from sources/, updates/patches Wiki pages, synthesizes topics, and runs wiki_lint.".into(),
        },
    ];
    let target_enum: Vec<String> = default_agents
        .iter()
        .flat_map(|a| [a.target.agent_id.clone(), a.target.canonical_key()])
        .collect();
    let target_prop = json!({
        "type": "string",
        "enum": target_enum,
        "description": "Target primary agent ID ('daily_root' or 'wiki_root')."
    });

    vec![
        TaskToolSpec {
            name: "task_create".into(),
            description: "Create and dispatch a background Task immediately (fire-and-forget, does NOT wait for completion). The task will run asynchronously in the background and appear on the Task View UI.".into(),
            parameters: json!({
                "type": "object",
                "required": ["title", "prompt"],
                "properties": {
                    "title": {
                        "type": "string",
                        "description": "Concise human-readable title shown on the Task View UI card."
                    },
                    "prompt": {
                        "type": "string",
                        "description": "Detailed instruction or objective for the background agent to execute."
                    },
                    "target_agent": target_prop,
                    "related_wiki_links": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "Optional list of related Obsidian [[WikiLinks]]."
                    }
                },
                "additionalProperties": false
            }),
        },
        TaskToolSpec {
            name: "task_followup".into(),
            description: "Send a follow-up instruction to an existing background Task. If the Task is currently running, the instruction is queued and automatically submitted as soon as the current turn completes.".into(),
            parameters: json!({
                "type": "object",
                "required": ["task_id", "prompt"],
                "properties": {
                    "task_id": {
                        "type": "string",
                        "description": "ID of the existing Task."
                    },
                    "prompt": {
                        "type": "string",
                        "description": "Additional instruction to send to the Task session."
                    }
                },
                "additionalProperties": false
            }),
        },
        TaskToolSpec {
            name: "task_list".into(),
            description: "List background Tasks and their live status/progress summary on the Task View UI.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "status": {
                        "type": "string",
                        "enum": ["queued", "running", "completed", "failed", "cancelled", "interrupted", "all"],
                        "description": "Optional status filter (defaults to 'all')."
                    },
                    "limit": {
                        "type": "integer",
                        "description": "Maximum number of tasks to return (1..=50, default 10)."
                    }
                },
                "additionalProperties": false
            }),
        },
        TaskToolSpec {
            name: "task_detail".into(),
            description: "Get detailed execution state of a background Task, including Root Agent and Subagent sessions, step/token usage summary, tool invocations, and follow-up queue.".into(),
            parameters: json!({
                "type": "object",
                "required": ["task_id"],
                "properties": {
                    "task_id": {
                        "type": "string",
                        "description": "ID of the Task to inspect."
                    },
                    "include_messages": {
                        "type": "boolean",
                        "description": "Whether to include recent conversation messages of Root and Subagent sessions (default false)."
                    }
                },
                "additionalProperties": false
            }),
        },
        TaskToolSpec {
            name: "task_cancel".into(),
            description: "Cancel a running or queued background Task and clear its pending follow-ups.".into(),
            parameters: json!({
                "type": "object",
                "required": ["task_id"],
                "properties": {
                    "task_id": {
                        "type": "string",
                        "description": "ID of the Task to cancel."
                    }
                },
                "additionalProperties": false
            }),
        },
        TaskToolSpec {
            name: "reminder_create".into(),
            description: "Create a user reminder notification (`notify_only`) or a scheduled background agent task (`spawn_task`) triggered at `due_at`, after `delay_seconds`, or on a `repeat_cron` schedule.".into(),
            parameters: json!({
                "type": "object",
                "required": ["title"],
                "properties": {
                    "title": {
                        "type": "string",
                        "description": "Title of the reminder or scheduled task."
                    },
                    "due_at": {
                        "type": "string",
                        "description": "ISO-8601 timestamp for one-shot trigger (e.g. '2026-10-22T09:00:00+07:00')."
                    },
                    "delay_seconds": {
                        "type": "integer",
                        "description": "Optional relative delay in seconds from now (alternative to due_at)."
                    },
                    "repeat_cron": {
                        "type": "string",
                        "description": "Optional 5-field or 6-field cron expression for recurring schedules (e.g. '0 7 * * *')."
                    },
                    "priority": {
                        "type": "string",
                        "enum": ["low", "medium", "high"],
                        "description": "Priority level (default 'medium')."
                    },
                    "notes": {
                        "type": "string",
                        "description": "Detailed notes for the reminder, or the prompt to execute when action_kind='spawn_task'."
                    },
                    "related_wiki_links": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "Optional list of [[WikiLinks]] attached to this reminder/schedule."
                    },
                    "action_kind": {
                        "type": "string",
                        "enum": ["notify_only", "spawn_task"],
                        "description": "Whether to only notify the user ('notify_only', default) or automatically spawn a background agent Task ('spawn_task')."
                    },
                    "target_agent": target_prop
                },
                "additionalProperties": false
            }),
        },
        TaskToolSpec {
            name: "reminder_list".into(),
            description: "List upcoming or completed reminders and scheduled tasks.".into(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "status": {
                        "type": "string",
                        "enum": ["pending", "active", "paused", "completed", "cancelled", "all"],
                        "description": "Filter by status (default 'pending')."
                    },
                    "from_date": {
                        "type": "string",
                        "description": "Optional ISO-8601 lower bound."
                    },
                    "to_date": {
                        "type": "string",
                        "description": "Optional ISO-8601 upper bound."
                    },
                    "limit": {
                        "type": "integer",
                        "description": "Max items to return (default 20)."
                    }
                },
                "additionalProperties": false
            }),
        },
        TaskToolSpec {
            name: "reminder_complete".into(),
            description: "Mark a reminder or schedule as completed.".into(),
            parameters: json!({
                "type": "object",
                "required": ["reminder_id"],
                "properties": {
                    "reminder_id": {
                        "type": "string",
                        "description": "ID of the reminder/schedule to mark completed."
                    },
                    "resolution_note": {
                        "type": "string",
                        "description": "Optional completion note."
                    }
                },
                "additionalProperties": false
            }),
        },
        TaskToolSpec {
            name: "reminder_delete".into(),
            description: "Delete a reminder or schedule.".into(),
            parameters: json!({
                "type": "object",
                "required": ["reminder_id"],
                "properties": {
                    "reminder_id": {
                        "type": "string",
                        "description": "ID of the reminder/schedule to delete."
                    }
                },
                "additionalProperties": false
            }),
        },
    ]
}
