//! Provider benchmark contracts and target-specific runners.
mod error;
mod provider;
mod report;
mod stats;
mod workload;
pub use error::BenchmarkErrorCategory;
pub use provider::asr::{AsrFeedMode, AsrRunMetrics, run_asr_provider};
pub use provider::tts::run_tts_benchmark;
pub use provider::vad::{VadRunMetrics, run_vad_provider};
pub use report::{
    AsrBenchmarkResult, MetricSummary, TtsBenchmarkMode, TtsBenchmarkResult, TtsRunMetrics,
    VadBenchmarkResult,
};
pub use stats::summarize;
pub use workload::TTS_WORKLOAD_VERSION;

#[cfg(test)]
mod tests;
