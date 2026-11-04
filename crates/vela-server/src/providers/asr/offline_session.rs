use std::sync::Arc;

use sherpa_onnx::OfflineRecognizer;

use crate::{
    audio::PcmF32Mono,
    providers::{AsrError, AsrEvent, AsrResult, AsrSession},
};

/// Shared offline `sherpa-onnx` utterance session: buffers canonical 16 kHz PCM up to
/// `max_buffered_samples`, pads sub-second clips to 1 second of silence so convolutional
/// encoders never fail on single frames, and decodes once at `finish()`.
pub(crate) struct SherpaOfflineAsrSession {
    recognizer: Arc<OfflineRecognizer>,
    samples: Vec<f32>,
    cancelled: bool,
    max_buffered_samples: usize,
    label: &'static str,
}

impl SherpaOfflineAsrSession {
    pub(crate) fn new(
        recognizer: Arc<OfflineRecognizer>,
        max_buffered_samples: usize,
        label: &'static str,
    ) -> Self {
        Self {
            recognizer,
            samples: Vec::new(),
            cancelled: false,
            max_buffered_samples,
            label,
        }
    }
}

impl AsrSession for SherpaOfflineAsrSession {
    fn push_pcm(&mut self, pcm: &PcmF32Mono) -> Result<Vec<AsrEvent>, AsrError> {
        if self.cancelled {
            return Err(AsrError::Failed(format!(
                "{} ASR session is cancelled",
                self.label
            )));
        }
        if pcm.sample_rate_hz() != 16_000 {
            return Err(AsrError::Failed(format!(
                "{} ASR requires canonical 16 kHz PCM",
                self.label
            )));
        }
        let next_len = self.samples.len().saturating_add(pcm.samples().len());
        if next_len > self.max_buffered_samples {
            return Err(AsrError::Failed(format!(
                "{} ASR utterance exceeds the configured canonical audio limit",
                self.label
            )));
        }
        self.samples.extend_from_slice(pcm.samples());
        Ok(Vec::new())
    }

    fn finish(&mut self) -> Result<AsrResult, AsrError> {
        if self.cancelled {
            return Err(AsrError::Failed(format!(
                "{} ASR session is cancelled",
                self.label
            )));
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
            .ok_or_else(|| AsrError::Failed(format!("{} returned no final result", self.label)))?;
        let utterance_duration_ms = ((self.samples.len() as u64) * 1_000) / 16_000;
        self.samples.clear();
        let text = result.text.trim().to_owned();
        let segments = crate::providers::asr::subtitle_segmenter::segment_timed_tokens(
            &text,
            &result.tokens,
            result.timestamps.as_deref(),
            utterance_duration_ms,
        );
        Ok(AsrResult::with_segments(text, segments))
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
