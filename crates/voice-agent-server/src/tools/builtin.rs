//! Typed server-side LLM tools that do not use Device MCP transport.

use crate::providers::llm::ToolDefinition;

pub const EXIT_TOOL_NAME: &str = "handle_exit_intent";
pub const MAX_GOODBYE_CHARS: usize = 240;

/// Session-local Template switch.
///
/// The name is namespaced with an underscore because OpenAI-compatible function tools accept only
/// ASCII letters, digits, `_`, and `-`. Device MCP tool names are sanitized to the same alphabet.
pub const SWITCH_TEMPLATE_TOOL_NAME: &str = "server_switch_template";
pub const TASK_CREATE_TOOL_NAME: &str = "task_create";
pub const CREATE_REMINDER_TOOL_NAME: &str = "server_create_reminder";
pub const LIST_REMINDERS_TOOL_NAME: &str = "server_list_reminders";
pub const UPDATE_REMINDER_TOOL_NAME: &str = "server_update_reminder";
pub const CANCEL_REMINDER_TOOL_NAME: &str = "server_cancel_reminder";
pub const LEGACY_CREATE_REMINDER_TOOL_NAME: &str = "reminder_create";
pub const LEGACY_LIST_REMINDERS_TOOL_NAME: &str = "reminder_list";
pub const LEGACY_COMPLETE_REMINDER_TOOL_NAME: &str = "reminder_complete";
pub const LEGACY_DELETE_REMINDER_TOOL_NAME: &str = "reminder_delete";
pub const REMINDER_TOOL_NAMES: &[&str] = &[
    CREATE_REMINDER_TOOL_NAME,
    LIST_REMINDERS_TOOL_NAME,
    UPDATE_REMINDER_TOOL_NAME,
    CANCEL_REMINDER_TOOL_NAME,
];
pub const LEGACY_REMINDER_TOOL_NAMES: &[&str] = &[
    LEGACY_CREATE_REMINDER_TOOL_NAME,
    LEGACY_LIST_REMINDERS_TOOL_NAME,
    LEGACY_COMPLETE_REMINDER_TOOL_NAME,
    LEGACY_DELETE_REMINDER_TOOL_NAME,
];
pub use crate::services::task_flow::{TASK_SUITE_TOOL_NAMES, is_task_suite_tool_name};
/// Matches the Resource Key bound the Admin API enforces on a stored Template key, so a
/// model-supplied key can never be an unbounded lookup payload even before membership is checked.
pub const MAX_TEMPLATE_KEY_CHARS: usize = 64;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BuiltinTool {
    EndConversation,
    SwitchTemplate,
    TaskSuite,
    Reminder,
}

pub fn is_reminder_tool_name(name: &str) -> bool {
    REMINDER_TOOL_NAMES.contains(&name)
}

pub fn is_legacy_reminder_tool_name(name: &str) -> bool {
    LEGACY_REMINDER_TOOL_NAMES.contains(&name)
}

pub fn reminder_tool_definitions() -> Vec<ToolDefinition> {
    vec![
        ToolDefinition {
            name: CREATE_REMINDER_TOOL_NAME.into(),
            description: concat!(
                "Create a device reminder on the server. Provide the reminder message and ",
                "either delay_seconds (relative to the server clock) or scheduled_at (RFC3339 with timezone offset)."
            )
            .into(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "message": {
                        "type": "string",
                        "description": "The reminder message to speak when due."
                    },
                    "delay_seconds": {
                        "type": "integer",
                        "minimum": 1,
                        "maximum": 31536000,
                        "description": "Relative delay in seconds from the server's current clock."
                    },
                    "scheduled_at": {
                        "type": "string",
                        "description": "Absolute RFC3339 timestamp with explicit timezone offset."
                    },
                    "timezone": {
                        "type": "string",
                        "description": "Optional IANA timezone or offset name (e.g. Asia/Ho_Chi_Minh)."
                    }
                },
                "required": ["message"],
                "additionalProperties": false
            }),
        },
        ToolDefinition {
            name: LIST_REMINDERS_TOOL_NAME.into(),
            description: "List reminders for the current target device.".into(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "status": {
                        "type": "string",
                        "enum": ["pending", "completed", "cancelled", "expired"],
                        "description": "Optional status filter (defaults to pending)."
                    },
                    "limit": {
                        "type": "integer",
                        "minimum": 1,
                        "maximum": 50,
                        "description": "Maximum number of reminders to return (1-50)."
                    }
                },
                "additionalProperties": false
            }),
        },
        ToolDefinition {
            name: UPDATE_REMINDER_TOOL_NAME.into(),
            description: "Update an existing pending reminder's message or schedule using compare-and-swap.".into(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "reminder_id": {
                        "type": "string",
                        "description": "The ID of the reminder to update."
                    },
                    "expected_revision": {
                        "type": "integer",
                        "minimum": 1,
                        "description": "Current row_revision of the reminder."
                    },
                    "message": {
                        "type": "string",
                        "description": "Updated reminder message."
                    },
                    "delay_seconds": {
                        "type": "integer",
                        "minimum": 1,
                        "maximum": 31536000,
                        "description": "Updated relative delay in seconds."
                    },
                    "scheduled_at": {
                        "type": "string",
                        "description": "Updated absolute RFC3339 timestamp with offset."
                    },
                    "timezone": {
                        "type": "string",
                        "description": "Optional timezone."
                    }
                },
                "required": ["reminder_id", "expected_revision"],
                "additionalProperties": false
            }),
        },
        ToolDefinition {
            name: CANCEL_REMINDER_TOOL_NAME.into(),
            description: "Cancel a pending reminder using compare-and-swap.".into(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "reminder_id": {
                        "type": "string",
                        "description": "The ID of the reminder to cancel."
                    },
                    "expected_revision": {
                        "type": "integer",
                        "minimum": 1,
                        "description": "Current row_revision of the reminder."
                    }
                },
                "required": ["reminder_id", "expected_revision"],
                "additionalProperties": false
            }),
        },
    ]
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum BuiltinToolError {
    #[error("invalid builtin tool arguments")]
    InvalidArguments,
}

