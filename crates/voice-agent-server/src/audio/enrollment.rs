//! Enrollment sample contract: pinned preprocessing, bounded WAV parsing, and the audio
//! quality gate that runs before an embedding is stored.
//!
//! The browser recorder owns normalization; the server still verifies the exact encoding and
//! runs its own quality pass. Nothing here resamples: a 48 kHz clip is rejected, not relabelled
//! to 16 kHz.

use std::io::Cursor;

use super::PcmF32Mono;

/// Pinned preprocessing the recorder must produce. Bumping this invalidates stored samples.
pub const PREPROCESSING_CONTRACT: &str = "pcm16-mono16k-v1";

/// Exact sample rate enrollment accepts.
pub const SAMPLE_RATE_HZ: u32 = 16_000;
/// Hard cap on decoded clip length; longer WAVs are refused before inference.
pub const MAX_CLIP_MS: u64 = 12_000;
/// Longest window the extractor accepts; provider window config is clamped to this.
pub const MAX_WINDOW_MS: u64 = 6_000;
/// Upper bound on a stored embedding dimension.
pub const MAX_EMBEDDING_DIMS: usize = 4096;

const SPEECH_FRAME_MS: u64 = 20;
/// A 20 ms frame counts as speech above this RMS (~-60 dBFS).
const SPEECH_RMS: f32 = 0.001;
const CLIPPING_AMPLITUDE: f32 = 0.999;
const MAX_CLIPPING_FRACTION: f64 = 0.05;

/// Quality/window rules for one clip. Sourced from deployment config and the pinned provider
/// calibration, never from a user-supplied threshold.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct QualityProfile {
    pub min_clip_ms: u64,
    pub max_clip_ms: u64,
    pub min_speech_ms: u64,
    pub max_window_ms: u64,
}

/// Bounded quality metadata surfaced to the wizard. No audio or embedding leaves here.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SampleQuality {
    pub duration_ms: u64,
    pub speech_ms: u64,
}

/// A clip that failed the quality gate. The draft is never mutated for these.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reject {
    TooShort,
    TooLong,
    InsufficientAudio,
    Clipped,
}

/// A body that is not a usable enrollment WAV.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WavReject {
    /// Not a well-formed RIFF/WAVE stream.
    Malformed,
    /// Well-formed WAV but not 16-bit mono PCM at 16 kHz.
    UnsupportedFormat,
    /// Decodes to more than [`MAX_CLIP_MS`].
    TooLong,
}

/// A runtime embedding that cannot be trusted for storage.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EmbeddingReject {
    DimensionMismatch,
    NotFinite,
    Degenerate,
}

/// Decode a bounded PCM16 mono 16 kHz WAV body.
pub fn parse_wav(body: &[u8]) -> Result<(PcmF32Mono, u64), WavReject> {
    let reader = hound::WavReader::new(Cursor::new(body)).map_err(|_| WavReject::Malformed)?;
    let spec = reader.spec();
    if spec.channels != 1
        || spec.sample_rate != SAMPLE_RATE_HZ
        || spec.bits_per_sample != 16
        || spec.sample_format != hound::SampleFormat::Int
    {
        return Err(WavReject::UnsupportedFormat);
    }
    let samples: Vec<f32> = reader
        .into_samples::<i16>()
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| WavReject::Malformed)?
        .into_iter()
        .map(|sample| f32::from(sample) / 32_768.0)
        .collect();
    if samples.is_empty() {
        return Err(WavReject::Malformed);
    }
    let duration_ms = samples.len() as u64 * 1000 / u64::from(SAMPLE_RATE_HZ);
    if duration_ms > MAX_CLIP_MS {
        return Err(WavReject::TooLong);
    }
    Ok((PcmF32Mono::new(samples, SAMPLE_RATE_HZ), duration_ms))
}

/// A clip plus the highest-speech window the extractor will consume.
#[derive(Clone, Debug, PartialEq)]
pub struct Analyzed {
    pub quality: SampleQuality,
    pub window: PcmF32Mono,
}

/// Run the quality gate and pick the extraction window.
///
/// Duration and clipping are judged on the whole clip; speech energy on the selected window,
/// so silence around a real utterance never counts toward the speech floor.
pub fn analyze(
    clip: &PcmF32Mono,
    duration_ms: u64,
    profile: &QualityProfile,
) -> Result<Analyzed, Reject> {
    if duration_ms < profile.min_clip_ms {
        return Err(Reject::TooShort);
    }
    if duration_ms > profile.max_clip_ms {
        return Err(Reject::TooLong);
    }
    let samples = clip.samples();
    let clipped = samples
        .iter()
        .filter(|sample| sample.abs() >= CLIPPING_AMPLITUDE)
        .count() as f64
        / samples.len() as f64;
    if clipped > MAX_CLIPPING_FRACTION {
        return Err(Reject::Clipped);
    }
    let window = select_window(clip, profile.max_window_ms.min(MAX_WINDOW_MS));
    let speech_ms = speech_ms(window.samples(), window.sample_rate_hz());
    if speech_ms < profile.min_speech_ms {
        return Err(Reject::InsufficientAudio);
    }
    Ok(Analyzed {
        quality: SampleQuality {
            duration_ms,
            speech_ms,
        },
        window,
    })
}

