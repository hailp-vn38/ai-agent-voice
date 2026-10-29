use crate::audio::PcmF32Mono;

use super::media_asr::{
    MAX_ASR_SEGMENT_SAMPLES, decode_wav_to_16khz_mono, split_pcm_into_asr_segments,
};

fn encode_pcm16_wav(sample_rate: u32, channels: u16, samples: &[i16]) -> Vec<u8> {
    let mut cursor = std::io::Cursor::new(Vec::new());
    let spec = hound::WavSpec {
        channels,
        sample_rate,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut writer = hound::WavWriter::new(&mut cursor, spec).unwrap();
    for &s in samples {
        writer.write_sample(s).unwrap();
    }
    writer.finalize().unwrap();
    cursor.into_inner()
}

fn encode_f32_wav(sample_rate: u32, channels: u16, samples: &[f32]) -> Vec<u8> {
    let mut cursor = std::io::Cursor::new(Vec::new());
    let spec = hound::WavSpec {
        channels,
        sample_rate,
        bits_per_sample: 32,
        sample_format: hound::SampleFormat::Float,
    };
    let mut writer = hound::WavWriter::new(&mut cursor, spec).unwrap();
    for &s in samples {
        writer.write_sample(s).unwrap();
    }
    writer.finalize().unwrap();
    cursor.into_inner()
}

fn rms_energy(samples: &[f32]) -> f32 {
    if samples.is_empty() {
        return 0.0;
    }
    let sum_sq: f32 = samples.iter().map(|s| s * s).sum();
    (sum_sq / (samples.len() as f32)).sqrt()
}

#[test]
fn decode_wav_to_16khz_mono_handles_formats_channels_rates_and_antialiasing() {
    // 1. 48 kHz stereo 16-bit PCM with 1 kHz tone vs 10 kHz tone (anti-aliasing check)
    let mut stereo_1khz = Vec::with_capacity(96_000);
    let mut stereo_10khz = Vec::with_capacity(96_000);
    for i in 0..48_000 {
        let t = (i as f32) / 48_000.0;
        let s1 = ((2.0 * std::f32::consts::PI * 1_000.0 * t).sin() * 20_000.0) as i16;
        let s10 = ((2.0 * std::f32::consts::PI * 10_000.0 * t).sin() * 20_000.0) as i16;
        stereo_1khz.extend_from_slice(&[s1, s1]);
        stereo_10khz.extend_from_slice(&[s10, s10]);
    }

    let pcm_1k = decode_wav_to_16khz_mono(&encode_pcm16_wav(48_000, 2, &stereo_1khz)).unwrap();
    let pcm_10k = decode_wav_to_16khz_mono(&encode_pcm16_wav(48_000, 2, &stereo_10khz)).unwrap();
    assert_eq!(pcm_1k.sample_rate_hz(), 16_000);
    assert_eq!(pcm_1k.samples().len(), 16_000);
    assert_eq!(pcm_10k.samples().len(), 16_000);

    let energy_1k = rms_energy(&pcm_1k.samples()[1_000..15_000]);
    let energy_10k = rms_energy(&pcm_10k.samples()[1_000..15_000]);
    assert!(
        energy_1k > 0.35,
        "1 kHz speech-band tone should be preserved (got rms={energy_1k})"
    );
    assert!(
        energy_10k < energy_1k * 0.25,
        "10 kHz out-of-band tone above 8 kHz Nyquist must be strongly attenuated (1k={energy_1k}, 10k={energy_10k})"
    );

    // 2. 8 kHz mono 16-bit PCM -> 16 kHz mono
    let samples_8k: Vec<i16> = (0..8_000)
        .map(|i| ((2.0 * std::f32::consts::PI * 440.0 * (i as f32) / 8_000.0).sin() * 16_000.0) as i16)
        .collect();
    let pcm_from_8k = decode_wav_to_16khz_mono(&encode_pcm16_wav(8_000, 1, &samples_8k)).unwrap();
    assert_eq!(pcm_from_8k.samples().len(), 16_000);

    // 3. 44.1 kHz 4-channel 32-bit float -> 16 kHz mono (multi-channel average)
    let mut float_4ch = Vec::with_capacity(44_100 * 4);
    for i in 0..44_100 {
        let s = (2.0 * std::f32::consts::PI * 1_000.0 * (i as f32) / 44_100.0).sin() * 0.6;
        float_4ch.extend_from_slice(&[s, s, s, s]);
    }
    let pcm_from_44k_float =
        decode_wav_to_16khz_mono(&encode_f32_wav(44_100, 4, &float_4ch)).unwrap();
    assert_eq!(pcm_from_44k_float.samples().len(), 16_000);
    assert!(rms_energy(&pcm_from_44k_float.samples()[1_000..15_000]) > 0.35);

    // 4. Segmentation on 35 seconds (560,000 samples) cuts at silence inside the [28s, 30s] window
    let mut raw = vec![0.2_f32; 35 * 16_000];
    let silence_start = 29 * 16_000;
    for s in &mut raw[silence_start..silence_start + 640] {
        *s = 0.0;
    }
    let long_pcm = PcmF32Mono::from_samples(raw);
    let segments = split_pcm_into_asr_segments(&long_pcm);
    assert_eq!(segments.len(), 2);
    assert!(segments[0].samples().len() <= MAX_ASR_SEGMENT_SAMPLES);
    assert!(
        (segments[0].samples().len() as isize - (silence_start as isize + 160)).abs() <= 320,
        "expected cut inside the low-energy silence window around 29s, got {}",
        segments[0].samples().len()
    );
    assert_eq!(
        segments[0].samples().len() + segments[1].samples().len(),
        35 * 16_000
    );
}
