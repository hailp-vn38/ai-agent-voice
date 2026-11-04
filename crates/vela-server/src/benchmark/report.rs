pub use super::stats::MetricSummary;
use serde::Serialize;
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
#[derive(Clone, Debug, Serialize)]
pub struct TtsRunMetrics {
    pub processing_ms: f64,
    pub pcm_chunks: u64,
    pub provider_samples: u64,
    pub provider_audio_duration_ms: f64,
    pub ttfa_ms: Option<f64>,
    pub first_packet_ms: Option<f64>,
    pub synthesis_with_delivery_ms: Option<f64>,
    pub delivery_total_ms: Option<f64>,
    pub packet_count: Option<u64>,
    pub opus_bytes: Option<u64>,
    pub delivery_audio_duration_ms: Option<u64>,
    pub rtf: f64,
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
    pub synthesis_with_delivery_ms: Option<MetricSummary>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub delivery_total_ms: Option<MetricSummary>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub adapter: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_identity: Option<String>,
    /// Wall time to install the provider's model files before the runtime was built.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_preparation_ms: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider_build_and_readiness_ms: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub worker_open_ms: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub overall_elapsed_ms: Option<f64>,
}

#[derive(Clone, Debug, Serialize)]
pub struct AsrBenchmarkResult {
    pub schema_version: u8,
    pub status: &'static str,
    pub workload_version: String,
    pub feed: super::AsrFeedMode,
    pub warmup_runs: usize,
    pub iterations: usize,
    pub samples: Vec<super::AsrRunMetrics>,
    pub session_open_ms: MetricSummary,
    pub first_partial_compute_ms: Option<MetricSummary>,
    pub first_partial_wall_ms: Option<MetricSummary>,
    pub finish_ms: MetricSummary,
    pub compute_total_ms: MetricSummary,
    pub wall_total_ms: MetricSummary,
    pub compute_rtf: MetricSummary,
}

#[derive(Clone, Debug, Serialize)]
pub struct VadBenchmarkResult {
    pub schema_version: u8,
    pub status: &'static str,
    pub workload_version: String,
    pub warmup_runs: usize,
    pub iterations: usize,
    pub samples: Vec<super::VadRunMetrics>,
    pub session_open_ms: MetricSummary,
    pub frame_latency_us: MetricSummary,
    pub frames_per_second: MetricSummary,
    pub compute_rtf: MetricSummary,
}

#[derive(Clone, Debug, Serialize)]
pub struct LlmBenchmarkResult {
    pub schema_version: u8,
    pub status: &'static str,
    pub workload_version: String,
    pub warmup_runs: usize,
    pub iterations: usize,
    pub samples: Vec<super::LlmRunMetrics>,
    pub ttft_ms: Option<MetricSummary>,
    pub total_ms: MetricSummary,
    pub text_delta_count: MetricSummary,
    pub output_chars: MetricSummary,
    pub tool_call_count: MetricSummary,
}
