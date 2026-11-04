use std::sync::Arc;

use sherpa_onnx::OfflineRecognizer;

use crate::providers::{
    AsrError, AsrProvider, AsrSession, asr::offline_session::SherpaOfflineAsrSession,
};

/// An offline SenseVoiceSmall recognizer shared by every ASR worker session.
pub(crate) struct SenseVoiceAsrProvider {
    pub(crate) recognizer: Arc<OfflineRecognizer>,
    pub(crate) max_buffered_samples: usize,
}

impl AsrProvider for SenseVoiceAsrProvider {
    fn open(&self) -> Result<Box<dyn AsrSession>, AsrError> {
        Ok(Box::new(SherpaOfflineAsrSession::new(
            Arc::clone(&self.recognizer),
            self.max_buffered_samples,
            "SenseVoice",
        )))
    }

    fn adapter(&self) -> &'static str {
        "sensevoice_sherpa_offline"
    }
}
