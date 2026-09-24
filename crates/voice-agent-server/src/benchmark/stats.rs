use serde::Serialize;
#[derive(Clone, Debug, Serialize)]
pub struct MetricSummary {
    pub sample_count: usize,
    pub min: f64,
    pub median: f64,
    pub max: f64,
    pub p95: Option<f64>,
}
pub fn summarize(values: impl Iterator<Item = f64>) -> MetricSummary {
    let mut v = values.collect::<Vec<_>>();
    v.sort_by(f64::total_cmp);
    let n = v.len();
    MetricSummary {
        sample_count: n,
        min: v[0],
        median: if n.is_multiple_of(2) {
            (v[n / 2 - 1] + v[n / 2]) / 2.0
        } else {
            v[n / 2]
        },
        max: v[n - 1],
        p95: (n >= 20).then(|| v[(n * 95).div_ceil(100) - 1]),
    }
}
