use std::sync::Arc;

use crate::{
    config::AppConfig,
    providers::{
        AsrError, AsrProvider, AsrSession, LlmProvider, ProviderLoadError, TtsProvider,
        VadProvider, asr::UnavailableAsr, llm::UnavailableLlm, loader, tts::UnavailableTts,
        vad::UnavailableVad,
    },
};

/// Application-owned provider pair injected into worker runtimes.
pub struct ProviderSet {
    asr: Arc<dyn AsrProvider>,
    vad: Arc<dyn VadProvider>,
    llm: Arc<dyn LlmProvider>,
    tts: Arc<dyn TtsProvider>,
}

impl ProviderSet {
    pub fn new(asr: Arc<dyn AsrProvider>) -> Self {
        Self::with_vad(Arc::new(UnavailableVad), asr)
    }
    pub fn with_vad(vad: Arc<dyn VadProvider>, asr: Arc<dyn AsrProvider>) -> Self {
        Self::with_all(vad, asr, Arc::new(UnavailableLlm), Arc::new(UnavailableTts))
    }
    pub fn with_llm_tts(llm: Arc<dyn LlmProvider>, tts: Arc<dyn TtsProvider>) -> Self {
        Self::with_all(Arc::new(UnavailableVad), Arc::new(UnavailableAsr), llm, tts)
    }
    pub fn with_all(
        vad: Arc<dyn VadProvider>,
        asr: Arc<dyn AsrProvider>,
        llm: Arc<dyn LlmProvider>,
        tts: Arc<dyn TtsProvider>,
    ) -> Self {
        Self { asr, vad, llm, tts }
    }
    pub fn open_asr(&self) -> Result<Box<dyn AsrSession>, AsrError> {
        self.asr.open()
    }
    pub(crate) fn asr_provider(&self) -> Arc<dyn AsrProvider> {
        Arc::clone(&self.asr)
    }
    pub(crate) fn vad_provider(&self) -> Arc<dyn VadProvider> {
        Arc::clone(&self.vad)
    }
    pub(crate) fn llm_provider(&self) -> Arc<dyn LlmProvider> {
        Arc::clone(&self.llm)
    }
    pub(crate) fn tts_provider(&self) -> Arc<dyn TtsProvider> {
        Arc::clone(&self.tts)
    }
    pub fn vad_adapter(&self) -> &'static str {
        self.vad.adapter()
    }
    pub fn llm_adapter(&self) -> &'static str {
        self.llm.adapter()
    }
    pub fn tts_adapter(&self) -> &'static str {
        self.tts.adapter()
    }
    pub fn unavailable() -> Self {
        Self::new(Arc::new(UnavailableAsr))
    }
    pub fn load(config: &AppConfig) -> Result<Self, ProviderLoadError> {
        loader::load_local(config)
    }
}
