use crate::providers::VadProbability;

/// Semantic configuration for probability-level VAD segmentation.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct VadSegmenterConfig {
    pub speech_threshold: f32,
    pub exit_threshold: f32,
    pub min_speech_samples: u64,
    pub end_silence_samples: u64,
}

impl Default for VadSegmenterConfig {
    fn default() -> Self {
        Self {
            speech_threshold: 0.50,
            exit_threshold: 0.35,
            min_speech_samples: 2_880,  // 180 ms at 16 kHz
            end_silence_samples: 9_600, // 600 ms at 16 kHz
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VadBoundary {
    SpeechStart { start_sample: u64 },
    SpeechEnd { end_sample: u64 },
}

/// An invalid probability range means the Auto cycle can no longer prove its timeline.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VadSegmenterError {
    StreamIntegrity,
}

/// Converts contiguous provider probabilities into stable utterance boundaries.
pub struct VadSegmenter {
    config: VadSegmenterConfig,
    expected_start_sample: Option<u64>,
    speech_candidate_start: Option<u64>,
    silence_candidate_start: Option<u64>,
    speech_active: bool,
}

impl VadSegmenter {
    pub fn new(config: VadSegmenterConfig) -> Self {
        assert!(
            config.exit_threshold < config.speech_threshold
                && config.speech_threshold <= 1.0
                && config.exit_threshold >= 0.0
                && config.min_speech_samples > 0
                && config.end_silence_samples > 0,
            "invalid VAD segmentation configuration"
        );
        Self {
            config,
            expected_start_sample: None,
            speech_candidate_start: None,
            silence_candidate_start: None,
            speech_active: false,
        }
    }

    pub fn observe(
        &mut self,
        probability: VadProbability,
    ) -> Result<Option<VadBoundary>, VadSegmenterError> {
        if !probability.probability.is_finite()
            || !(0.0..=1.0).contains(&probability.probability)
            || probability.end_sample <= probability.start_sample
            || self
                .expected_start_sample
                .is_some_and(|expected| expected != probability.start_sample)
        {
            return Err(VadSegmenterError::StreamIntegrity);
        }
        self.expected_start_sample = Some(probability.end_sample);

        if !self.speech_active {
            if probability.probability >= self.config.speech_threshold {
                let onset = *self
                    .speech_candidate_start
                    .get_or_insert(probability.start_sample);
                if probability.end_sample - onset >= self.config.min_speech_samples {
                    self.speech_active = true;
                    self.speech_candidate_start = None;
                    return Ok(Some(VadBoundary::SpeechStart {
                        start_sample: onset,
                    }));
                }
            } else if probability.probability < self.config.exit_threshold {
                self.speech_candidate_start = None;
            }
            return Ok(None);
        }

        if probability.probability < self.config.exit_threshold {
            let onset = *self
                .silence_candidate_start
                .get_or_insert(probability.start_sample);
            if probability.end_sample - onset >= self.config.end_silence_samples {
                self.speech_active = false;
                self.silence_candidate_start = None;
                return Ok(Some(VadBoundary::SpeechEnd { end_sample: onset }));
            }
        } else if probability.probability >= self.config.speech_threshold {
            self.silence_candidate_start = None;
        }
        Ok(None)
    }

    pub fn reset(&mut self) {
        self.expected_start_sample = None;
        self.speech_candidate_start = None;
        self.silence_candidate_start = None;
        self.speech_active = false;
    }
}

impl Default for VadSegmenter {
    fn default() -> Self {
        Self::new(VadSegmenterConfig::default())
    }
}