/// Pick the `max_window_ms` slice with the most speech energy. Short clips are returned whole.
pub fn select_window(clip: &PcmF32Mono, max_window_ms: u64) -> PcmF32Mono {
    let samples = clip.samples();
    let rate = clip.sample_rate_hz();
    let max_len = (max_window_ms * u64::from(rate) / 1000) as usize;
    if max_len == 0 || samples.len() <= max_len {
        return clip.clone();
    }
    let frame_len = (SPEECH_FRAME_MS * u64::from(rate) / 1000) as usize;
    if frame_len == 0 || max_len < frame_len {
        return clip.clone();
    }
    // Score speech per frame, then slide a `max_len` window and keep the highest-energy start.
    let mut speech = Vec::with_capacity(samples.len().div_ceil(frame_len));
    let mut index = 0;
    while index < samples.len() {
        let end = (index + frame_len).min(samples.len());
        speech.push(u32::from(rms(&samples[index..end]) >= SPEECH_RMS));
        index += frame_len;
    }
    let frame_count = speech.len();
    let window_frames = max_len.div_ceil(frame_len);
    if window_frames >= frame_count {
        return clip.clone();
    }
    let mut running: u32 = speech[..window_frames].iter().sum();
    let mut best_start = 0usize;
    let mut best_speech = running;
    for start in 1..=(frame_count - window_frames) {
        running = running - speech[start - 1] + speech[start + window_frames - 1];
        if running > best_speech {
            best_speech = running;
            best_start = start;
        }
    }
    let start = best_start * frame_len;
    let end = (start + max_len).min(samples.len());
    PcmF32Mono::new(samples[start..end].to_vec(), rate)
}

/// Count frames whose RMS clears the speech floor. This is a gate, not a speaker detector.
fn speech_ms(samples: &[f32], rate: u32) -> u64 {
    let frame_len = (SPEECH_FRAME_MS * u64::from(rate) / 1000) as usize;
    if frame_len == 0 {
        return 0;
    }
    let mut frames = 0u64;
    let mut index = 0;
    while index + frame_len <= samples.len() {
        if rms(&samples[index..index + frame_len]) >= SPEECH_RMS {
            frames += 1;
        }
        index += frame_len;
    }
    frames * SPEECH_FRAME_MS
}

fn rms(samples: &[f32]) -> f32 {
    if samples.is_empty() {
        return 0.0;
    }
    (samples.iter().map(|s| s * s).sum::<f32>() / samples.len() as f32).sqrt()
}

/// Check a runtime embedding before it is stored: exact dimension, all finite, non-degenerate.
pub fn validate_embedding(embedding: &[f32], expected_dims: usize) -> Result<(), EmbeddingReject> {
    if embedding.is_empty()
        || embedding.len() > MAX_EMBEDDING_DIMS
        || embedding.len() != expected_dims
    {
        return Err(EmbeddingReject::DimensionMismatch);
    }
    if embedding.iter().any(|value| !value.is_finite()) {
        return Err(EmbeddingReject::NotFinite);
    }
    let norm = embedding
        .iter()
        .map(|value| value * value)
        .sum::<f32>()
        .sqrt();
    if !norm.is_finite() || norm <= f32::EPSILON {
        return Err(EmbeddingReject::Degenerate);
    }
    Ok(())
}

/// Store an embedding as little-endian `f32` bytes.
pub fn encode_embedding(embedding: &[f32]) -> Vec<u8> {
    embedding
        .iter()
        .flat_map(|value| value.to_le_bytes())
        .collect()
}

