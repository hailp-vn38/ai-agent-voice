use super::*;
use crate::providers::tts::vieneu::contract::NanoModelConfig;

fn sample_nano_config() -> NanoModelConfig {
    let mut vocab = std::collections::BTreeMap::new();
    vocab.insert("<pad>".into(), 0);
    vocab.insert("<bos>".into(), 1);
    vocab.insert("<eos>".into(), 2);
    vocab.insert(" ".into(), 3);
    vocab.insert("s".into(), 4);
    vocab.insert("i".into(), 5);
    vocab.insert("n".into(), 6);
    for idx in 7..82 {
        vocab.insert(format!("tok_{idx}"), idx);
    }
    NanoModelConfig {
        model_type: "vieneu_v3_nano".into(),
        sample_rate: 24_000,
        latent_dim: 24,
        group: 6,
        flow_fps: 15.625,
        dim: 512,
        style_dim: 256,
        n_style: 50,
        steps_default: 16,
        cfg_default: 3.0,
        pad_id: 0,
        bos_id: 1,
        eos_id: 2,
        vocab,
    }
}

#[test]
fn split_text_into_chunks_respects_140_char_cap_and_sentence_breaks() {
    let text = "Xin chào Việt Nam. Đây là mô hình tổng hợp tiếng nói siêu nhẹ chạy trực tiếp trên CPU với tốc độ rất nhanh! Câu thứ ba cũng được tách gọn gàng.";
    let chunks = split_text_into_chunks(text, NANO_MAX_CHUNK_CHARS);
    assert!(chunks.len() >= 2);
    for chunk in &chunks {
        assert!(!chunk.is_empty());
        assert!(chunk.chars().count() <= NANO_MAX_CHUNK_CHARS);
    }

    let long_no_punct = "a".repeat(300);
    let split_long = split_text_into_chunks(&long_no_punct, NANO_MAX_CHUNK_CHARS);
    assert_eq!(split_long.len(), 3);
    assert_eq!(split_long[0].chars().count(), 140);
    assert_eq!(split_long[1].chars().count(), 140);
    assert_eq!(split_long[2].chars().count(), 20);
}

#[test]
fn phonemes_to_ids_wraps_bos_and_eos_and_skips_unknown_chars() {
    let config = sample_nano_config();
    let (ids, mask) = phonemes_to_ids("sin?", &config).unwrap();
    assert_eq!(ids, vec![1, 4, 5, 6, 2]);
    assert_eq!(mask, vec![true; 5]);

    assert!(phonemes_to_ids("???", &config).is_err());
}

#[test]
fn trim_and_fade_handles_short_and_padded_waveforms_safely() {
    let short = vec![0.5_f32, -0.5, 0.25, -0.25];
    let out_short = trim_and_fade(&short, 24_000);
    assert_eq!(out_short.len(), 4);
    assert!(out_short.iter().all(|v| v.is_finite() && (-1.0..=1.0).contains(v)));

    // 100ms leading silence + 100ms active signal + 100ms trailing silence at 24kHz
    let mut padded = vec![0.0_f32; 2_400];
    padded.extend((0..2_400).map(|i| ((i as f32) * 0.1).sin() * 0.5));
    padded.extend(vec![0.0_f32; 2_400]);

    let trimmed = trim_and_fade(&padded, 24_000);
    assert!(trimmed.len() < padded.len());
    assert!(trimmed.len() >= 2_400);
    assert_eq!(trimmed[0], 0.0);
    assert_eq!(*trimmed.last().unwrap(), 0.0);
}

#[test]
fn sample_standard_normal_produces_finite_values_of_exact_length() {
    let samples = sample_standard_normal(145).unwrap();
    assert_eq!(samples.len(), 145);
    assert!(samples.iter().all(|v| v.is_finite()));
}

#[test]
fn latent_frames_scale_inversely_with_speed() {
    let fps = 15.625;
    let log_s = 2.0_f32.ln(); // 2 s natural duration
    assert_eq!(latent_frames_for(log_s, fps, 1.0), 31);
    assert_eq!(latent_frames_for(log_s, fps, 2.0), 16);
    assert_eq!(latent_frames_for(log_s, fps, 0.5), 63);
    assert_eq!(latent_frames_for(log_s, fps, 1.25), 25);
}

#[test]
fn latent_frames_keep_predictor_guards_and_minimum_length() {
    let fps = 15.625;
    // Natural duration is clamped to 15 s before the speed scale is applied.
    assert_eq!(latent_frames_for(100.0, fps, 1.0), 234);
    assert_eq!(latent_frames_for(100.0, fps, 0.5), 469);
    // Very short predictions never produce fewer than two latent frames.
    assert_eq!(latent_frames_for(-100.0, fps, 2.0), 2);
}
