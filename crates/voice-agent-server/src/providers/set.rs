use std::sync::Arc;

use crate::{
    config::AppConfig,
    providers::{
        AsrError, AsrProvider, AsrSession, ProviderLoadError, VadProvider, asr::UnavailableAsr,
        loader, vad::UnavailableVad,
    },
};

/// Application-owned provider pair injected into worker runtimes.
pub struct ProviderSet {
    asr: Arc<dyn AsrProvider>,
    vad: Arc<dyn VadProvider>,
}

impl ProviderSet {
    pub fn new(asr: Arc<dyn AsrProvider>) -> Self {
        Self::with_vad(Arc::new(UnavailableVad), asr)
    }
    pub fn with_vad(vad: Arc<dyn VadProvider>, asr: Arc<dyn AsrProvider>) -> Self {
        Self { asr, vad }
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
    pub fn vad_adapter(&self) -> &'static str {
        self.vad.adapter()
    }
    pub fn unavailable() -> Self {
        Self::new(Arc::new(UnavailableAsr))
    }
    pub fn load(config: &AppConfig) -> Result<Self, ProviderLoadError> {
        loader::load_local(config)
    }
}
