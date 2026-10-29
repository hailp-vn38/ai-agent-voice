//! WAV decoding, anti-aliased 16 kHz mono resampling, low-energy 30s audio segmentation,
//! and local/leased `AsrWorkerRuntime` execution for `CatalogMediaBackend`.

use crate::audio::PcmF32Mono;

use super::media_backend::ServerMediaRuntimeBridge;

/// Maximum WAV duration accepted for background `asr_transcribe` (10 minutes).
pub const MAX_WAV_DURATION_SECS: u64 = 600;

/// Target canonical sample rate required by `AsrWorkerRuntime` (16 kHz).
pub const TARGET_ASR_SAMPLE_RATE_HZ: u32 = 16_000;

/// Maximum samples per ASR segment (30 seconds @ 16 kHz = 480,000 samples),
/// keeping offline Gipformer and streaming Zipformer within bounded utterance memory.
pub const MAX_ASR_SEGMENT_SAMPLES: usize = 30 * (TARGET_ASR_SAMPLE_RATE_HZ as usize);

/// Decodes a canonical RIFF/WAVE payload (`8/16/24/32-bit PCM` or `32-bit IEEE float`,
/// `1..=8` channels, 8 kHz – 192 kHz) into canonical 16 kHz mono `f32` samples.
pub(crate) fn decode_wav_to_16khz_mono(bytes: &[u8]) -> Result<PcmF32Mono, String> {
    let mut reader = hound::WavReader::new(std::io::Cursor::new(bytes))
        .map_err(|e| format!("invalid WAV audio: {e}"))?;
    let spec = reader.spec();
    if !(1..=8).contains(&spec.channels) {
        return Err(format!(
            "unsupported WAV channel count {} (expected 1..=8 channels)",
            spec.channels
        ));
    }
    if !(8_000..=192_000).contains(&spec.sample_rate) {
        return Err(format!(
            "unsupported WAV sample rate {} Hz (expected 8000..=192000 Hz)",
            spec.sample_rate
        ));
    }

    let total_frames = u64::from(reader.duration());
    if total_frames == 0 {
        return Err("WAV audio contains 0 sample frames".to_owned());
    }
    let duration_secs = total_frames / u64::from(spec.sample_rate);
    if duration_secs > MAX_WAV_DURATION_SECS {
        return Err(format!(
            "WAV duration ({duration_secs}s) exceeds maximum allowed limit ({MAX_WAV_DURATION_SECS}s)"
        ));
    }

    let raw_f32: Vec<f32> = match spec.sample_format {
        hound::SampleFormat::Int => match spec.bits_per_sample {
            8 => reader
                .samples::<i8>()
                .map(|s| s.map(|v| (f32::from(v) / 128.0).clamp(-1.0, 1.0)))
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| format!("failed to decode 8-bit PCM samples: {e}"))?,
            16 => reader
                .samples::<i16>()
                .map(|s| s.map(|v| (f32::from(v) / 32768.0).clamp(-1.0, 1.0)))
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| format!("failed to decode 16-bit PCM samples: {e}"))?,
            24 => {
                let scale = 8_388_608.0_f32;
                reader
                    .samples::<i32>()
                    .map(|s| s.map(|v| ((v as f32) / scale).clamp(-1.0, 1.0)))
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(|e| format!("failed to decode 24-bit PCM samples: {e}"))?
            }
            32 => {
                let scale = 2_147_483_648.0_f64;
                reader
                    .samples::<i32>()
                    .map(|s| s.map(|v| (((v as f64) / scale) as f32).clamp(-1.0, 1.0)))
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(|e| format!("failed to decode 32-bit PCM samples: {e}"))?
            }
            other => {
                return Err(format!(
                    "unsupported integer WAV bits_per_sample={other} (expected 8, 16, 24, or 32)"
                ));
            }
        },
        hound::SampleFormat::Float => {
            if spec.bits_per_sample != 32 {
                return Err(format!(
                    "unsupported float WAV bits_per_sample={} (expected 32)",
                    spec.bits_per_sample
                ));
            }
            reader
                .samples::<f32>()
                .map(|s| {
                    s.map(|v| {
                        if v.is_finite() {
                            v.clamp(-1.0, 1.0)
                        } else {
                            0.0
                        }
                    })
                })
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| format!("failed to decode 32-bit float WAV samples: {e}"))?
        }
    };

    if raw_f32.is_empty() {
        return Err("decoded WAV contains no audio samples".to_owned());
    }

    let ch = usize::from(spec.channels);
    let mono: Vec<f32> = if ch == 1 {
        raw_f32
    } else {
        let inv_ch = 1.0 / (ch as f32);
        raw_f32
            .chunks_exact(ch)
            .map(|frame| (frame.iter().copied().sum::<f32>() * inv_ch).clamp(-1.0, 1.0))
            .collect()
    };

    if mono.is_empty() {
        return Err("decoded mono WAV contains no frames".to_owned());
    }

    let resampled = if spec.sample_rate == TARGET_ASR_SAMPLE_RATE_HZ {
        mono
    } else {
        resample_mono_to_16khz(&mono, spec.sample_rate)
    };

    Ok(PcmF32Mono::from_samples(resampled))
}

