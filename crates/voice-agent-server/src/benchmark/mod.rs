//! Privacy-safe TTS provider benchmark primitives.

use std::{sync::atomic::AtomicBool, time::Instant};

use serde::{Serialize, Serializer};

use crate::{
    audio::{CanonicalDownlinkPipeline, MAX_DOWNLINK_OPUS_PACKET_BYTES, PcmF32Mono},
    providers::{TtsError, TtsWorker},
};

pub const TTS_WORKLOAD_VERSION: &str = "tts-vi-v1";
const TTS_WORKLOAD: &str = "Xin chào, đây là bài kiểm tra hiệu năng tổng hợp giọng nói tiếng Việt.";
#[cfg(test)]
const TTS_WORKLOAD_SHA256: &str =
    "38b03e92c82b42fdb37ba6e3b83564ac8cdaa322860941f2a8604a066d3dc22a";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TtsBenchmarkMode {
    Provider,
    Delivery,
}

impl TtsBenchmarkMode {
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "provider" => Some(Self::Provider),
            "delivery" => Some(Self::Delivery),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BenchmarkErrorCategory {
    Config,
    ModelPreparation,
    ProviderBuild,
    Warmup,
    Synthesis,
    InvalidPcm,
    Resample,
    OpusInit,
    OpusEncode,
    WorkerReset,
    OutputIo,
}

impl Serialize for BenchmarkErrorCategory {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

impl BenchmarkErrorCategory {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Config => "config",
            Self::ModelPreparation => "model_preparation",
            Self::ProviderBuild => "provider_build",
            Self::Warmup => "warmup",
            Self::Synthesis => "synthesis",
            Self::InvalidPcm => "invalid_pcm",
            Self::Resample => "resample",
            Self::OpusInit => "opus_init",
            Self::OpusEncode => "opus_encode",
            Self::WorkerReset => "worker_reset",
            Self::OutputIo => "output_io",
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct TtsRunMetrics {
    pub processing_ms: f64,
    pub pcm_chunks: u64,
    pub provider_samples: u64,
    pub provider_audio_duration_ms: f64,
    pub ttfa_ms: Option<f64>,
    pub first_packet_ms: Option<f64>,
    pub packet_count: Option<u64>,
    pub delivery_audio_duration_ms: Option<u64>,
    pub rtf: f64,
}

#[derive(Clone, Debug, Serialize)]
pub struct MetricSummary {
    pub sample_count: usize,
    pub min: f64,
    pub median: f64,
    pub max: f64,
    pub p95: Option<f64>,
}

#[derive(Clone, Debug, Serialize)]
pub struct TtsBenchmarkResult {
    pub schema_version: u8,
    pub status: &'static str,
    pub workload_version: &'static str,
    pub mode: TtsBenchmarkMode,
    pub runs: usize,
    pub warmup_runs: usize,
    pub samples: Vec<TtsRunMetrics>,
    pub processing_ms: MetricSummary,
    pub rtf: MetricSummary,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ttfa_ms: Option<MetricSummary>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub first_packet_ms: Option<MetricSummary>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub adapter: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_identity: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_preparation_ms: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider_build_and_readiness_ms: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub worker_open_ms: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub comparison_qualified: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub overall_elapsed_ms: Option<f64>,
}

pub fn run_tts_benchmark(
    worker: &mut dyn TtsWorker,
    mode: TtsBenchmarkMode,
    warmup_runs: usize,
    runs: usize,
) -> Result<TtsBenchmarkResult, BenchmarkErrorCategory> {
    for _ in 0..warmup_runs {
        let result = run_once(worker, mode);
        worker
            .reset()
            .map_err(|_| BenchmarkErrorCategory::WorkerReset)?;
        result.map_err(|_| BenchmarkErrorCategory::Warmup)?;
    }
    if runs == 0 {
        return Err(BenchmarkErrorCategory::Config);
    }
    let mut samples = Vec::with_capacity(runs);
    for _ in 0..runs {
        let result = run_once(worker, mode);
        worker
            .reset()
            .map_err(|_| BenchmarkErrorCategory::WorkerReset)?;
        samples.push(result?);
    }
    let processing_ms = summarize(samples.iter().map(|sample| sample.processing_ms));
    let rtf = summarize(samples.iter().map(|sample| sample.rtf));
    let ttfa_ms = (mode == TtsBenchmarkMode::Provider)
        .then(|| summarize(samples.iter().filter_map(|sample| sample.ttfa_ms)));
    let first_packet_ms = (mode == TtsBenchmarkMode::Delivery)
        .then(|| summarize(samples.iter().filter_map(|sample| sample.first_packet_ms)));
    Ok(TtsBenchmarkResult {
        schema_version: 1,
        status: "passed",
        workload_version: TTS_WORKLOAD_VERSION,
        mode,
        runs,
        warmup_runs,
        samples,
        processing_ms,
        rtf,
        ttfa_ms,
        first_packet_ms,
        adapter: None,
        model_identity: None,
        model_preparation_ms: None,
        provider_build_and_readiness_ms: None,
        worker_open_ms: None,
        comparison_qualified: None,
        overall_elapsed_ms: None,
    })
}

fn run_once(
    worker: &mut dyn TtsWorker,
    mode: TtsBenchmarkMode,
) -> Result<TtsRunMetrics, BenchmarkErrorCategory> {
    let started = Instant::now();
    let mut pcm_chunks = 0_u64;
    let mut provider_samples = 0_u64;
    let mut first_pcm_ms = None;
    let mut first_packet_ms = None;
    let mut packets = 0_u64;
    let mut callback_error = None;
    let mut pipeline = (mode == TtsBenchmarkMode::Delivery)
        .then(|| CanonicalDownlinkPipeline::new(MAX_DOWNLINK_OPUS_PACKET_BYTES))
        .transpose()
        .map_err(|_| BenchmarkErrorCategory::OpusInit)?;
    let cancelled = AtomicBool::new(false);
    worker
        .synthesize(TTS_WORKLOAD, &cancelled, &mut |pcm| {
            if validate_pcm(&pcm).is_err() {
                callback_error = Some(BenchmarkErrorCategory::InvalidPcm);
                return Err(TtsError::Failed);
            }
            pcm_chunks += 1;
            provider_samples += pcm.samples().len() as u64;
            first_pcm_ms.get_or_insert_with(|| elapsed_ms(started));
            if let Some(pipeline) = &mut pipeline {
                let ready = match pipeline.push_provider_pcm(pcm) {
                    Ok(ready) => ready,
                    Err(error) => {
                        callback_error = Some(map_audio_error(error));
                        return Err(TtsError::Failed);
                    }
                };
                if !ready.is_empty() {
                    first_packet_ms.get_or_insert_with(|| elapsed_ms(started));
                    packets += ready.len() as u64;
                }
            }
            Ok(())
        })
        .map_err(|_| callback_error.unwrap_or(BenchmarkErrorCategory::Synthesis))?;
    if pcm_chunks == 0 || provider_samples == 0 {
        return Err(BenchmarkErrorCategory::InvalidPcm);
    }
    if let Some(pipeline) = &mut pipeline {
        let ready = pipeline.finish().map_err(map_audio_error)?;
        if !ready.is_empty() {
            first_packet_ms.get_or_insert_with(|| elapsed_ms(started));
            packets += ready.len() as u64;
        }
        if packets == 0 {
            return Err(BenchmarkErrorCategory::OpusEncode);
        }
    }
    let processing_ms = elapsed_ms(started);
    let provider_audio_duration_ms = provider_samples as f64 * 1_000.0 / 48_000.0;
    Ok(TtsRunMetrics {
        processing_ms,
        pcm_chunks,
        provider_samples,
        provider_audio_duration_ms,
        ttfa_ms: (mode == TtsBenchmarkMode::Provider)
            .then(|| first_pcm_ms.expect("non-empty PCM sets first PCM time")),
        first_packet_ms: (mode == TtsBenchmarkMode::Delivery)
            .then(|| first_packet_ms.expect("delivery with packets sets first packet time")),
        packet_count: (mode == TtsBenchmarkMode::Delivery).then_some(packets),
        delivery_audio_duration_ms: (mode == TtsBenchmarkMode::Delivery).then_some(packets * 60),
        rtf: processing_ms / provider_audio_duration_ms,
    })
}

fn validate_pcm(pcm: &PcmF32Mono) -> Result<(), TtsError> {
    if pcm.sample_rate_hz() != 48_000
        || pcm.samples().is_empty()
        || pcm.samples().iter().any(|sample| !sample.is_finite())
    {
        return Err(TtsError::Failed);
    }
    Ok(())
}

fn map_audio_error(error: crate::audio::AudioError) -> BenchmarkErrorCategory {
    match error {
        crate::audio::AudioError::InvalidDownlinkPcm
        | crate::audio::AudioError::InvalidProviderPcm => BenchmarkErrorCategory::Resample,
        crate::audio::AudioError::EncoderInit => BenchmarkErrorCategory::OpusInit,
        _ => BenchmarkErrorCategory::OpusEncode,
    }
}

fn elapsed_ms(started: Instant) -> f64 {
    started.elapsed().as_secs_f64() * 1_000.0
}

fn summarize(values: impl Iterator<Item = f64>) -> MetricSummary {
    let mut values = values.collect::<Vec<_>>();
    values.sort_by(f64::total_cmp);
    let sample_count = values.len();
    MetricSummary {
        sample_count,
        min: values[0],
        median: if sample_count.is_multiple_of(2) {
            (values[sample_count / 2 - 1] + values[sample_count / 2]) / 2.0
        } else {
            values[sample_count / 2]
        },
        max: values[sample_count - 1],
        p95: (sample_count >= 20).then(|| values[(sample_count * 95).div_ceil(100) - 1]),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::AtomicBool;

    use sha2::{Digest, Sha256};

    use super::{TTS_WORKLOAD, TTS_WORKLOAD_SHA256, TtsBenchmarkMode, run_tts_benchmark};
    use crate::{
        audio::PcmF32Mono,
        providers::{TtsError, TtsWorker},
    };

    struct CountingWorker {
        syntheses: usize,
        resets: usize,
    }
    impl TtsWorker for CountingWorker {
        fn synthesize(
            &mut self,
            _: &str,
            _: &AtomicBool,
            on_pcm: &mut dyn FnMut(PcmF32Mono) -> Result<(), TtsError>,
        ) -> Result<(), TtsError> {
            self.syntheses += 1;
            on_pcm(PcmF32Mono::new(vec![0.25; 2_880], 48_000))
        }
        fn reset(&mut self) -> Result<(), TtsError> {
            self.resets += 1;
            Ok(())
        }
    }

    #[test]
    fn warmup_is_excluded_and_delivery_finalizes_the_tail() {
        let mut worker = CountingWorker {
            syntheses: 0,
            resets: 0,
        };
        let result = run_tts_benchmark(&mut worker, TtsBenchmarkMode::Delivery, 1, 2).unwrap();
        assert_eq!(worker.syntheses, 3);
        assert_eq!(worker.resets, 3);
        assert_eq!(result.samples.len(), 2);
        assert!(
            result
                .samples
                .iter()
                .all(|sample| sample.packet_count == Some(1))
        );
        assert!(result.first_packet_ms.is_some());
    }

    #[test]
    fn workload_hash_is_pinned() {
        assert_eq!(
            Sha256::digest(TTS_WORKLOAD.as_bytes())
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<String>(),
            TTS_WORKLOAD_SHA256
        );
    }

    #[test]
    fn invalid_provider_pcm_fails_the_current_mode_without_a_sample() {
        struct InvalidPcmWorker;
        impl TtsWorker for InvalidPcmWorker {
            fn synthesize(
                &mut self,
                _: &str,
                _: &AtomicBool,
                on_pcm: &mut dyn FnMut(PcmF32Mono) -> Result<(), TtsError>,
            ) -> Result<(), TtsError> {
                on_pcm(PcmF32Mono::new(vec![f32::NAN], 48_000))
            }
            fn reset(&mut self) -> Result<(), TtsError> {
                Ok(())
            }
        }

        let mut worker = InvalidPcmWorker;
        assert!(matches!(
            run_tts_benchmark(&mut worker, TtsBenchmarkMode::Provider, 0, 1),
            Err(super::BenchmarkErrorCategory::InvalidPcm)
        ));
    }

    #[test]
    fn report_omits_the_fixed_workload_text_and_p95_for_five_samples() {
        let mut worker = CountingWorker {
            syntheses: 0,
            resets: 0,
        };
        let result = run_tts_benchmark(&mut worker, TtsBenchmarkMode::Provider, 0, 5).unwrap();
        assert_eq!(result.processing_ms.p95, None);
        assert!(
            !serde_json::to_string(&result)
                .unwrap()
                .contains(TTS_WORKLOAD)
        );
    }

    #[test]
    fn error_category_has_one_shared_json_and_stderr_name() {
        assert_eq!(
            serde_json::to_string(&super::BenchmarkErrorCategory::OpusEncode).unwrap(),
            format!("\"{}\"", super::BenchmarkErrorCategory::OpusEncode.as_str())
        );
    }
}
