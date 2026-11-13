//! One process-local System Live Snapshot shared by every Admin observer.

use std::{
    future::Future,
    pin::Pin,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use chrono::{SecondsFormat, Utc};
use serde::Serialize;
use serde_json::Value;
use tokio::sync::{Notify, watch};
use tokio_util::sync::CancellationToken;

use super::metrics::{ResourceSnapshot, SystemMetrics};

const STATUS_INTERVAL: Duration = Duration::from_secs(15);
const RESOURCE_CHECK_INTERVAL: Duration = Duration::from_secs(5);

#[derive(Clone, Serialize)]
pub struct SystemLiveSnapshot {
    pub v: u8,
    pub resources: Option<ResourceSnapshot>,
    pub status: SystemLiveStatus,
}

#[derive(Clone, Serialize)]
pub struct SystemLiveStatus {
    pub checked_at: Option<String>,
    pub refresh_interval_ms: u64,
    pub summary: Option<Value>,
}

type SummaryFuture<'a> = Pin<Box<dyn Future<Output = Option<Value>> + Send + 'a>>;

/// Internal seam: production wraps SystemStatus; tests provide a counted fake.
pub(crate) trait StatusSummarySource: Send + Sync {
    fn summary(&self) -> SummaryFuture<'_>;
}

pub(crate) struct SystemStatusAdapter(pub(crate) crate::services::system_status::SystemStatus);

impl StatusSummarySource for SystemStatusAdapter {
    fn summary(&self) -> SummaryFuture<'_> {
        Box::pin(async move { self.0.summary().await.ok() })
    }
}

pub(crate) struct SystemLive {
    metrics: Arc<SystemMetrics>,
    source: Arc<dyn StatusSummarySource>,
    snapshot: watch::Sender<Arc<SystemLiveSnapshot>>,
    subscriber_changed: Notify,
    started: AtomicBool,
}

impl SystemLive {
    pub(crate) fn new(metrics: Arc<SystemMetrics>, source: Arc<dyn StatusSummarySource>) -> Self {
        let initial = Arc::new(SystemLiveSnapshot {
            v: 1,
            resources: metrics.latest(),
            status: SystemLiveStatus {
                checked_at: None,
                refresh_interval_ms: STATUS_INTERVAL.as_millis() as u64,
                summary: None,
            },
        });
        let (snapshot, _) = watch::channel(initial);
        Self {
            metrics,
            source,
            snapshot,
            subscriber_changed: Notify::new(),
            started: AtomicBool::new(false),
        }
    }

    /// Returns the current snapshot immediately and signals the owner to refresh status.
    pub(crate) fn subscribe(&self) -> watch::Receiver<Arc<SystemLiveSnapshot>> {
        let receiver = self.snapshot.subscribe();
        self.subscriber_changed.notify_one();
        receiver
    }

    /// Bootstrap owns the task; it is started once per AppState.
    pub(crate) fn start(self: Arc<Self>, stop: CancellationToken) -> tokio::task::JoinHandle<()> {
        if self.started.swap(true, Ordering::AcqRel) {
            return tokio::spawn(async {});
        }
        tokio::spawn(async move { self.run(stop).await })
    }

    async fn run(self: Arc<Self>, stop: CancellationToken) {
        let mut resource_samples = self.metrics.subscribe();
        let mut resource_check = tokio::time::interval(RESOURCE_CHECK_INTERVAL);
        resource_check.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        resource_check.tick().await;
        let mut last_status_refresh = None;
        let mut status = SystemLiveStatus {
            checked_at: None,
            refresh_interval_ms: STATUS_INTERVAL.as_millis() as u64,
            summary: None,
        };
        let mut resources = self.metrics.latest();

        loop {
            tokio::select! {
                biased;
                _ = stop.cancelled() => break,
                _ = self.subscriber_changed.notified() => {},
                changed = resource_samples.changed() => {
                    if changed.is_err() {
                        break;
                    }
                    resources = self.metrics.latest();
                    self.publish(resources.clone(), status.clone());
                }
                _ = resource_check.tick() => {
                    let current = self.metrics.latest();
                    if resources.is_some() && current.is_none() {
                        resources = current;
                        self.publish(resources.clone(), status.clone());
                    }
                }
            }

            if self.snapshot.receiver_count() == 0 {
                continue;
            }
            let should_refresh = last_status_refresh
                .is_none_or(|last: tokio::time::Instant| last.elapsed() >= STATUS_INTERVAL);
            if should_refresh {
                last_status_refresh = Some(tokio::time::Instant::now());
                status.checked_at = Some(Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true));
                status.summary = tokio::select! {
                    biased;
                    _ = stop.cancelled() => break,
                    result = self.source.summary() => result,
                };
                self.publish(resources.clone(), status.clone());
            }
        }
    }

    fn publish(&self, resources: Option<ResourceSnapshot>, status: SystemLiveStatus) {
        self.snapshot.send_replace(Arc::new(SystemLiveSnapshot {
            v: 1,
            resources,
            status,
        }));
    }
}

#[cfg(test)]
#[path = "live/tests.rs"]
mod tests;
