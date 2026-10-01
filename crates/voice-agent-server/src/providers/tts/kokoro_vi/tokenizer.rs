use std::{collections::HashMap, fs, path::Path};

use serde::Deserialize;

use crate::providers::tts::TtsError;

#[derive(Clone)]
pub(super) struct Tokenizer {
    vocab: HashMap<char, i64>,
}

#[derive(Deserialize)]
struct KokoroConfig {
    vocab: HashMap<String, i64>,
}

impl Tokenizer {
    pub(super) fn load(path: &Path) -> Result<Self, TtsError> {
        let config: KokoroConfig =
            serde_json::from_slice(&fs::read(path).map_err(|_| TtsError::Failed)?)
                .map_err(|_| TtsError::Failed)?;
        let vocab = config
            .vocab
            .into_iter()
            .filter_map(|(token, id)| {
                let mut chars = token.chars();
                let character = chars.next()?;
                chars.next().is_none().then_some((character, id))
            })
            .collect();
        Ok(Self { vocab })
    }

    pub(super) fn encode(&self, phonemes: &str) -> Result<Vec<i64>, TtsError> {
        let ids: Vec<_> = phonemes
            .chars()
            .filter_map(|character| self.vocab.get(&character).copied())
            .collect();
        (!ids.is_empty()).then_some(ids).ok_or(TtsError::Failed)
    }
}

#[cfg(test)]
mod tests {
    use super::Tokenizer;

    #[test]
    fn unknown_phonemes_are_dropped_but_known_unicode_is_preserved() {
        let tokenizer = Tokenizer {
            vocab: [('a', 1), ('ɯ', 2)].into(),
        };
        assert_eq!(tokenizer.encode("a?ɯ").unwrap(), vec![1, 2]);
    }
}
