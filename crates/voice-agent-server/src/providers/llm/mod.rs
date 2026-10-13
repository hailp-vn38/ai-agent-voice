//! Vendor-neutral LLM streaming boundary.

use std::pin::Pin;

use futures_util::Stream;
use serde_json::Value;
use thiserror::Error;

pub(crate) mod openai;
pub(crate) mod opencode_free;
#[cfg(feature = "qualification-providers")]
pub mod qualification;

pub type LlmEventStream = Pin<Box<dyn Stream<Item = Result<LlmEvent, LlmError>> + Send>>;

#[derive(Clone, Debug, PartialEq)]
pub enum LlmEvent {
    TextDelta(String),
    ToolCall(ToolCall),
    Finished,
}

#[derive(Clone, Debug, PartialEq)]
pub enum ChatMessage {
    System {
        content: String,
    },
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
    fn supports_tools(&self) -> bool {
        false
    }
    fn adapter(&self) -> &'static str;
    fn complete(&self, _: &LlmRequest) -> Result<String, LlmError> {
        Err(LlmError::Failed)
    }

    /// Background-only completion. Voice always uses `stream()` (Chat Completions).
    /// Structured schema is advisory for chat-only providers; Rust validates the result.
    async fn background_complete(&self, request: LlmRequest, _schema: Option<Value>) -> Result<String, LlmError> {
        complete_background_chat(self.stream(request).await?).await
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

/// Chat-only fallback for Background; no MCP, provider tools or streaming to the Device.
async fn complete_background_chat(mut stream: LlmEventStream) -> Result<String,LlmError> {
    use futures_util::StreamExt;
    let mut output=String::new();
    while let Some(event)=stream.next().await {
        match event? {
            LlmEvent::TextDelta(delta) => {
                if output.len().saturating_add(delta.len())>65536 {
                    return Err(LlmError::Failed);
                }
                output.push_str(&delta);
            }
            LlmEvent::Finished => {
                return if output.is_empty() {Err(LlmError::Failed)} else {Ok(output)};
            }
            LlmEvent::ToolCall(_) => return Err(LlmError::Failed),
        }
    }
    Err(LlmError::Failed)
}

pub struct UnavailableLlm;

impl LlmProvider for UnavailableLlm {
    fn adapter(&self) -> &'static str {
        "unavailable"
    }
}

pub(crate) struct ConfiguredOpenAiLlm {
    api_key: String,
    base_url: String,
    model: String,
    timeout_seconds: u64,
    supports_responses: bool,
}

impl ConfiguredOpenAiLlm {
    pub(crate) fn build(
        api_key: &str, base_url: &str, model: &str, timeout_seconds: u64,
    ) -> Result<Self, LlmError> {
        Self::build_with_responses(api_key,base_url,model,timeout_seconds,false)
    }
    pub(crate) fn build_with_responses(
        api_key: &str, base_url: &str, model: &str, timeout_seconds: u64,
        supports_responses: bool,
    ) -> Result<Self,LlmError> {
        if model.trim().is_empty() || timeout_seconds == 0 {
            return Err(LlmError::Failed);
        }
        Ok(Self {
            api_key:api_key.into(),base_url:base_url.into(),
            model:model.into(),timeout_seconds,supports_responses,
        })
    }
}

#[async_trait::async_trait]
impl LlmProvider for ConfiguredOpenAiLlm {
    fn supports_tools(&self) -> bool {
        true
    }
    fn adapter(&self) -> &'static str {
        "openai"
    }

    async fn background_complete(&self, request: LlmRequest, schema: Option<Value>) -> Result<String,LlmError> {
        if self.supports_responses {
            openai::responses::complete(self,request,schema).await
        } else {
            complete_background_chat(self.stream(request).await?).await
        }
    }

    async fn stream(&self, request: LlmRequest) -> Result<LlmEventStream, LlmError> {
        let system = request.messages.iter().find_map(|message| match message {
            ChatMessage::System { content } => Some(content.as_str()),
            _ => None,
        });
        let mut builder = llm::builder::LLMBuilder::new()
            .backend(llm::builder::LLMBackend::OpenAI)
            .api_key(&self.api_key)
            .base_url(&self.base_url)
            .model(&self.model)
            .timeout_seconds(self.timeout_seconds);
        if let Some(system) = system {
            builder = builder.system(system);
        }
        let client = builder.build().map_err(|_| LlmError::Failed)?;
        stream_openai_request(client.as_ref(), request, self.adapter()).await
    }
}

async fn stream_openai_request(
    client: &dyn llm::chat::ChatProvider,
    request: LlmRequest,
    adapter: &'static str,
) -> Result<LlmEventStream, LlmError> {
    use futures_util::StreamExt;
    let messages = request
        .messages
        .iter()
        .filter(|message| !matches!(message, ChatMessage::System { .. }))
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
    let stream = client
        .chat_stream_with_tools(&messages, (!tools.is_empty()).then_some(tools.as_slice()))
        .await
        .map_err(|_| {
            tracing::warn!(
                event = "llm_provider_open_stream_failed",
                adapter,
                "OpenAI-compatible LLM stream could not be opened"
            );
            LlmError::Failed
        })?;
    Ok(Box::pin(stream.filter_map(|chunk| async move {
        adapt_stream_chunk(chunk)
    })))
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
        Err(_) => {
            tracing::warn!(
                event = "llm_provider_stream_chunk_failed",
                "OpenAI-compatible LLM stream chunk failed"
            );
            Some(Err(LlmError::Failed))
        }
    }
}

fn to_llm_message(message: &ChatMessage) -> llm::chat::ChatMessage {
    match message {
        // The upstream `llm` crate has no system-role ChatMessage. The configured
        // OpenAI client carries the system instruction separately; this arm is
        // unreachable after `ConfiguredOpenAiLlm::stream` removes it.
        ChatMessage::System { .. } => unreachable!("system prompt is configured separately"),
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
