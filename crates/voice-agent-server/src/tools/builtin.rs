//! Typed server-side LLM tools that do not use Device MCP transport.

use crate::providers::llm::ToolDefinition;

pub const EXIT_TOOL_NAME: &str = "handle_exit_intent";
pub const MAX_GOODBYE_CHARS: usize = 240;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BuiltinTool {
    EndConversation,
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
