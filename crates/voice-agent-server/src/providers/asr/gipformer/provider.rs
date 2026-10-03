use std::sync::Arc;

use sherpa_onnx::OfflineRecognizer;

use crate::{
    audio::PcmF32Mono,
    providers::{AsrError, AsrEvent, AsrProvider, AsrResult, AsrSession},
};

/// An offline Gipformer recognizer shared by every ASR worker session.
pub(crate) struct GipformerAsrProvider {
    pub(crate) recognizer: Arc<OfflineRecognizer>,
    pub(crate) max_buffered_samples: usize,
}

impl AsrProvider for GipformerAsrProvider {
    fn open(&self) -> Result<Box<dyn AsrSession>, AsrError> {
        Ok(Box::new(GipformerAsrSession {
            recognizer: Arc::clone(&self.recognizer),
            samples: Vec::new(),
            cancelled: false,
            max_buffered_samples: self.max_buffered_samples,
        }))
    }
}

struct GipformerAsrSession {
    recognizer: Arc<OfflineRecognizer>,
    samples: Vec<f32>,
    cancelled: bool,
    max_buffered_samples: usize,
}

impl AsrSession for GipformerAsrSession {
    fn push_pcm(&mut self, pcm: &PcmF32Mono) -> Result<Vec<AsrEvent>, AsrError> {
        if self.cancelled {
            return Err(AsrError::Failed(
                "Gipformer ASR session is cancelled".into(),
            ));
        }
        if pcm.sample_rate_hz() != 16_000 {
            return Err(AsrError::Failed(
                "Gipformer ASR requires canonical 16 kHz PCM".into(),
            ));
        }
        let next_len = self.samples.len().saturating_add(pcm.samples().len());
        if next_len > self.max_buffered_samples {
            return Err(AsrError::Failed(
                "Gipformer ASR utterance exceeds the configured canonical audio limit".into(),
            ));
        }
        self.samples.extend_from_slice(pcm.samples());
        Ok(Vec::new())
    }

    fn finish(&mut self) -> Result<AsrResult, AsrError> {
        if self.cancelled {
            return Err(AsrError::Failed(
                "Gipformer ASR session is cancelled".into(),
            ));
        }
        if self.samples.is_empty() {
            return Ok(AsrResult::new(""));
        }

        let stream = self.recognizer.create_stream();
        // The offline encoder's convolution cannot decode a single Voice frame. Pad only
        // the inference input with bounded silence; the utterance buffer keeps its limit.
        let padded;
        let samples = if self.samples.len() < 16_000 {
            padded = {
                let mut input = self.samples.clone();
                input.resize(16_000, 0.0);
                input
            };
            &padded
        } else {
            &self.samples
        };
        stream.accept_waveform(16_000, samples);
        self.recognizer.decode(&stream);
        let result = stream
            .get_result()
            .ok_or_else(|| AsrError::Failed("Gipformer returned no final result".into()))?;
        self.samples.clear();
        Ok(AsrResult::new(result.text.trim().to_owned()))
    }

    fn reset(&mut self) -> Result<(), AsrError> {
        self.samples.clear();
        self.cancelled = false;
        Ok(())
    }

    fn cancel(&mut self) {
        self.cancelled = true;
        self.samples.clear();
    }
}
