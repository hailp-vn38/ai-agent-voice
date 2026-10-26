use crate::{
    config::EffectiveAgentConfig,
    providers::llm::{ChatMessage, LlmRequest, ToolCall},
    session::SpeakerContext,
};

pub const LLM_REQUEST_HARD_LIMIT_BYTES: usize = 256 * 1024;
const SPEAKER_SLOT: &str = "{{speakers_info}}";
const UNKNOWN_SPEAKER: &str = "No verified speaker for this turn.";
const REMINDER_POLICY_BLOCK: &str = "\
<reminder_policy>
Use server reminder tools for create/list/update/cancel.
Never pretend to run a countdown or schedule messages by yourself.
Use the server's clock and the specified timezone; clarify ambiguous times.
Confirm success only after the tool returns success.
Distinguish saved_on_server from synced_to_device and played.
For a physical device, default to the authenticated device.
For Floating Chat, require an explicitly authorized target physical device.
Do not claim offline wake guarantees for unsynced reminders.
The scheduler (not the model) executes due reminders.
</reminder_policy>";
const REQUEST_BASE_BYTES: usize = 32;
const MESSAGE_BYTES: usize = 32;
const TOOL_DEFINITION_BYTES: usize = 64;
const TOOL_CALL_BYTES: usize = 48;
const TOOL_RESULT_BYTES: usize = 48;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReminderTurnContext {
    pub timezone: String,
    pub current_time: String,
    pub current_device_id: String,
    pub device_kind: String,
    pub target_device_id: Option<String>,
    pub reminder_tools_enabled: bool,
}

#[derive(Clone, Debug, thiserror::Error, PartialEq, Eq)]
pub enum PromptError {
    #[error("invalid prompt template")]
    InvalidTemplate,
    #[error("prompt template is missing {{persona}}")]
    MissingPersona,
    #[error("LLM request has no current user")]
    MissingCurrentUser,
    #[error("LLM request exceeds the Phase A hard limit")]
    RequestTooLarge,
    #[error("system prompt exceeds the hard bound")]
    SystemPromptTooLarge,
}

pub fn render_system(agent: &EffectiveAgentConfig) -> Result<String, PromptError> {
    let template = &agent.prompt_template;
    if template.contains("{%") {
        return Err(PromptError::InvalidTemplate);
    }
    let mut output = String::with_capacity(template.len() + agent.persona.len());
    let mut cursor = 0;
    let mut persona_seen = false;
    while let Some(relative) = template[cursor..].find("{{") {
        let start = cursor + relative;
        if template[cursor..start].contains("}}") {
            return Err(PromptError::InvalidTemplate);
        }
        output.push_str(&template[cursor..start]);
        let token_start = start + 2;
        let Some(end_relative) = template[token_start..].find("}}") else {
            return Err(PromptError::InvalidTemplate);
        };
        let end = token_start + end_relative;
        let token = &template[token_start..end];
        let value = match token {
            "agent_name" => &agent.name,
            "persona" => {
                persona_seen = true;
                &agent.persona
            }
            "language" => &agent.language,
            "speakers_info" => SPEAKER_SLOT,
            _ => return Err(PromptError::InvalidTemplate),
        };
        output.push_str(value);
        cursor = end + 2;
    }
    if template[cursor..].contains("}}") {
        return Err(PromptError::InvalidTemplate);
    }
    output.push_str(&template[cursor..]);
    if !persona_seen {
        return Err(PromptError::MissingPersona);
    }
    if output.len() > crate::config::MAX_RENDERED_SYSTEM_PROMPT_BYTES {
        return Err(PromptError::SystemPromptTooLarge);
    }
    Ok(output)
}

/// Compose the single System message for one conversational turn.
///
/// The base prompt is an immutable admission/switch snapshot. Speaker metadata is verified
/// for this turn, escaped as JSON, and is never saved to the base prompt or dialogue history.
/// Stored Agent Templates without the slot retain compatibility: only matched speakers produce
/// an appended block. No other runtime placeholders are interpreted here.
#[cfg(test)]
pub fn compose_turn_system(
    base: &str,
    speaker: Option<&SpeakerContext>,
) -> Result<String, PromptError> {
    compose_turn_system_with_reminders(base, speaker, None)
}

