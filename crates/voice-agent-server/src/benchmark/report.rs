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
