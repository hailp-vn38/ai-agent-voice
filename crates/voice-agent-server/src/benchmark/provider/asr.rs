use std::time::{Duration, Instant};

use crate::{
    audio::PcmF32Mono,
    providers::{AsrEvent, AsrProvider},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AsrFeedMode {
    Burst,
    Realtime,
}

#[derive(Clone, Debug, serde::Serialize)]
pub struct AsrRunMetrics {
    pub session_open_ms: f64,
    pub first_partial_compute_ms: Option<f64>,
    pub first_partial_wall_ms: Option<f64>,
    pub finish_ms: f64,
    pub compute_total_ms: f64,
    pub wall_total_ms: f64,
    pub audio_duration_ms: f64,
    pub partial_count: usize,
    pub final_text_chars: usize,
}

pub fn run_asr_provider(
    provider: &dyn AsrProvider,
    samples: &[f32],
    mode: AsrFeedMode,
) -> Result<AsrRunMetrics, crate::providers::AsrError> {
    if samples.is_empty() || !samples.len().is_multiple_of(960) {
        return Err(crate::providers::AsrError::Failed(
            "ASR workload must contain a non-empty whole number of 960-sample frames".into(),
        ));
    }
    let open = Instant::now();
    let mut session = provider.open()?;
    let session_open_ms = ms(open);
    let wall = Instant::now();
    let mut compute = Duration::ZERO;
    let mut first_compute = None;
    let mut first_wall = None;
    let mut partial_count = 0;
    let (frames, remainder) = samples.as_chunks::<960>();
    debug_assert!(remainder.is_empty());
    for (index, chunk) in frames.iter().enumerate() {
        if mode == AsrFeedMode::Realtime {
            let target = wall + Duration::from_millis(index as u64 * 60);
            if let Some(delay) = target.checked_duration_since(Instant::now()) {
                std::thread::sleep(delay)
            }
        }
        let begin = Instant::now();
        let events = session.push_pcm(&PcmF32Mono::new(chunk.to_vec(), 16_000))?;
        compute += begin.elapsed();
        for event in events {
            if let AsrEvent::Partial(text) = event
                && !text.trim().is_empty()
            {
                partial_count += 1;
                first_compute.get_or_insert(compute.as_secs_f64() * 1000.0);
                first_wall.get_or_insert(ms(wall));
            }
        }
    }
    let begin = Instant::now();
    let final_result = session.finish()?;
    let finish_ms = ms(begin);
    compute += begin.elapsed();
    Ok(AsrRunMetrics {
        session_open_ms,
        first_partial_compute_ms: first_compute,
        first_partial_wall_ms: first_wall,
        finish_ms,
        compute_total_ms: compute.as_secs_f64() * 1000.0,
        wall_total_ms: ms(wall),
        audio_duration_ms: samples.len() as f64 * 1000.0 / 16_000.0,
        partial_count,
        final_text_chars: final_result.text().chars().count(),
    })
}
fn ms(start: Instant) -> f64 {
    start.elapsed().as_secs_f64() * 1000.0
}
