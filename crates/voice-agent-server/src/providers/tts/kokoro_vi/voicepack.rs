use std::{fs, path::Path};

use crate::providers::tts::TtsError;

const MAGIC: &[u8] = b"KOVI_VOICEPACK_V1";
const CHANNELS: u32 = 1;
const DIMENSION: u32 = 256;

#[derive(Clone)]
pub(super) struct Voicepack {
    rows: usize,
    values: Vec<f32>,
}

impl Voicepack {
    pub(super) fn load(path: &Path) -> Result<Self, TtsError> {
        let bytes = fs::read(path).map_err(|_| TtsError::Failed)?;
        let header = MAGIC.len() + 12;
        if bytes.len() < header || &bytes[..MAGIC.len()] != MAGIC {
            return Err(TtsError::IncompatibleContract(
                "Kokoro voicepack must use KOVI_VOICEPACK_V1".into(),
            ));
        }
        let read_u32 = |offset: usize| -> u32 {
            u32::from_le_bytes(bytes[offset..offset + 4].try_into().expect("fixed header"))
        };
        let rows = read_u32(MAGIC.len()) as usize;
        let channels = read_u32(MAGIC.len() + 4);
        let dimension = read_u32(MAGIC.len() + 8);
        if rows == 0 || channels != CHANNELS || dimension != DIMENSION {
            return Err(TtsError::IncompatibleContract(
                "Kokoro voicepack has an incompatible shape".into(),
            ));
        }
        let expected = header + rows * DIMENSION as usize * std::mem::size_of::<f32>();
        if bytes.len() != expected {
            return Err(TtsError::IncompatibleContract(
                "Kokoro voicepack has a truncated payload".into(),
            ));
        }
        let values = bytes[header..]
            .chunks_exact(4)
            .map(|chunk| f32::from_le_bytes(chunk.try_into().expect("four-byte chunk")))
            .collect::<Vec<_>>();
        if values.iter().any(|sample| !sample.is_finite()) {
            return Err(TtsError::IncompatibleContract(
                "Kokoro voicepack contains non-finite values".into(),
            ));
        }
        Ok(Self { rows, values })
    }

    pub(super) fn latent(&self, phoneme_count: usize) -> &[f32] {
        let row = phoneme_count.saturating_sub(1).min(self.rows - 1);
        let start = row * DIMENSION as usize;
        &self.values[start..start + DIMENSION as usize]
    }
}
