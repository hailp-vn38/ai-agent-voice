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
