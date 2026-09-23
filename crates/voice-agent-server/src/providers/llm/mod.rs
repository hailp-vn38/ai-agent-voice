//! Vendor-neutral LLM streaming boundary.

use std::pin::Pin;

use futures_util::Stream;
use serde_json::Value;
use thiserror::Error;

pub type LlmEventStream = Pin<Box<dyn Stream<Item = Result<LlmEvent, LlmError>> + Send>>;

#[derive(Clone, Debug, PartialEq)]
pub enum LlmEvent {
    TextDelta(String),
    ToolCall(ToolCall),
    Finished,
}

#[derive(Clone, Debug, PartialEq)]
pub enum ChatMessage {
    User {
        content: String,
    },
    AssistantText {
        content: String,
    },
    AssistantToolCalls {
        calls: Vec<ToolCall>,
    },
    ToolResult {
        tool_call_id: String,
        content: String,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub struct ToolDefinition {
    pub name: String,
    pub description: String,
    pub parameters: Value,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub arguments: Value,
}

#[derive(Clone, Debug, PartialEq)]
pub struct LlmRequest {
    pub messages: Vec<ChatMessage>,
    pub tools: Vec<ToolDefinition>,
}

impl LlmRequest {
    pub fn text_turn(text: String) -> Self {
        Self {
            messages: vec![ChatMessage::User { content: text }],
            tools: Vec::new(),
        }
    }
}

impl From<String> for LlmRequest {
    fn from(value: String) -> Self {
        Self::text_turn(value)
    }
}

impl From<&str> for LlmRequest {
    fn from(value: &str) -> Self {
        Self::text_turn(value.to_owned())
    }
}

#[async_trait::async_trait]
pub trait LlmProvider: Send + Sync {
    fn adapter(&self) -> &'static str;
    fn complete(&self, _: &LlmRequest) -> Result<String, LlmError> {
        Err(LlmError::Failed)
    }

    async fn stream(&self, request: LlmRequest) -> Result<LlmEventStream, LlmError> {
        let response = self.complete(&request)?;
        Ok(Box::pin(futures_util::stream::iter([
            Ok(LlmEvent::TextDelta(response)),
            Ok(LlmEvent::Finished),
        ])))
    }
}

#[derive(Clone, Debug, Error)]
pub enum LlmError {
    #[error("LLM completion failed")]
    Failed,
}

pub struct UnavailableLlm;

impl LlmProvider for UnavailableLlm {
    fn adapter(&self) -> &'static str {
        "unavailable"
    }
}

pub(crate) struct ConfiguredOpenAiLlm {
    client: std::sync::Arc<dyn llm::LLMProvider>,
}

impl ConfiguredOpenAiLlm {
    pub(crate) fn build(
        api_key: &str,
        base_url: &str,
        model: &str,
        timeout_seconds: u64,
    ) -> Result<Self, LlmError> {
        let client = llm::builder::LLMBuilder::new()
            .backend(llm::builder::LLMBackend::OpenAI)
            .api_key(api_key)
            .base_url(base_url)
            .model(model)
            .timeout_seconds(timeout_seconds)
            .build()
            .map_err(|_| LlmError::Failed)?;
        Ok(Self {
            client: std::sync::Arc::from(client),
        })
    }
}

#[async_trait::async_trait]
impl LlmProvider for ConfiguredOpenAiLlm {
    fn adapter(&self) -> &'static str {
        "openai"
    }

    async fn stream(&self, request: LlmRequest) -> Result<LlmEventStream, LlmError> {
        use futures_util::StreamExt;
        let messages = request
            .messages
            .iter()
            .map(to_llm_message)
            .collect::<Vec<_>>();
        let tools = request
            .tools
            .iter()
            .map(|tool| llm::chat::Tool {
                tool_type: "function".into(),
                function: llm::chat::FunctionTool {
                    name: tool.name.clone(),
                    description: tool.description.clone(),
                    parameters: tool.parameters.clone(),
                },
                cache_control: None,
            })
            .collect::<Vec<_>>();
        let stream = self
            .client
            .chat_stream_with_tools(&messages, (!tools.is_empty()).then_some(tools.as_slice()))
            .await
            .map_err(|_| LlmError::Failed)?;
        Ok(Box::pin(stream.map(|chunk| {
            match chunk {
                Ok(llm::chat::StreamChunk::Text(text)) => Ok(LlmEvent::TextDelta(text)),
                Ok(llm::chat::StreamChunk::ToolUseComplete { tool_call, .. }) => {
                    serde_json::from_str(&tool_call.function.arguments)
                        .map(|arguments| {
                            LlmEvent::ToolCall(ToolCall {
                                id: tool_call.id,
                                name: tool_call.function.name,
                                arguments,
                            })
                        })
                        .map_err(|_| LlmError::Failed)
                }
                Ok(llm::chat::StreamChunk::Done { .. }) => Ok(LlmEvent::Finished),
                Ok(_) => Err(LlmError::Failed),
                Err(_) => Err(LlmError::Failed),
            }
        })))
    }
}

fn to_llm_message(message: &ChatMessage) -> llm::chat::ChatMessage {
    match message {
        ChatMessage::User { content } => llm::chat::ChatMessage::user().content(content).build(),
        ChatMessage::AssistantText { content } => {
            llm::chat::ChatMessage::assistant().content(content).build()
        }
        ChatMessage::AssistantToolCalls { calls } => llm::chat::ChatMessage::assistant()
            .tool_use(calls.iter().map(to_llm_call).collect())
            .build(),
        ChatMessage::ToolResult {
            tool_call_id,
            content,
        } => llm::chat::ChatMessage::user()
            .tool_result(vec![llm::ToolCall {
                id: tool_call_id.clone(),
                call_type: "function".into(),
                function: llm::FunctionCall {
                    name: "tool_result".into(),
                    arguments: content.clone(),
                },
            }])
            .build(),
    }
}

fn to_llm_call(call: &ToolCall) -> llm::ToolCall {
    llm::ToolCall {
        id: call.id.clone(),
        call_type: "function".into(),
        function: llm::FunctionCall {
            name: call.name.clone(),
            arguments: call.arguments.to_string(),
        },
    }
}