/// Resamples a mono `f32` buffer from `src_rate` to `16_000` Hz.
/// When downsampling (`src_rate > 16_000`), applies a windowed-sinc FIR low-pass filter
/// before linear interpolation to prevent high-frequency aliasing into the 0–8 kHz speech band.
fn resample_mono_to_16khz(samples: &[f32], src_rate: u32) -> Vec<f32> {
    if samples.is_empty() || src_rate == TARGET_ASR_SAMPLE_RATE_HZ {
        return samples.to_vec();
    }

    let filtered_owned: Option<Vec<f32>> = if src_rate > TARGET_ASR_SAMPLE_RATE_HZ {
        Some(lowpass_fir_hamming(
            samples,
            src_rate,
            TARGET_ASR_SAMPLE_RATE_HZ,
        ))
    } else {
        None
    };
    let input = filtered_owned.as_deref().unwrap_or(samples);

    let ratio = f64::from(src_rate) / f64::from(TARGET_ASR_SAMPLE_RATE_HZ);
    let out_len = ((input.len() as f64) / ratio).round().max(1.0) as usize;
    let mut out = Vec::with_capacity(out_len);
    let last_idx = input.len() - 1;

    for i in 0..out_len {
        let src_pos = (i as f64) * ratio;
        let idx0 = (src_pos.floor() as usize).min(last_idx);
        let idx1 = (idx0 + 1).min(last_idx);
        let frac = (src_pos - (idx0 as f64)) as f32;
        let sample = input[idx0] + (input[idx1] - input[idx0]) * frac;
        out.push(sample.clamp(-1.0, 1.0));
    }

    out
}

fn lowpass_fir_hamming(samples: &[f32], src_rate: u32, dst_rate: u32) -> Vec<f32> {
    // Normalized cutoff frequency (cycles per source sample), slightly below target Nyquist (0.45 * dst/src)
    let fc = (0.45 * f64::from(dst_rate) / f64::from(src_rate)).clamp(0.01, 0.49);
    let decimation = (f64::from(src_rate) / f64::from(dst_rate)).ceil() as usize;
    let half_taps = (decimation * 6).clamp(8, 64);
    let num_taps = 2 * half_taps + 1;
    let mut kernel = Vec::with_capacity(num_taps);
    let mut sum = 0.0_f64;

    for n in 0..num_taps {
        let k = (n as isize) - (half_taps as isize);
        let sinc = if k == 0 {
            2.0 * fc
        } else {
            let x = std::f64::consts::PI * (k as f64);
            (2.0 * fc * x).sin() / x
        };
        let window =
            0.54 - 0.46 * (2.0 * std::f64::consts::PI * (n as f64) / ((num_taps - 1) as f64)).cos();
        let coeff = sinc * window;
        kernel.push(coeff as f32);
        sum += coeff;
    }

    if sum.abs() > 1e-9 {
        let inv = (1.0 / sum) as f32;
        for c in &mut kernel {
            *c *= inv;
        }
    }

    let len = samples.len();
    let mut out = Vec::with_capacity(len);
    for i in 0..len {
        let mut acc = 0.0_f32;
        for (tap_idx, &coeff) in kernel.iter().enumerate() {
            let src_idx = (i as isize) + (tap_idx as isize) - (half_taps as isize);
            let clamped_idx = src_idx.clamp(0, (len - 1) as isize) as usize;
            acc += samples[clamped_idx] * coeff;
        }
        out.push(acc.clamp(-1.0, 1.0));
    }
    out
}

