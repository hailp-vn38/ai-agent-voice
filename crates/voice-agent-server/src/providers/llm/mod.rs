//! LLM provider boundary.

use thiserror::Error;

pub trait LlmProvider: Send + Sync {
    fn adapter(&self) -> &'static str;
    fn complete(&self, _: &str) -> Result<String, LlmError> {
        Err(LlmError::Failed)
    }
}

#[derive(Debug, Error)]
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

pub(crate) struct ConfiguredOpenAiLlm;

impl LlmProvider for ConfiguredOpenAiLlm {
    fn adapter(&self) -> &'static str {
        "openai"
    }
}