#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExitIntentArgs {
    pub say_goodbye: String,
}

#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SwitchTemplateArgs {
    pub template: String,
}

pub fn exit_tool_definition() -> ToolDefinition {
    ToolDefinition {
        name: EXIT_TOOL_NAME.into(),
        description: concat!(
            "Call only when the user clearly asks to end the conversation or leave the session. ",
            "Do not call when the user merely asks how to end a conversation, ",
            "discusses ending conversations hypothetically, or asks why a conversation ended."
        )
        .into(),
        parameters: serde_json::json!({
            "type": "object",
            "properties": {
                "say_goodbye": {
                    "type": "string",
                    "description": "A short, friendly farewell to say before ending the conversation."
                }
            },
            "required": ["say_goodbye"],
            "additionalProperties": false
        }),
    }
}

/// Advertises exactly the Templates this session admitted at its own admission.  A session with
/// no catalog never calls this, so a capability the session cannot honor is never offered.
pub fn switch_template_tool_definition(template_keys: &[&str]) -> ToolDefinition {
    ToolDefinition {
        name: SWITCH_TEMPLATE_TOOL_NAME.into(),
        description: format!(
            "Call when the user asks to use a different assistant persona or scenario. \
             Available templates for this conversation: {}. \
             The switch takes effect from the next turn, so keep answering in the current persona \
             until then.",
            template_keys.join(", ")
        ),
        parameters: serde_json::json!({
            "type": "object",
            "properties": {
                "template": {
                    "type": "string",
                    "description": "The exact template key to switch to.",
                    "enum": template_keys,
                }
            },
            "required": ["template"],
            "additionalProperties": false
        }),
    }
}

pub fn parse_exit_args(arguments: &serde_json::Value) -> Result<ExitIntentArgs, BuiltinToolError> {
    let args: ExitIntentArgs = serde_json::from_value(arguments.clone())
        .map_err(|_| BuiltinToolError::InvalidArguments)?;
    let say_goodbye = args.say_goodbye.trim();
    if say_goodbye.is_empty() || say_goodbye.chars().count() > MAX_GOODBYE_CHARS {
        return Err(BuiltinToolError::InvalidArguments);
    }
    Ok(ExitIntentArgs {
        say_goodbye: say_goodbye.to_owned(),
    })
}

/// Validates shape and bound only.  Whether the key names a candidate this session admitted is a
/// separate, session-local decision the caller makes against its own catalog.
pub fn parse_switch_template_args(
    arguments: &serde_json::Value,
) -> Result<SwitchTemplateArgs, BuiltinToolError> {
    let args: SwitchTemplateArgs = serde_json::from_value(arguments.clone())
        .map_err(|_| BuiltinToolError::InvalidArguments)?;
    let template = args.template.trim();
    if template.is_empty() || template.chars().count() > MAX_TEMPLATE_KEY_CHARS {
        return Err(BuiltinToolError::InvalidArguments);
    }
    Ok(SwitchTemplateArgs {
        template: template.to_owned(),
    })
}
