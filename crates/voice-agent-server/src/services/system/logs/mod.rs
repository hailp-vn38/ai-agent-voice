mod layer;
mod sanitize;
mod types;

#[cfg(test)]
mod tests;

pub use layer::LogHubLayer;
pub use types::{
    CorrelationContext, LogFilter, LogLevel, LogLevelSummary, RawLogRecord, StructuredLogEntry,
    SystemLogPage,
};

use sanitize::{ParsedFilter, is_self_observation_noise, normalize_entry};
use std::{
    collections::VecDeque,
    sync::{
        Arc, OnceLock, RwLock,
        atomic::{AtomicU64, Ordering},
    },
};
use tokio::sync::broadcast::{self, error::RecvError};
use tracing_subscriber::{EnvFilter, layer::SubscriberExt, util::SubscriberInitExt};

pub const DEFAULT_RING_BUFFER_CAPACITY: usize = 2_000;
pub const DEFAULT_BROADCAST_CAPACITY: usize = 512;

struct RingState {
    entries: VecDeque<StructuredLogEntry>,
    dropped_evicted: u64,
    counts: LogLevelSummary,
}

impl RingState {
    fn increment_count(&mut self, level: LogLevel) {
        match level {
            LogLevel::Error => self.counts.error = self.counts.error.saturating_add(1),
            LogLevel::Warn => self.counts.warn = self.counts.warn.saturating_add(1),
            LogLevel::Info => self.counts.info = self.counts.info.saturating_add(1),
            LogLevel::Debug => self.counts.debug = self.counts.debug.saturating_add(1),
            LogLevel::Trace => self.counts.trace = self.counts.trace.saturating_add(1),
        }
    }

    fn decrement_count(&mut self, level: LogLevel) {
        match level {
            LogLevel::Error => self.counts.error = self.counts.error.saturating_sub(1),
            LogLevel::Warn => self.counts.warn = self.counts.warn.saturating_sub(1),
            LogLevel::Info => self.counts.info = self.counts.info.saturating_sub(1),
            LogLevel::Debug => self.counts.debug = self.counts.debug.saturating_sub(1),
            LogLevel::Trace => self.counts.trace = self.counts.trace.saturating_sub(1),
        }
    }
}

/// Events emitted by [`FilteredLogSubscription::recv`].
#[derive(Debug, Clone, PartialEq)]
pub enum LogStreamItem {
    Entry(Arc<StructuredLogEntry>),
    Lagged { missed: u64, latest_seq: u64 },
}

/// A live log subscription that evaluates its [`LogFilter`] server-side before yielding entries.
pub struct FilteredLogSubscription {
    rx: broadcast::Receiver<Arc<StructuredLogEntry>>,
    filter: ParsedFilter,
    hub: Arc<SystemLogHub>,
    pub latest_seq_at_subscribe: u64,
    pub capacity: usize,
}

impl FilteredLogSubscription {
    pub async fn recv(&mut self) -> Option<LogStreamItem> {
        loop {
            match self.rx.recv().await {
                Ok(entry) => {
                    if self.filter.matches(&entry) {
                        return Some(LogStreamItem::Entry(entry));
                    }
                }
                Err(RecvError::Lagged(missed)) => {
                    return Some(LogStreamItem::Lagged {
                        missed,
                        latest_seq: self.hub.latest_seq(),
                    });
                }
                Err(RecvError::Closed) => return None,
            }
        }
    }
}

/// Deep module owning the in-memory Log Ring Buffer, redaction gate, and live broadcast stream.
pub struct SystemLogHub {
    capacity: usize,
    next_seq: AtomicU64,
    ring: RwLock<RingState>,
    broadcaster: broadcast::Sender<Arc<StructuredLogEntry>>,
}

impl Default for SystemLogHub {
    fn default() -> Self {
        Self::new()
    }
}

impl SystemLogHub {
    pub fn new() -> Self {
        Self::with_capacity(DEFAULT_RING_BUFFER_CAPACITY, DEFAULT_BROADCAST_CAPACITY)
    }

    pub fn with_capacity(ring_capacity: usize, broadcast_capacity: usize) -> Self {
        let (broadcaster, _) = broadcast::channel(broadcast_capacity.max(1));
        Self {
            capacity: ring_capacity.max(1),
            next_seq: AtomicU64::new(0),
            ring: RwLock::new(RingState {
                entries: VecDeque::with_capacity(ring_capacity.min(2_048)),
                dropped_evicted: 0,
                counts: LogLevelSummary::default(),
            }),
            broadcaster,
        }
    }

