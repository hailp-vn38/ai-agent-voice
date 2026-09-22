//! Vendor-neutral LLM streaming boundary.

use std::pin::Pin;

use futures_util::Stream;
use thiserror::Error;

pub type LlmEventStream = Pin<Box<dyn Stream<Item = Result<LlmEvent, LlmError>> + Send>>;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LlmEvent {
    TextDelta(String),
    UnexpectedToolCall,
    Finished,
}

#[async_trait::async_trait]
pub trait LlmProvider: Send + Sync {
    fn adapter(&self) -> &'static str;
    fn complete(&self, _: &str) -> Result<String, LlmError> {
        Err(LlmError::Failed)
    }

    async fn stream(&self, prompt: String) -> Result<LlmEventStream, LlmError> {
        let response = self.complete(&prompt)?;
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

    async fn stream(&self, prompt: String) -> Result<LlmEventStream, LlmError> {
        use futures_util::StreamExt;
        let messages = [llm::chat::ChatMessage::user().content(prompt).build()];
        let stream = self
            .client
            .chat_stream_with_tools(&messages, None)
            .await
            .map_err(|_| LlmError::Failed)?;
        Ok(Box::pin(stream.map(|chunk| match chunk {
            Ok(llm::chat::StreamChunk::Text(text)) => Ok(LlmEvent::TextDelta(text)),
            Ok(llm::chat::StreamChunk::Done { .. }) => Ok(LlmEvent::Finished),
            Ok(_) => Ok(LlmEvent::UnexpectedToolCall),
            Err(_) => Err(LlmError::Failed),
        })))
    }
}