#[derive(Debug, Clone)]
pub(crate) struct AsrPcmSegmentSpan {
    pub start_sample: usize,
    pub end_sample: usize,
    pub pcm: PcmF32Mono,
}

impl AsrPcmSegmentSpan {
    pub fn start_offset_ms(&self) -> u64 {
        ((self.start_sample as u64) * 1_000) / 16_000
    }

    pub fn end_offset_ms(&self) -> u64 {
        ((self.end_sample as u64) * 1_000) / 16_000
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct MediaAsrTranscript {
    pub text: String,
    pub segments: Vec<crate::providers::AsrTimedSegment>,
}

/// Splits a 16 kHz mono buffer into spans of at most [`MAX_ASR_SEGMENT_SAMPLES`] (30 seconds),
/// preserving each chunk's `[start_sample, end_sample)` offset from the start of the file.
pub(crate) fn split_pcm_into_asr_segment_spans(pcm: &PcmF32Mono) -> Vec<AsrPcmSegmentSpan> {
    let all = pcm.samples();
    if all.len() <= MAX_ASR_SEGMENT_SAMPLES {
        return vec![AsrPcmSegmentSpan {
            start_sample: 0,
            end_sample: all.len(),
            pcm: pcm.clone(),
        }];
    }

    const WINDOW_SAMPLES: usize = 320; // 20 ms @ 16 kHz
    const HALF_SPAN_SAMPLES: usize = TARGET_ASR_SAMPLE_RATE_HZ as usize; // ±1 s (total 2s span)

    let mut spans = Vec::new();
    let mut offset = 0;
    while offset < all.len() {
        let remaining = all.len() - offset;
        if remaining <= MAX_ASR_SEGMENT_SAMPLES {
            spans.push(AsrPcmSegmentSpan {
                start_sample: offset,
                end_sample: all.len(),
                pcm: PcmF32Mono::from_samples(all[offset..].to_vec()),
            });
            break;
        }

        let max_end = offset + MAX_ASR_SEGMENT_SAMPLES;
        let center = max_end.saturating_sub(HALF_SPAN_SAMPLES);
        let search_start = center
            .saturating_sub(HALF_SPAN_SAMPLES)
            .max(offset + WINDOW_SAMPLES);
        let search_end = (center + HALF_SPAN_SAMPLES).min(max_end);
        let mut best_cut = max_end;
        let mut best_energy = f32::MAX;

        let mut win_start = search_start;
        while win_start + WINDOW_SAMPLES <= search_end {
            let win = &all[win_start..win_start + WINDOW_SAMPLES];
            let energy: f32 = win.iter().map(|s| s * s).sum();
            if energy < best_energy {
                best_energy = energy;
                best_cut = win_start + (WINDOW_SAMPLES / 2);
            }
            win_start += WINDOW_SAMPLES;
        }

        let cut = best_cut.max(offset + 1).min(max_end);
        spans.push(AsrPcmSegmentSpan {
            start_sample: offset,
            end_sample: cut,
            pcm: PcmF32Mono::from_samples(all[offset..cut].to_vec()),
        });
        offset = cut;
    }

    spans
}

/// Splits a 16 kHz mono buffer into segments of at most [`MAX_ASR_SEGMENT_SAMPLES`] (30 seconds).
#[cfg(test)]
pub(crate) fn split_pcm_into_asr_segments(pcm: &PcmF32Mono) -> Vec<PcmF32Mono> {
    split_pcm_into_asr_segment_spans(pcm)
        .into_iter()
        .map(|span| span.pcm)
        .collect()
}

impl ServerMediaRuntimeBridge {
    pub(crate) fn insert_asr_runtime(
        &mut self,
        provider_ref: String,
        runtime: std::sync::Arc<crate::workers::AsrWorkerRuntime>,
    ) {
        std::sync::Arc::make_mut(self.runtimes_mut())
            .asr
            .insert(provider_ref, runtime);
    }

    pub(crate) async fn transcribe_asr(
        &self,
        provider_ref: &str,
        pcm: PcmF32Mono,
    ) -> Result<MediaAsrTranscript, String> {
        let spans = split_pcm_into_asr_segment_spans(&pcm);
        let bare_key = provider_ref
            .strip_prefix("config:asr:")
            .unwrap_or(provider_ref);

        // 1. Resident ASR runtime in `self.runtimes.asr`
        if let Some(runtime) = self
            .runtimes()
            .asr
            .get(provider_ref)
            .or_else(|| self.runtimes().asr.get(bare_key))
            && runtime.adapter() != "unavailable"
        {
            return Self::transcribe_segments_with_runtime(runtime, spans).await;
        }

        // 2. Managed runtime lease via `ProviderRuntimeManager` (ADR-0071)
        let (desired, lease) = self
            .acquire_managed_lease("asr", "ASR", provider_ref)
            .await?;
        let runtime = lease
            .runtimes()
            .and_then(|cat| cat.asr.get(&desired.key).cloned())
            .ok_or_else(|| format!("ASR runtime `{provider_ref}` is unavailable in lease"))?;
        Self::transcribe_segments_with_runtime(&runtime, spans).await
    }

    pub(crate) async fn transcribe_segments_with_runtime(
        runtime: &std::sync::Arc<crate::workers::AsrWorkerRuntime>,
        spans: Vec<AsrPcmSegmentSpan>,
    ) -> Result<MediaAsrTranscript, String> {
        let mut parts = Vec::with_capacity(spans.len());
        let mut segments = Vec::new();
        for span in spans {
            let start_offset_ms = span.start_offset_ms();
            let end_offset_ms = span.end_offset_ms().max(start_offset_ms + 1);
            let result = runtime
                .transcribe_background_pcm_detailed(span.pcm)
                .await
                .map_err(|e| e.to_string())?;
            let (raw_text, raw_segments) = result.into_parts();
            let trimmed = raw_text.trim();
            if !trimmed.is_empty() {
                parts.push(trimmed.to_owned());
            }
            if !raw_segments.is_empty() {
                for seg in raw_segments {
                    let seg_text = seg.text.trim();
                    if seg_text.is_empty() {
                        continue;
                    }
                    let s_ms =
                        (start_offset_ms + seg.start_ms).min(end_offset_ms.saturating_sub(1));
                    let e_ms = (start_offset_ms + seg.end_ms)
                        .max(s_ms + 1)
                        .min(end_offset_ms);
                    segments.push(crate::providers::AsrTimedSegment {
                        start_ms: s_ms,
                        end_ms: e_ms,
                        text: seg_text.to_owned(),
                    });
                }
            } else if !trimmed.is_empty() {
                segments.push(crate::providers::AsrTimedSegment {
                    start_ms: start_offset_ms,
                    end_ms: end_offset_ms,
                    text: trimmed.to_owned(),
                });
            }
        }
        Ok(MediaAsrTranscript {
            text: parts.join(" "),
            segments,
        })
    }
}