    pub fn global() -> Arc<Self> {
        static GLOBAL: OnceLock<Arc<SystemLogHub>> = OnceLock::new();
        Arc::clone(GLOBAL.get_or_init(|| Arc::new(Self::new())))
    }

    pub fn latest_seq(&self) -> u64 {
        self.next_seq.load(Ordering::Acquire)
    }

    pub fn capacity(&self) -> usize {
        self.capacity
    }

    pub fn summary_counts(&self) -> LogLevelSummary {
        self.ring
            .read()
            .expect("system log ring lock")
            .counts
            .clone()
    }

    /// Sanitizes, sequences, buffers, and broadcasts one operational log observation.
    /// Returns `None` when suppressed by the self-observation noise gate.
    pub fn record(&self, raw: RawLogRecord) -> Option<u64> {
        if is_self_observation_noise(&raw) {
            return None;
        }
        let seq = self.next_seq.fetch_add(1, Ordering::AcqRel) + 1;
        let entry = normalize_entry(seq, raw);
        let shared = Arc::new(entry.clone());
        {
            let mut guard = self.ring.write().expect("system log ring lock");
            while guard.entries.len() >= self.capacity {
                if let Some(evicted) = guard.entries.pop_front() {
                    guard.decrement_count(evicted.level);
                    guard.dropped_evicted = guard.dropped_evicted.saturating_add(1);
                }
            }
            guard.increment_count(entry.level);
            guard.entries.push_back(entry);
        }
        let _ = self.broadcaster.send(shared);
        Some(seq)
    }

    /// Queries the buffered log snapshot or fetches entries newer than `filter.after_seq`.
    #[allow(clippy::needless_lifetimes)]
    pub fn query<'a>(&self, filter: &'a LogFilter) -> Result<SystemLogPage, &'static str> {
        let parsed = ParsedFilter::parse(filter)?;
        let guard = self.ring.read().expect("system log ring lock");
        let oldest_seq = guard.entries.front().map(|e| e.seq);
        let latest_seq = self.next_seq.load(Ordering::Acquire);
        let buffered_count = guard.entries.len();
        let dropped_evicted = guard.dropped_evicted;
        let counts = guard.counts.clone();

        let items: Vec<StructuredLogEntry> = if parsed.after_seq.is_some() {
            guard
                .entries
                .iter()
                .filter(|entry| parsed.matches(entry))
                .take(parsed.limit)
                .cloned()
                .collect()
        } else {
            let mut matched: Vec<StructuredLogEntry> = guard
                .entries
                .iter()
                .rev()
                .filter(|entry| parsed.matches(entry))
                .take(parsed.limit)
                .cloned()
                .collect();
            matched.reverse();
            matched
        };

        Ok(SystemLogPage {
            items,
            oldest_seq,
            latest_seq,
            buffered_count,
            capacity: self.capacity,
            dropped_evicted,
            counts,
        })
    }

    /// Subscribes to live log entries matching `filter`.
    pub fn subscribe(
        self: &Arc<Self>,
        filter: &LogFilter,
    ) -> Result<FilteredLogSubscription, &'static str> {
        let parsed = ParsedFilter::parse(filter)?;
        let rx = self.broadcaster.subscribe();
        let latest_seq_at_subscribe = self.latest_seq();
        Ok(FilteredLogSubscription {
            rx,
            filter: parsed,
            hub: Arc::clone(self),
            latest_seq_at_subscribe,
            capacity: self.capacity,
        })
    }
}

/// Initializes the process-wide `tracing` subscriber combining `EnvFilter`, `fmt::Layer`
/// (`VOICE_AGENT_LOG_FORMAT=json|pretty|compact`), and [`LogHubLayer`].
pub fn init_tracing_subscriber(hub: Arc<SystemLogHub>) {
    let env_filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    let format = std::env::var("VOICE_AGENT_LOG_FORMAT")
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();

    let registry = tracing_subscriber::registry()
        .with(env_filter)
        .with(LogHubLayer::new(hub));

    match format.as_str() {
        "json" => {
            let _ = registry
                .with(tracing_subscriber::fmt::layer().json().with_current_span(true))
                .try_init();
        }
        "pretty" => {
            let _ = registry
                .with(tracing_subscriber::fmt::layer().pretty())
                .try_init();
        }
        _ => {
            let _ = registry
                .with(tracing_subscriber::fmt::layer().compact())
                .try_init();
        }
    }
}
