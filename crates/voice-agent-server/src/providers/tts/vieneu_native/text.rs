//! SEA-G2P Vietnamese text frontend with no PyO3 or Python process.
//!
//! Rust matches SEAPipeline(vi): normalize first, then phonemize. The caller
//! owns phrase segmentation and should not pass an unbounded document.

use std::path::Path;

use sea_g2p_rs::{g2p::G2PEngine, lang::vi::Normalizer};

use crate::providers::tts::TtsError;

pub struct VieneuTextFrontend {
    normalizer: Normalizer,
    g2p: G2PEngine,
}

impl VieneuTextFrontend {
    pub fn load(dictionary: &Path) -> Result<Self, TtsError> {
        let path = dictionary.to_str().ok_or_else(|| {
            TtsError::IncompatibleContract("VieNeu: non-UTF8 G2P dictionary path".into())
        })?;
        let g2p = G2PEngine::new(path).map_err(|error| {
            TtsError::IncompatibleContract(format!("VieNeu G2P dictionary failed: {error}"))
        })?;
        Ok(Self {
            normalizer: Normalizer::new("vi", None),
            g2p,
        })
    }

    /// Vietnamese normalization and G2P with punctuation normalization as used
    /// by the upstream VieNeu CPU engine. Emotion cues require a separate parity
    /// gate before being accepted in production.
    pub fn phonemize(&self, text: &str) -> Result<String, TtsError> {
        let text = text.trim();
        if text.is_empty() || text.len() > 8_192 || text.chars().any(|c| c == '\0') {
            return Err(TtsError::Failed);
        }
        let normalized = self.normalizer.normalize(text, true);
        if normalized.trim().is_empty() {
            return Err(TtsError::Failed);
        }
        let phones = self.g2p.phonemize(&normalized);
        if phones.trim().is_empty() {
            return Err(TtsError::Failed);
        }
        Ok(phones)
    }
}
