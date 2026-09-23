use crate::audio::PcmF32Mono;

/// Actor-owned bounded PCM storage for onset-relative Auto pre-roll.
pub(super) struct AutoPcmRetention {
    samples: Vec<f32>,
    first_sample: u64,
    cursor: u64,
    capacity: usize,
}
impl AutoPcmRetention {
    pub(super) fn new(capacity: usize) -> Self {
        Self {
            samples: Vec::with_capacity(capacity),
            first_sample: 0,
            cursor: 0,
            capacity,
        }
    }
    pub(super) fn push(&mut self, pcm: &PcmF32Mono) {
        debug_assert_eq!(pcm.sample_rate_hz(), 16_000);
        self.samples.extend_from_slice(pcm.samples());
        self.cursor += pcm.samples().len() as u64;
        let overflow = self.samples.len().saturating_sub(self.capacity);
        if overflow > 0 {
            self.samples.drain(..overflow);
            self.first_sample += overflow as u64;
        }
    }
    pub(super) fn range(&self, start_sample: u64) -> Option<PcmF32Mono> {
        if start_sample < self.first_sample || start_sample > self.cursor {
            return None;
        }
        let offset = (start_sample - self.first_sample) as usize;
        Some(PcmF32Mono::from_samples(self.samples[offset..].to_vec()))
    }
    pub(super) fn reset(&mut self) {
        self.samples.clear();
        self.first_sample = 0;
        self.cursor = 0;
    }
}
pub(super) fn auto_retention_capacity(
    vad_command_capacity: usize,
    confirmation_samples: u64,
    pre_roll_samples: u64,
) -> usize {
    const FRAME_SAMPLES: usize = 960;
    const RECHUNK_SLACK_SAMPLES: usize = 512;
    pre_roll_samples as usize
        + confirmation_samples as usize
        + vad_command_capacity.saturating_mul(FRAME_SAMPLES)
        + FRAME_SAMPLES
        + RECHUNK_SLACK_SAMPLES
}
