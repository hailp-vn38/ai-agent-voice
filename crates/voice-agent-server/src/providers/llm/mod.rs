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
        Ok(Box::pin(stream.filter_map(|chunk| async move {
            adapt_stream_chunk(chunk)
        })))
    }
}

/// Maps provider transport chunks to the small domain vocabulary used by the actor.
/// Tool start and argument deltas are intentionally not terminal events: the `llm`
/// crate assembles them and emits the completed call later in the same stream.
fn adapt_stream_chunk(
    chunk: Result<llm::chat::StreamChunk, llm::error::LLMError>,
) -> Option<Result<LlmEvent, LlmError>> {
    match chunk {
        Ok(llm::chat::StreamChunk::Text(text)) => Some(Ok(LlmEvent::TextDelta(text))),
        Ok(llm::chat::StreamChunk::ToolUseStart { index, .. }) => {
            tracing::info!(
                event = "llm_tool_use_started",
                tool_index = index,
                "LLM tool use started"
            );
            None
        }
        Ok(llm::chat::StreamChunk::ToolUseInputDelta { .. }) => None,
        Ok(llm::chat::StreamChunk::ToolUseComplete { tool_call, .. }) => {
            let name = tool_call.function.name.clone();
            let result = serde_json::from_str(&tool_call.function.arguments)
                .map(|arguments| {
                    LlmEvent::ToolCall(ToolCall {
                        id: tool_call.id,
                        name,
                        arguments,
                    })
                })
                .map_err(|_| LlmError::Failed);
            if result.is_ok() {
                tracing::info!(event = "llm_tool_call_complete", "LLM tool call completed");
            }
            Some(result)
        }
        Ok(llm::chat::StreamChunk::Done { .. }) => Some(Ok(LlmEvent::Finished)),
        Err(_) => Some(Err(LlmError::Failed)),
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

#[cfg(test)]
mod tests {
    use futures_util::StreamExt;

    use super::*;

    #[tokio::test]
    async fn tool_stream_intermediate_chunks_do_not_fail_the_round() {
        let chunks = vec![
            Ok(llm::chat::StreamChunk::ToolUseStart {
                index: 0,
                id: "call-1".into(),
                name: "test_set_value".into(),
            }),
            Ok(llm::chat::StreamChunk::ToolUseInputDelta {
                index: 0,
                partial_json: r#"{"value":50}"#.into(),
            }),
            Ok(llm::chat::StreamChunk::ToolUseComplete {
                index: 0,
                tool_call: llm::ToolCall {
                    id: "call-1".into(),
                    call_type: "function".into(),
                    function: llm::FunctionCall {
                        name: "test_set_value".into(),
                        arguments: r#"{"value":50}"#.into(),
                    },
                },
            }),
            Ok(llm::chat::StreamChunk::Done {
                stop_reason: "tool_use".into(),
            }),
        ];

        let events = futures_util::stream::iter(chunks)
            .filter_map(|chunk| async move { adapt_stream_chunk(chunk) })
            .collect::<Vec<_>>()
            .await;

        assert_eq!(events.len(), 2);
        assert!(matches!(
            &events[0],
            Ok(LlmEvent::ToolCall(ToolCall { id, name, arguments }))
                if id == "call-1"
                    && name == "test_set_value"
                    && *arguments == serde_json::json!({"value": 50})
        ));
        assert!(matches!(&events[1], Ok(LlmEvent::Finished)));
    }
}
