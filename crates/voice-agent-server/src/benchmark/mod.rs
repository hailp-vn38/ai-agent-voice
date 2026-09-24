//! Provider benchmark contracts and target-specific runners.
mod error;
mod provider;
mod report;
mod stats;
mod workload;
pub use error::BenchmarkErrorCategory;
pub use provider::tts::run_tts_benchmark;
pub use report::{MetricSummary, TtsBenchmarkMode, TtsBenchmarkResult, TtsRunMetrics};
pub use workload::TTS_WORKLOAD_VERSION;

#[cfg(test)]
mod tests;