/// L2-normalize in place. Returns `false` if the vector has no usable magnitude.
pub fn normalize(embedding: &mut [f32]) -> bool {
    let norm = embedding
        .iter()
        .map(|value| value * value)
        .sum::<f32>()
        .sqrt();
    if !norm.is_finite() || norm <= f32::EPSILON {
        return false;
    }
    for value in embedding.iter_mut() {
        *value /= norm;
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wav(samples: &[i16], rate: u32, channels: u16, bits: u16) -> Vec<u8> {
        let spec = hound::WavSpec {
            channels,
            sample_rate: rate,
            bits_per_sample: bits,
            sample_format: hound::SampleFormat::Int,
        };
        let mut buffer = Cursor::new(Vec::new());
        let mut writer = hound::WavWriter::new(&mut buffer, spec).unwrap();
        for sample in samples {
            writer.write_sample(*sample).unwrap();
        }
        writer.finalize().unwrap();
        buffer.into_inner()
    }

    fn tone(ms: u64, amplitude: i16) -> Vec<i16> {
        let count = (ms * u64::from(SAMPLE_RATE_HZ) / 1000) as usize;
        (0..count)
            .map(|i| if i % 2 == 0 { amplitude } else { -amplitude })
            .collect()
    }

    #[test]
    fn parses_pcm16_mono_16k() {
        let body = wav(&tone(6000, 8000), SAMPLE_RATE_HZ, 1, 16);
        let (pcm, duration_ms) = parse_wav(&body).unwrap();
        assert_eq!(pcm.sample_rate_hz(), SAMPLE_RATE_HZ);
        assert_eq!(pcm.samples().len(), 96_000);
        assert_eq!(duration_ms, 6000);
    }

    #[test]
    fn rejects_stereo_and_other_rates_without_relabelling() {
        assert_eq!(
            parse_wav(&wav(&tone(6000, 8000), SAMPLE_RATE_HZ, 2, 16)),
            Err(WavReject::UnsupportedFormat)
        );
        assert_eq!(
            parse_wav(&wav(&tone(6000, 8000), 48_000, 1, 16)),
            Err(WavReject::UnsupportedFormat)
        );
        assert_eq!(parse_wav(b"not a wav"), Err(WavReject::Malformed));
    }

    #[test]
    fn rejects_clips_longer_than_the_cap() {
        let body = wav(&tone(13_000, 8000), SAMPLE_RATE_HZ, 1, 16);
        assert_eq!(parse_wav(&body), Err(WavReject::TooLong));
    }

    #[test]
    fn quality_gate_accepts_a_speech_like_clip() {
        let profile = QualityProfile {
            min_clip_ms: 5_000,
            max_clip_ms: 10_000,
            min_speech_ms: 3_000,
            max_window_ms: 6_000,
        };
        let (pcm, duration_ms) =
            parse_wav(&wav(&tone(8_000, 8_000), SAMPLE_RATE_HZ, 1, 16)).unwrap();
        let analyzed = analyze(&pcm, duration_ms, &profile).unwrap();
        assert_eq!(analyzed.quality.duration_ms, 8_000);
        assert!(analyzed.quality.speech_ms >= 3_000);
        assert_eq!(analyzed.window.samples().len(), 6_000 * 16);
    }

    #[test]
    fn quality_gate_rejects_short_silent_and_clipped() {
        let profile = QualityProfile {
            min_clip_ms: 5_000,
            max_clip_ms: 10_000,
            min_speech_ms: 3_000,
            max_window_ms: 6_000,
        };
        let (short, short_ms) =
            parse_wav(&wav(&tone(2_000, 8_000), SAMPLE_RATE_HZ, 1, 16)).unwrap();
        assert_eq!(analyze(&short, short_ms, &profile), Err(Reject::TooShort));

        let (silent, silent_ms) = parse_wav(&wav(&tone(6_000, 0), SAMPLE_RATE_HZ, 1, 16)).unwrap();
        assert_eq!(
            analyze(&silent, silent_ms, &profile),
            Err(Reject::InsufficientAudio)
        );

        let (clipped, clipped_ms) =
            parse_wav(&wav(&tone(6_000, i16::MAX), SAMPLE_RATE_HZ, 1, 16)).unwrap();
        assert_eq!(
            analyze(&clipped, clipped_ms, &profile),
            Err(Reject::Clipped)
        );
    }

    #[test]
    fn window_selection_prefers_the_speechiest_span() {
        let profile_max = 4_000;
        let mut samples = vec![0.0f32; 6_000 * 16];
        // Loud speech only in the last 3 seconds.
        for sample in samples.iter_mut().skip(3_000 * 16) {
            *sample = 0.5;
        }
        let clip = PcmF32Mono::new(samples, SAMPLE_RATE_HZ);
        let window = select_window(&clip, profile_max);
        assert_eq!(window.samples().len(), 4_000 * 16);
        // A 4 s window covering the final 3 s of speech must include all of it.
        let loud = window
            .samples()
            .iter()
            .filter(|sample| sample.abs() > 0.1)
            .count();
        assert!(loud >= 3_000 * 16);
    }

    #[test]
    fn embedding_validation_and_encoding() {
        assert_eq!(validate_embedding(&[0.1, 0.2, 0.3], 3), Ok(()));
        assert_eq!(
            validate_embedding(&[0.1, 0.2], 3),
            Err(EmbeddingReject::DimensionMismatch)
        );
        assert_eq!(
            validate_embedding(&[f32::NAN, 0.2, 0.3], 3),
            Err(EmbeddingReject::NotFinite)
        );
        assert_eq!(
            validate_embedding(&[0.0, 0.0, 0.0], 3),
            Err(EmbeddingReject::Degenerate)
        );
        assert_eq!(encode_embedding(&[1.0, -1.0]).len(), 8);

        let mut vector = vec![3.0f32, 4.0];
        assert!(normalize(&mut vector));
        assert!((vector[0] - 0.6).abs() < 1e-6);
        assert!((vector[1] - 0.8).abs() < 1e-6);
        assert!(!normalize(&mut [0.0f32, 0.0]));
    }
}
