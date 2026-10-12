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

/// The pinned upstream frame cap prevents short phrases from growing extra speech.
pub(crate) fn frame_limit(phonemes: &str) -> usize {
    let length = phonemes.chars().count();
    let mut syllables = 0;
    for token in phonemes.split_whitespace() {
        let (mut groups, mut in_vowel, mut consonant_seen) = (0, false, true);
        for ch in token.chars() {
            if "aeiouyæɐɑɒɔəɘɛɜɤɯɵøœʉʊʌɪɨɚɝᵻᵿ".contains(ch) {
                if !in_vowel && consonant_seen {
                    groups += 1;
                }
                in_vowel = true;
                consonant_seen = false;
            } else if "ːˈˌ".contains(ch) || ch.is_ascii_digit() {
                if "ˈˌ".contains(ch) && groups > 0 {
                    consonant_seen = true;
                }
                in_vowel = false;
            } else {
                in_vowel = false;
                consonant_seen = true;
            }
        }
        if token.chars().any(char::is_alphabetic) {
            syllables += groups.max(1);
        }
    }
    let syllables = syllables.max(1);
    let cap = (24 + 2 * length).min(300);
    if syllables <= 4 && length <= 24 * syllables {
        cap.min(13 + 5 * (syllables - 1))
    } else {
        cap
    }
}

#[cfg(test)]
#[path = "text_tests.rs"]
mod tests;

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
