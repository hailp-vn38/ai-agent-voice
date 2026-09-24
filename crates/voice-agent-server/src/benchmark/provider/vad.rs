use crate::providers::{VadInput, VadProvider};
use std::time::Instant;

#[derive(Clone, Debug, serde::Serialize)]
pub struct VadRunMetrics {
    pub session_open_ms: f64,
    pub frame_latencies_us: Vec<f64>,
    pub frames_per_second: f64,
    pub compute_rtf: f64,
}
pub fn run_vad_provider(
    provider: &dyn VadProvider,
    samples: &[f32],
) -> Result<VadRunMetrics, crate::providers::VadError> {
    if samples.is_empty() || !samples.len().is_multiple_of(512) {
        return Err(crate::providers::VadError::Failed(
            "VAD workload must contain a non-empty whole number of 512-sample frames".into(),
        ));
    }
    let opened = Instant::now();
    let mut session = provider.open()?;
    let session_open_ms = opened.elapsed().as_secs_f64() * 1_000.0;
    let mut latencies = Vec::with_capacity(samples.len() / 512);
    let (frames, remainder) = samples.as_chunks::<512>();
    debug_assert!(remainder.is_empty());
    for (index, chunk) in frames.iter().enumerate() {
        let started = Instant::now();
        let result = session.push(VadInput {
            pcm: chunk.to_vec(),
            start_sample: (index * 512) as u64,
        })?;
        if !result.probability.is_finite()
            || !(0.0..=1.0).contains(&result.probability)
            || result.start_sample != (index * 512) as u64
            || result.end_sample != result.start_sample + 512
        {
            return Err(crate::providers::VadError::Failed(
                "VAD provider returned an invalid probability timeline".into(),
            ));
        }
        latencies.push(started.elapsed().as_secs_f64() * 1_000_000.0)
    }
    session.close()?;
    let mean = latencies.iter().sum::<f64>() / latencies.len() as f64;
    Ok(VadRunMetrics {
        session_open_ms,
        frames_per_second: 1_000_000.0 / mean,
        compute_rtf: mean / 32_000.0,
        frame_latencies_us: latencies,
    })
}
