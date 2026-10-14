//! One OS-visible snapshot; collection never runs on the HTTP path.
use std::{
    path::Path,
    sync::{Arc, RwLock},
    time::{Duration, Instant},
};

use chrono::{SecondsFormat, Utc};
use serde::Serialize;
use sysinfo::{
    DiskRefreshKind, Disks, MemoryRefreshKind, Pid, ProcessRefreshKind, ProcessesToUpdate, System,
};
use tokio_util::sync::CancellationToken;

const SAMPLE_INTERVAL: Duration = Duration::from_secs(5);
const MAX_AGE: Duration = Duration::from_secs(15);

#[derive(Clone, Serialize)]
pub struct ResourceSnapshot {
    sampled_at: String,
    sample_interval_ms: u64,
    system: SystemResources,
    process: ProcessResources,
    storage: StorageResources,
}

#[derive(Clone, Serialize)]
struct SystemResources {
    cpu_percent: Option<f32>,
    logical_cores: Option<usize>,
    memory_total_bytes: Option<u64>,
    memory_used_bytes: Option<u64>,
}

#[derive(Clone, Serialize)]
struct ProcessResources {
    pid: u32,
    cpu_percent: Option<f32>,
    rss_bytes: Option<u64>,
}

#[derive(Clone, Serialize)]
struct StorageResources {
    total_bytes: Option<u64>,
    available_bytes: Option<u64>,
}

#[derive(Default)]
pub struct SystemMetrics {
    snapshot: RwLock<Option<(Instant, ResourceSnapshot)>>,
}

impl SystemMetrics {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn latest(&self) -> Option<ResourceSnapshot> {
        let guard = self.snapshot.read().ok()?;
        let (sampled, snapshot) = guard.as_ref()?;
        (sampled.elapsed() <= MAX_AGE).then(|| snapshot.clone())
    }

    /// Bootstrap owns this task; constructing or cloning application state never starts it.
    pub fn start(self: Arc<Self>, stop: CancellationToken) -> tokio::task::JoinHandle<()> {
        tokio::spawn(async move {
            let mut collector = None;
            let mut interval = tokio::time::interval(SAMPLE_INTERVAL);
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                tokio::select! {
                    biased;
                    _ = stop.cancelled() => break,
                    _ = interval.tick() => {}
                }
                let sample = tokio::task::spawn_blocking(move || {
                    let mut collector = collector.unwrap_or_else(Collector::new);
                    let snapshot = collector.sample();
                    (collector, snapshot)
                });
                let result = tokio::select! {
                    biased;
                    _ = stop.cancelled() => break,
                    result = sample => result,
                };
                match result {
                    Ok((next, snapshot)) => {
                        collector = Some(next);
                        if let Ok(mut guard) = self.snapshot.write() {
                            *guard = Some((Instant::now(), snapshot));
                        }
                    }
                    Err(_) => {
                        tracing::warn!("system metrics sampler unavailable");
                        break;
                    }
                }
            }
        })
    }
}

struct Collector {
    system: System,
    cpu_baseline: bool,
    process_baseline: bool,
}

impl Collector {
    fn new() -> Self {
        Self {
            system: System::new(),
            cpu_baseline: false,
            process_baseline: false,
        }
    }

    fn sample(&mut self) -> ResourceSnapshot {
        let cpu_available = os_readable("/proc/stat");
        let memory_available = os_readable("/proc/meminfo");
        self.system.refresh_cpu_usage();
        self.system
            .refresh_memory_specifics(MemoryRefreshKind::nothing().with_ram());
        let pid = Pid::from_u32(std::process::id());
        let refreshed = self.system.refresh_processes_specifics(
            ProcessesToUpdate::Some(&[pid]),
            true,
            ProcessRefreshKind::nothing()
                .with_cpu()
                .with_memory()
                .without_tasks(),
        );
        // A failed refresh can retain old capacity in sysinfo, so each disk starts empty.
        let mut disks = Disks::new_with_refreshed_list_specifics(DiskRefreshKind::nothing());
        let storage = disks
            .list_mut()
            .iter_mut()
            .find(|disk| disk.mount_point() == Path::new("/"))
            .and_then(|disk| {
                disk.refresh_specifics(DiskRefreshKind::nothing().with_storage())
                    .then(|| (disk.total_space(), disk.available_space()))
            })
            .filter(|(total, available)| *total > 0 && available <= total);
        self.snapshot(cpu_available, memory_available, refreshed > 0, storage)
    }

    fn snapshot(
        &mut self,
        cpu_available: bool,
        memory_available: bool,
        process_refreshed: bool,
        storage: Option<(u64, u64)>,
    ) -> ResourceSnapshot {
        let cores = self.system.cpus().len();
        let total = self.system.total_memory();
        let process = process_refreshed
            .then(|| self.system.process(Pid::from_u32(std::process::id())))
            .flatten();
        let snapshot = ResourceSnapshot {
            sampled_at: Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true),
            sample_interval_ms: SAMPLE_INTERVAL.as_millis() as u64,
            system: SystemResources {
                cpu_percent: (cpu_available && self.cpu_baseline && cores > 0)
                    .then(|| self.system.global_cpu_usage())
                    .filter(|value| value.is_finite())
                    .map(|value| value.clamp(0.0, 100.0)),
                logical_cores: (cpu_available && cores > 0).then_some(cores),
                memory_total_bytes: (memory_available && total > 0).then_some(total),
                memory_used_bytes: (memory_available && total > 0)
                    .then(|| self.system.used_memory().min(total)),
            },
            process: ProcessResources {
                pid: std::process::id(),
                cpu_percent: process
                    .filter(|_| cpu_available && self.cpu_baseline && self.process_baseline)
                    .map(|p| p.cpu_usage())
                    .filter(|value| value.is_finite() && *value >= 0.0),
                rss_bytes: process.map(|p| p.memory()),
            },
            storage: StorageResources {
                total_bytes: storage.map(|(total, _)| total),
                available_bytes: storage.map(|(_, available)| available),
            },
        };
        self.cpu_baseline = cpu_available && cores > 0;
        self.process_baseline = cpu_available && process.is_some();
        snapshot
    }
}

fn os_readable(path: &str) -> bool {
    #[cfg(target_os = "linux")]
    {
        std::fs::read(path).is_ok_and(|data| !data.is_empty())
    }
    #[cfg(not(target_os = "linux"))]
    {
        // shortcut: sysinfo hides CPU/RAM refresh errors here, add OS probes for non-Linux deployments.
        let _ = path;
        sysinfo::IS_SUPPORTED_SYSTEM
    }
}

#[cfg(test)]
#[path = "metrics/tests.rs"]
mod tests;
