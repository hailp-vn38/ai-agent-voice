use super::{GAP_SAMPLES, PromptAssets};

#[test]
fn leading_zero_is_a_digit_and_pcm_order_is_exact_before_lossy_encoding() {
    let assets = PromptAssets {
        intro: vec![100; 1_000],
        digits: (0..10)
            .map(|digit| vec![1_000 + digit as i16; 1_000])
            .collect(),
    };
    let pcm = assets.assemble("042731").unwrap();
    let mut offset = 1_000;
    for digit in [0, 4, 2, 7, 3, 1] {
        offset += GAP_SAMPLES;
        assert_eq!(pcm[offset + 500], 1_000 + digit);
        offset += 1_000;
    }
    assert_eq!(pcm.len() % 1_440, 0);
    assert_eq!(*pcm.last().unwrap(), 0);
}
