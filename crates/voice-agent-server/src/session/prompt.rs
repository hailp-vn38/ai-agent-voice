use crate::{
    config::EffectiveAgentConfig,
    providers::llm::{ChatMessage, LlmRequest, ToolCall},
};

pub const LLM_REQUEST_HARD_LIMIT_BYTES: usize = 256 * 1024;
const REQUEST_BASE_BYTES: usize = 32;
const MESSAGE_BYTES: usize = 32;
const TOOL_DEFINITION_BYTES: usize = 64;
const TOOL_CALL_BYTES: usize = 48;
const TOOL_RESULT_BYTES: usize = 48;

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
    Ok(output)
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
mod tests {
    use super::*;
    use crate::config::EffectiveAgentConfig;

    #[test]
    fn template_is_strict_and_single_pass() {
        let mut agent = EffectiveAgentConfig::default();
        agent.prompt_template = "{{persona}} {{agent_name}}".into();
        agent.persona = "{{language}}".into();
        assert_eq!(render_system(&agent).unwrap(), "{{language}} Mây");
        agent.prompt_template = "{{ persona }}".into();
        assert_eq!(render_system(&agent), Err(PromptError::InvalidTemplate));
    }

    #[test]
    fn request_bound_uses_utf8_content_and_fixed_overhead() {
        let request = LlmRequest {
            messages: vec![ChatMessage::System {
                content: "é".into(),
            }],
            tools: Vec::new(),
        };
        assert_eq!(llm_request_size_bytes(&request), Ok(32 + 32 + 2));
        let oversized = LlmRequest {
            messages: vec![ChatMessage::User {
                content: "x".repeat(LLM_REQUEST_HARD_LIMIT_BYTES),
            }],
            tools: Vec::new(),
        };
        assert_eq!(
            llm_request_size_bytes(&oversized),
            Err(PromptError::RequestTooLarge)
        );
    }
}
