//! Typed server-side LLM tools that do not use Device MCP transport.

use crate::providers::llm::ToolDefinition;

pub const EXIT_TOOL_NAME: &str = "handle_exit_intent";
pub const MAX_GOODBYE_CHARS: usize = 240;

/// Session-local Template switch.
///
/// The dotted name namespaces it as a server action and makes a collision structurally impossible:
/// a Device MCP tool name is sanitized to alphanumerics, `_` and `-` before it can reach the model.
pub const SWITCH_TEMPLATE_TOOL_NAME: &str = "server.switch_template";
/// Matches the Resource Key bound the Admin API enforces on a stored Template key, so a
/// model-supplied key can never be an unbounded lookup payload even before membership is checked.
pub const MAX_TEMPLATE_KEY_CHARS: usize = 64;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BuiltinTool {
    EndConversation,
    SwitchTemplate,
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
