//! Deterministic Qualification Speaker provider (ADR 0068).
//!
//! Compiled only into the qualification build. It implements the same extractor
//! contract as CAM++ without native models, so the Integration Harness can drive
//! the production entrypoint with no model download and no credentials.

use super::{SpeakerError, SpeakerProvider};
use crate::audio::PcmF32Mono;

/// Maps clip loudness to an angle on the unit circle: identical recordings
/// collapse to one point, a deliberately different speaker lands far away.
/// Same deterministic shape the enrollment tests use, promoted to a real adapter.
pub struct QualificationSpeaker;

impl SpeakerProvider for QualificationSpeaker {
    fn dimension(&self) -> usize {
        3
    }

    fn extract(&mut self, pcm: &PcmF32Mono) -> Result<Vec<f32>, SpeakerError> {
        let samples = pcm.samples();
        if samples.is_empty() {
            return Err(SpeakerError::InvalidInput);
        }
        let rms = (samples.iter().map(|sample| sample * sample).sum::<f32>()
            / samples.len() as f32)
            .sqrt();
        let angle = (rms * std::f32::consts::PI * 4.0).rem_euclid(std::f32::consts::TAU);
        Ok(vec![angle.cos(), angle.sin(), 0.0])
    }
}

pub fn build(_threads: i32) -> Result<Box<dyn SpeakerProvider>, SpeakerError> {
    Ok(Box::new(QualificationSpeaker))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::PcmF32Mono;

    fn clip(amplitude: f32) -> PcmF32Mono {
        PcmF32Mono::new(vec![amplitude, -amplitude].repeat(8_000), 16_000)
    }

    #[test]
    fn same_loudness_maps_to_the_same_point_and_other_loudness_does_not() {
        let mut speaker = QualificationSpeaker;
        let first = speaker.extract(&clip(0.5)).unwrap();
        let again = speaker.extract(&clip(0.5)).unwrap();
        let other = speaker.extract(&clip(0.8)).unwrap();
        assert_eq!(first, again);
        assert_ne!(first, other);
    }
}