pub fn compose_turn_system_with_reminders(
    base: &str,
    speaker: Option<&SpeakerContext>,
    reminders: Option<&ReminderTurnContext>,
) -> Result<String, PromptError> {
    let speaker_data = speaker.map(speaker_data);
    let mut composed = if base.contains(SPEAKER_SLOT) {
        base.replace(
            SPEAKER_SLOT,
            speaker_data.as_deref().unwrap_or(UNKNOWN_SPEAKER),
        )
    } else if let Some(data) = speaker_data {
        format!("{base}\n<speakers_info>\n{data}\n</speakers_info>")
    } else {
        base.to_owned()
    };
    if let Some(reminders) = reminders.filter(|r| r.reminder_tools_enabled) {
        if !composed.contains("<reminder_policy>") {
            composed.push('\n');
            composed.push_str(REMINDER_POLICY_BLOCK);
        }
        let data = reminder_data(reminders);
        composed.push_str("\n<reminders_info>\n");
        composed.push_str(&data);
        composed.push_str("\n</reminders_info>");
    }
    if composed.len() > crate::config::MAX_RENDERED_SYSTEM_PROMPT_BYTES {
        return Err(PromptError::SystemPromptTooLarge);
    }
    Ok(composed)
}

fn clean_prompt_field(value: &str, max_chars: usize) -> String {
    value
        .chars()
        .filter(|ch| !ch.is_control())
        .take(max_chars)
        .collect()
}

fn reminder_data(ctx: &ReminderTurnContext) -> String {
    let mut value = serde_json::json!({
        "timezone": clean_prompt_field(&ctx.timezone, 128),
        "current_time": clean_prompt_field(&ctx.current_time, 64),
        "current_device_id": clean_prompt_field(&ctx.current_device_id, 128),
        "device_kind": clean_prompt_field(&ctx.device_kind, 32),
        "reminder_tools_enabled": ctx.reminder_tools_enabled,
    });
    if let Some(target) = &ctx.target_device_id {
        value["target_device_id"] = serde_json::json!(clean_prompt_field(target, 128));
    }
    value
        .to_string()
        .replace('<', "\\u003c")
        .replace('>', "\\u003e")
}

/// Keep speaker data out of the instruction grammar: use a bounded JSON value in its data block.
fn speaker_data(profile: &SpeakerContext) -> String {
    serde_json::json!({
        "recognition_status": "verified_voice_match_not_authorization",
        "display_name": clean_prompt_field(&profile.name, 96),
        "description": profile.description.as_deref().map(|value| clean_prompt_field(value, 1_024)),
    })
    .to_string()
    .replace('<', "\\u003c")
    .replace('>', "\\u003e")
}

pub fn llm_request_size_bytes(request: &LlmRequest) -> Result<usize, PromptError> {
    fn add(total: &mut usize, value: usize) -> Result<(), PromptError> {
        *total = total
            .checked_add(value)
            .ok_or(PromptError::RequestTooLarge)?;
        Ok(())
    }
    fn string(total: &mut usize, value: &str) -> Result<(), PromptError> {
        add(total, value.len())
    }
    let mut total = REQUEST_BASE_BYTES;
    for message in &request.messages {
        add(&mut total, MESSAGE_BYTES)?;
        match message {
            ChatMessage::System { content }
            | ChatMessage::User { content }
            | ChatMessage::AssistantText { content } => string(&mut total, content)?,
            ChatMessage::AssistantToolCalls { calls } => {
                for call in calls {
                    add(&mut total, TOOL_CALL_BYTES)?;
                    string(&mut total, &call.id)?;
                    string(&mut total, &call.name)?;
                    string(
                        &mut total,
                        &serde_json::to_string(&call.arguments)
                            .map_err(|_| PromptError::RequestTooLarge)?,
                    )?;
                }
            }
            ChatMessage::ToolResult {
                tool_call_id,
                content,
            } => {
                add(&mut total, TOOL_RESULT_BYTES)?;
                string(&mut total, tool_call_id)?;
                string(&mut total, content)?;
            }
        }
    }
    for tool in &request.tools {
        add(&mut total, TOOL_DEFINITION_BYTES)?;
        string(&mut total, &tool.name)?;
        string(&mut total, &tool.description)?;
        string(
            &mut total,
            &serde_json::to_string(&tool.parameters).map_err(|_| PromptError::RequestTooLarge)?,
        )?;
    }
    if total > LLM_REQUEST_HARD_LIMIT_BYTES {
        Err(PromptError::RequestTooLarge)
    } else {
        Ok(total)
    }
}

pub fn append_completed_round(
    messages: &mut Vec<ChatMessage>,
    calls: Vec<ToolCall>,
    results: Vec<ChatMessage>,
) {
    if calls.is_empty() {
        return;
    }
    debug_assert_eq!(calls.len(), results.len());
    messages.push(ChatMessage::AssistantToolCalls { calls });
    messages.extend(results);
}

#[cfg(test)]
#[path = "prompt_tests.rs"]
mod tests;
