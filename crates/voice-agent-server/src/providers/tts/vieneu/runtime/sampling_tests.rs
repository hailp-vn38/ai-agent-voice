use super::*;

#[test]
fn nucleus_excludes_unlikely_codes_and_invalid_logits_fail() {
    for draw in [0.0, 0.5, 0.999999] {
        assert_eq!(
            sample_with_draw(&[20.0, -20.0], &VecDeque::new(), draw).unwrap(),
            0
        );
    }
    assert!(sample_with_draw(&[f32::NAN], &VecDeque::new(), 0.5).is_err());
    assert!(sample_with_draw(&[], &VecDeque::new(), 0.5).is_err());
}

#[test]
fn repetition_penalty_changes_ranking_and_history_expires() {
    let mut history = VecDeque::from([0]);
    assert_eq!(sample_with_draw(&[1.1, 1.0], &history, 0.0).unwrap(), 1);
    for _ in 0..64 {
        remember(&mut history, 1);
    }
    assert_eq!(history.len(), 64);
    assert!(!history.contains(&0));
    assert_eq!(sample_with_draw(&[1.1, 1.0], &history, 0.0).unwrap(), 0);
}
