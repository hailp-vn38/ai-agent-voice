use std::{collections::HashMap, sync::Arc};

use crate::providers::{AsrProvider, LlmProvider, TtsProvider, VadProvider};

#[derive(Debug, thiserror::Error)]
pub enum ProviderLookupError {
    #[error("unknown {kind} provider instance `{id}`")]
    Unknown { kind: &'static str, id: String },
}

/// Read-only configured provider instances loaded at application startup.
pub struct ProviderCatalog {
    pub(crate) vad: HashMap<String, Arc<dyn VadProvider>>,
    pub(crate) asr: HashMap<String, Arc<dyn AsrProvider>>,
    pub(crate) llm: HashMap<String, Arc<dyn LlmProvider>>,
    pub(crate) tts: HashMap<String, Arc<dyn TtsProvider>>,
}

impl ProviderCatalog {
    pub fn vad(&self, id: &str) -> Result<Arc<dyn VadProvider>, ProviderLookupError> {
        self.vad
            .get(id)
            .cloned()
            .ok_or_else(|| ProviderLookupError::Unknown {
                kind: "VAD",
                id: id.into(),
            })
    }

    pub fn asr(&self, id: &str) -> Result<Arc<dyn AsrProvider>, ProviderLookupError> {
        self.asr
            .get(id)
            .cloned()
            .ok_or_else(|| ProviderLookupError::Unknown {
                kind: "ASR",
                id: id.into(),
            })
    }
    pub fn llm(&self, id: &str) -> Result<Arc<dyn LlmProvider>, ProviderLookupError> {
        self.llm
            .get(id)
            .cloned()
            .ok_or_else(|| ProviderLookupError::Unknown {
                kind: "LLM",
                id: id.into(),
            })
    }
    pub fn tts(&self, id: &str) -> Result<Arc<dyn TtsProvider>, ProviderLookupError> {
        self.tts
            .get(id)
            .cloned()
            .ok_or_else(|| ProviderLookupError::Unknown {
                kind: "TTS",
                id: id.into(),
            })
    }
}
