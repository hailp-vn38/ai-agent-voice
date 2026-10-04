//! Fixed-cardinality aggregate metrics. No provider IDs, revisions or resource hashes are labels.
use serde::Serialize;
use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::Instant,
};
#[derive(Clone, Copy)]
pub enum RuntimePhase {
    Snapshot,
    Queue,
    Build,
    Acquisition,
    WsAdmission,
    SwitchPrepare,
    SwitchCommit,
    Unload,
    WorkerInit,
    Warmup,
    /// Ensuring the provider's declared assets, including any missing download or transform.
    ArtifactPrepare,
    /// Resolving the provider's declared asset paths on the materialization path.
    ArtifactVerify,
    /// Adapter-owned native construction, e.g. reading the pinned ZeroTTS contract.
    ProviderContract,
    /// Wall time of one complete native materialization, from preparation through retention.
    RuntimeTotal,
}
#[derive(Clone, Copy)]
pub enum RuntimeCounter {
    Hit,
    Miss,
    Coalesced,
    BuildSuccess,
    BuildFailure,
    Eviction,
    ObsoleteIntent,
    DroppedIntent,
    Reload,
    SwitchFailure,
}
const BOUNDS: [u64; 15] = [
    1, 5, 10, 25, 50, 100, 250, 500, 1000, 2500, 5000, 10000, 30000, 60000, 120000,
];
#[derive(Default, Serialize, Clone)]
struct DurationSummary {
    count: u64,
    sum_ms: u64,
    buckets: [u64; 16],
}
pub struct RuntimeMetrics {
    counters: [AtomicU64; 10],
    durations: [Mutex<DurationSummary>; 14],
}
impl Default for RuntimeMetrics {
    fn default() -> Self {
        Self {
            counters: std::array::from_fn(|_| AtomicU64::new(0)),
            durations: std::array::from_fn(|_| Mutex::new(DurationSummary::default())),
        }
    }
}
impl RuntimeMetrics {
    pub fn increment(&self, counter: RuntimeCounter) {
        self.counters[counter as usize].fetch_add(1, Ordering::Relaxed);
    }
    pub fn observe(&self, phase: RuntimePhase, elapsed: std::time::Duration) {
        let ms = u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX);
        let bucket = BOUNDS.iter().position(|bound| ms <= *bound).unwrap_or(15);
        let mut summary = self.durations[phase as usize]
            .lock()
            .expect("runtime metrics poisoned");
        summary.count = summary.count.saturating_add(1);
        summary.sum_ms = summary.sum_ms.saturating_add(ms);
        summary.buckets[bucket] = summary.buckets[bucket].saturating_add(1);
    }
    pub fn timer(self: &Arc<Self>, phase: RuntimePhase) -> RuntimeTimer {
        RuntimeTimer {
            metrics: self.clone(),
            phase,
            started: Instant::now(),
        }
    }
    pub fn snapshot(&self) -> serde_json::Value {
        let names = [
            "hit",
            "miss",
            "coalesced",
            "build_success",
            "build_failure",
            "eviction",
            "obsolete_intent",
            "dropped_intent",
            "reload",
            "switch_failure",
        ];
        let phases = [
            "snapshot",
            "queue",
            "build",
            "acquisition",
            "ws_admission",
            "switch_prepare",
            "switch_commit",
            "unload",
            "worker_init",
            "warmup",
            "artifact_prepare",
            "artifact_verify",
            "provider_contract",
            "runtime_total",
        ];
        let counters: serde_json::Map<_, _> = names
            .into_iter()
            .zip(&self.counters)
            .map(|(name, count)| {
                (
                    name.into(),
                    serde_json::json!(count.load(Ordering::Relaxed)),
                )
            })
            .collect();
        let durations: serde_json::Map<_, _> = phases
            .into_iter()
            .zip(&self.durations)
            .map(|(name, summary)| {
                (
                    name.into(),
                    serde_json::to_value(summary.lock().expect("runtime metrics poisoned").clone())
                        .expect("safe metric serialization"),
                )
            })
            .collect();
        serde_json::json!({"counters":counters,"duration_ms":{"upper_bounds":BOUNDS,"phases":durations}})
    }
}
pub struct RuntimeTimer {
    metrics: Arc<RuntimeMetrics>,
    phase: RuntimePhase,
    started: Instant,
}
impl Drop for RuntimeTimer {
    fn drop(&mut self) {
        self.metrics.observe(self.phase, self.started.elapsed());
    }
}
