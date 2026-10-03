use serde::Serialize;
#[derive(Clone, Debug, Serialize)]
pub struct MetricSummary {
    pub sample_count: usize,
    pub min: f64,
    pub mean: f64,
    pub p50: f64,
    pub median: f64,
    pub max: f64,
    pub p95: f64,
    pub p99: f64,
}
pub fn summarize(values: impl Iterator<Item = f64>) -> MetricSummary {
    let mut v = values.collect::<Vec<_>>();
    v.sort_by(f64::total_cmp);
    let n = v.len();
    MetricSummary {
        sample_count: n,
        min: v[0],
        mean: v.iter().sum::<f64>() / n as f64,
        p50: v[(n * 50).div_ceil(100) - 1],
        median: if n.is_multiple_of(2) {
            (v[n / 2 - 1] + v[n / 2]) / 2.0
        } else {
            v[n / 2]
        },
        max: v[n - 1],
        p95: v[(n * 95).div_ceil(100) - 1],
        p99: v[(n * 99).div_ceil(100) - 1],
    }
}
