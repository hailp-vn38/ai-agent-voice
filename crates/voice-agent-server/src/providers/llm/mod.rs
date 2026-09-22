//! LLM provider boundary. Operations are added by the LLM runtime ticket.

pub trait LlmProvider: Send + Sync {
    fn adapter(&self) -> &'static str;
}

pub struct UnavailableLlm;

impl LlmProvider for UnavailableLlm {
    fn adapter(&self) -> &'static str {
        "unavailable"
    }
}

pub(crate) struct ConfiguredOpenAiLlm;

impl LlmProvider for ConfiguredOpenAiLlm {
    fn adapter(&self) -> &'static str {
        "openai"
    }
}
