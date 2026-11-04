use crate::providers::llm::{
    ChatMessage, LlmError, LlmEventStream, LlmOutput, LlmProvider, LlmRequest, collect_chat_stream,
    stream_openai_request,
};

use super::provider::ConfiguredOpenAiLlm;

impl ConfiguredOpenAiLlm {
    pub(super) async fn open_chat_stream(
        &self,
        request: LlmRequest,
        force_reasoning: bool,
    ) -> Result<LlmEventStream, LlmError> {
        let system = request.messages.iter().find_map(|message| match message {
            ChatMessage::System { content } => Some(content.as_str()),
            _ => None,
        });
        let mut builder = llm::builder::LLMBuilder::new()
            .backend(llm::builder::LLMBackend::OpenAI)
            .api_key(&self.api_key)
            .base_url(&self.base_url_str)
            .model(&self.model)
            .timeout_seconds(self.timeout_seconds);
        if let Some(system) = system {
            builder = builder.system(system);
        }
        if let Some(max_tokens) = self.max_tokens {
            builder = builder.max_tokens(max_tokens);
        }
        if let Some(temperature) = self.temperature() {
            builder = builder.temperature(temperature);
        }
        if let Some(effort) = self.resolved_reasoning_effort(force_reasoning) {
            builder = builder.reasoning(true).reasoning_effort(effort);
        }
        let client = builder.build().map_err(|_| LlmError::Failed)?;
        stream_openai_request(client.as_ref(), request, self.adapter()).await
    }

    pub(super) async fn complete_chat(
        &self,
        request: LlmRequest,
        force_reasoning: bool,
    ) -> Result<LlmOutput, LlmError> {
        let stream = self.open_chat_stream(request, force_reasoning).await?;
        collect_chat_stream(stream).await
    }
}
