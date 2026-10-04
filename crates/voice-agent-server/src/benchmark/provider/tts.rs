use super::super::{
    error::BenchmarkErrorCategory,
    report::{TtsBenchmarkMode, TtsBenchmarkResult, TtsRunMetrics},
    stats::summarize,
    workload::{TTS_WORKLOAD, TTS_WORKLOAD_VERSION},
};
use crate::{
    audio::{CanonicalDownlinkPipeline, MAX_DOWNLINK_OPUS_PACKET_BYTES, PcmF32Mono},
    providers::{TtsBinding, TtsError, TtsSynthesisRequest, TtsWorker},
};
use std::{sync::atomic::AtomicBool, time::Instant};

pub fn run_tts_benchmark(
    worker: &mut dyn TtsWorker,
    binding: &TtsBinding,
    mode: TtsBenchmarkMode,
    warmup_runs: usize,
    runs: usize,
) -> Result<TtsBenchmarkResult, BenchmarkErrorCategory> {
    for _ in 0..warmup_runs {
        let result = run_once(worker, binding, mode);
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
        let result = run_once(worker, binding, mode);
        worker
            .reset()
            .map_err(|_| BenchmarkErrorCategory::WorkerReset)?;
        samples.push(result?)
    }
    let processing_ms = summarize(samples.iter().map(|s| s.processing_ms));
    let rtf = summarize(samples.iter().map(|s| s.rtf));
    let delivery = mode == TtsBenchmarkMode::Delivery;
    let ttfa_ms = (!delivery).then(|| summarize(samples.iter().filter_map(|s| s.ttfa_ms)));
    let first_packet_ms =
        delivery.then(|| summarize(samples.iter().filter_map(|s| s.first_packet_ms)));
    let synthesis_with_delivery_ms =
        delivery.then(|| summarize(samples.iter().filter_map(|s| s.synthesis_with_delivery_ms)));
    let delivery_total_ms =
        delivery.then(|| summarize(samples.iter().filter_map(|s| s.delivery_total_ms)));
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
        synthesis_with_delivery_ms,
        delivery_total_ms,
        adapter: None,
        model_identity: None,
        model_preparation_ms: None,
        provider_build_and_readiness_ms: None,
        worker_open_ms: None,
        overall_elapsed_ms: None,
    })
}
fn run_once(
    worker: &mut dyn TtsWorker,
    binding: &TtsBinding,
    mode: TtsBenchmarkMode,
) -> Result<TtsRunMetrics, BenchmarkErrorCategory> {
    let started = Instant::now();
    let mut chunks = 0;
    let mut samples = 0;
    let mut sample_rate = None;
    let mut first_pcm = None;
    let mut first_packet = None;
    let mut packets = 0;
    let mut bytes = 0;
    let mut callback_error = None;
    let mut pipeline = (mode == TtsBenchmarkMode::Delivery)
        .then(|| CanonicalDownlinkPipeline::new(MAX_DOWNLINK_OPUS_PACKET_BYTES))
        .transpose()
        .map_err(|_| BenchmarkErrorCategory::OpusInit)?;
    worker
        .synthesize(
            &TtsSynthesisRequest {
                text: TTS_WORKLOAD.into(),
                selection: binding.clone(),
            },
            &AtomicBool::new(false),
            &mut |pcm| {
                if invalid(&pcm) || sample_rate.is_some_and(|rate| rate != pcm.sample_rate_hz()) {
                    callback_error = Some(BenchmarkErrorCategory::InvalidPcm);
                    return Err(TtsError::Failed);
                }
                sample_rate = Some(pcm.sample_rate_hz());
                chunks += 1;
                samples += pcm.samples().len() as u64;
                first_pcm.get_or_insert_with(|| ms(started));
                if let Some(p) = &mut pipeline {
                    let ready = p.push_provider_pcm(pcm).map_err(|_| TtsError::Failed)?;
                    if !ready.is_empty() {
                        first_packet.get_or_insert_with(|| ms(started));
                        packets += ready.len() as u64;
                        bytes += ready.iter().map(|p| p.as_bytes().len() as u64).sum::<u64>();
                    }
                }
                Ok(())
            },
        )
        .map_err(|_| callback_error.unwrap_or(BenchmarkErrorCategory::Synthesis))?;
    let synthesis = ms(started);
    if chunks == 0 || samples == 0 {
        return Err(BenchmarkErrorCategory::InvalidPcm);
    }
    if let Some(p) = &mut pipeline {
        let ready = p.finish().map_err(|_| BenchmarkErrorCategory::OpusEncode)?;
        if !ready.is_empty() {
            first_packet.get_or_insert_with(|| ms(started));
            packets += ready.len() as u64;
            bytes += ready.iter().map(|p| p.as_bytes().len() as u64).sum::<u64>();
        }
        if packets == 0 {
            return Err(BenchmarkErrorCategory::OpusEncode);
        }
    }
    let total = ms(started);
    let duration =
        samples as f64 * 1000.0 / f64::from(sample_rate.expect("nonempty PCM validated"));
    Ok(TtsRunMetrics {
        processing_ms: total,
        pcm_chunks: chunks,
        provider_samples: samples,
        provider_audio_duration_ms: duration,
        ttfa_ms: (mode == TtsBenchmarkMode::Provider).then(|| first_pcm.unwrap()),
        first_packet_ms: (mode == TtsBenchmarkMode::Delivery).then(|| first_packet.unwrap()),
        synthesis_with_delivery_ms: (mode == TtsBenchmarkMode::Delivery).then_some(synthesis),
        delivery_total_ms: (mode == TtsBenchmarkMode::Delivery).then_some(total),
        packet_count: (mode == TtsBenchmarkMode::Delivery).then_some(packets),
        opus_bytes: (mode == TtsBenchmarkMode::Delivery).then_some(bytes),
        delivery_audio_duration_ms: (mode == TtsBenchmarkMode::Delivery).then_some(packets * 60),
        rtf: total / duration,
    })
}
fn invalid(pcm: &PcmF32Mono) -> bool {
    !matches!(pcm.sample_rate_hz(), 24_000 | 48_000)
        || pcm.samples().is_empty()
        || pcm.samples().iter().any(|s| !s.is_finite())
}
fn ms(started: Instant) -> f64 {
    started.elapsed().as_secs_f64() * 1000.0
}
