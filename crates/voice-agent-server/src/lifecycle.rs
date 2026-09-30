//! The application-owned runtime lifecycle: one admission/work gate, one drain registry, and the
//! ordered shutdown that uses them.
//!
//! Two failures have to be told apart, and this module is where they are.  **Liveness** says the
//! process is running; it is deliberately independent of anything else, so a database outage can
//! never make an orchestrator kill a process that is still serving its already-admitted Voice
//! Sessions.  **Readiness** says the process can accept a *new* connection, and it is built only
//! from the dependencies the application owns: startup/schema state, the required RuntimeCatalog,
//! the admission resolver and the database whenever an active feature needs it.  It never runs
//! full admission, never resolves a Device, never discovers External MCP and never reloads a
//! provider or a secret — a probe that expensive would turn a health check into load.
//!
//! Shutdown is the mirror image of that split, and it is ordered rather than best-effort:
//!
//! 1. the admission/work gate closes, so no new listener, DB admission or Tool-round work starts;
//! 2. open Voice Sessions drain on their own until the configured grace deadline;
//! 3. whatever is still registered is issued a *controlled* close at that deadline;
//! 4. the archival writer flushes best-effort inside the same deadline, never extending it.
//!
//! Step 1 is deliberately independent of any single Voice Session observing shutdown: the gate is
//! flipped by the application, so a session that never notices cannot keep the process accepting
//! work.  The drain registry holds a registered completion handle per session, so shutdown can
//! *observe* completion rather than guess it, and the controlled close is a protocol close the
//! session itself performs — task abort is never the normal mechanism, only a last resort for a
//! session that has stopped answering entirely.

use std::{
    collections::HashMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::Duration,
};

use tokio::{sync::watch, time::Instant};
use tokio_util::sync::CancellationToken;

use crate::database::history::HistoryWriterMetrics;

/// How long shutdown waits after issuing controlled closes before the process stops waiting at
/// all.
///
/// This is not a second grace period and it never becomes one: it bounds how long a close frame
/// and the session's own teardown may take to land, and nothing more.  A session that has not gone
/// by then is a session that stopped answering, which the process owner resolves out of band.
pub const CONTROLLED_CLOSE_SETTLE: Duration = Duration::from_secs(2);

/// How often the shutdown sequence re-reads the archival writer's counters while it waits.
///
/// The wait is best-effort by contract, so the only thing a longer poll interval changes is how
/// promptly it notices that there is nothing left to flush — never whether it eventually returns.
const HISTORY_FLUSH_POLL: Duration = Duration::from_millis(20);

/// Whether this process can accept a *new* Voice connection right now.
///
/// These are the only states readiness reports, and each one names an application-owned dependency
/// rather than a symptom of somebody else's.  There is deliberately no class for "an External MCP
/// server is down" and none for "a Device is unknown": neither is a property of this process's
/// ability to accept a connection, and a probe that resolved either would be doing a session's job.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Readiness {
    Ready,
    /// The shutdown grace deadline has passed, or shutdown has begun: this process is finishing the
    /// sessions it already has and will not take another.
    ShuttingDown,
    /// The database is configured but this process holds none, so no schema is authoritative yet.
    StartupIncomplete,
    /// A provider instance this deployment requires is absent from the RuntimeCatalog, so a session
    /// admitted now could not be given a runtime to run on.
    RequiredRuntimeUnavailable,
    /// Database-backed admission is configured and the database did not answer.  A Voice Session
    /// already admitted keeps its snapshot and is unaffected.
    DatabaseUnreachable,
}

impl Readiness {
    /// The bounded word this state is reported and logged as.  It never names a Device, an Agent, a
    /// provider key or a destination.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Ready => "ready",
            Self::ShuttingDown => "shutting_down",
            Self::StartupIncomplete => "startup_incomplete",
            Self::RequiredRuntimeUnavailable => "required_runtime_unavailable",
            Self::DatabaseUnreachable => "database_unreachable",
        }
    }

    pub fn is_ready(self) -> bool {
        matches!(self, Self::Ready)
    }
}

/// The one application-owned gate every piece of new work passes.
///
/// It is closed by the application during shutdown and by nothing else.  A Voice Session that has
/// not observed shutdown — because it never received the signal, or because it is mid-turn — still
/// cannot start new work once this is closed, which is what makes the gate rather than a
/// per-session flag the thing shutdown actually depends on.
#[derive(Debug, Default)]
pub struct AdmissionGate {
    open: AtomicBool,
}

impl AdmissionGate {
    /// A gate that is already open and can be closed exactly once.
    pub fn open() -> Arc<Self> {
        Arc::new(Self {
            open: AtomicBool::new(true),
        })
    }

    /// Whether new work may start.  Cheap enough for a request path and for a tool round.
    pub fn is_open(&self) -> bool {
        self.open.load(Ordering::Acquire)
    }

    /// Closes the gate for good.  Returns whether this call was the one that closed it, so an
    /// ordered shutdown can log the transition exactly once without any caller having to be first.
    pub fn close(&self) -> bool {
        self.open.swap(false, Ordering::AcqRel)
    }
}

/// One registered Voice Session's completion handle.
///
/// Holding this is what makes a session countable: it registers before the session begins work and
/// drops when the connection is finished, so the registry's count is a fact about live sessions
/// rather than an estimate.  It carries a controlled-close signal of its own, so the deadline can
/// reach a specific session instead of broadcasting to whatever happens to be listening.
#[derive(Debug)]
pub struct DrainRegistration {
    registry: Arc<DrainRegistry>,
    id: u64,
    close: CancellationToken,
}

impl DrainRegistration {
    /// The signal that asks *this* session to close, without deciding how it closes.
    pub fn close_signal(&self) -> CancellationToken {
        self.close.clone()
    }
}

impl Drop for DrainRegistration {
    fn drop(&mut self) {
        self.registry.unregister(self.id);
    }
}

/// Every Voice Session this process has accepted and not yet finished.
///
/// The registry stores one cancellation token per live session and nothing else, so its memory is
/// bounded by the sessions in flight and it holds no transcript, prompt, Device identity or
/// credential — only the ability to ask one session to close.
#[derive(Debug)]
pub struct DrainRegistry {
    sessions: Mutex<HashMap<u64, CancellationToken>>,
    next_id: AtomicU64,
    active: watch::Sender<usize>,
}

impl DrainRegistry {
    pub fn new() -> Arc<Self> {
        let (active, _) = watch::channel(0);
        Arc::new(Self {
            sessions: Mutex::new(HashMap::new()),
            next_id: AtomicU64::new(1),
            active,
        })
    }

    /// Registers a session before it begins work.  The returned handle deregisters on drop, which
    /// is the only way a session ever leaves the registry.
    pub fn register(self: &Arc<Self>) -> DrainRegistration {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let close = CancellationToken::new();
        let mut sessions = self
            .sessions
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        sessions.insert(id, close.clone());
        let active = sessions.len();
        drop(sessions);
        self.active.send_replace(active);
        DrainRegistration {
            registry: Arc::clone(self),
            id,
            close,
        }
    }

    /// How many sessions have registered and not yet completed.
    pub fn active(&self) -> usize {
        *self.active.borrow()
    }

    fn unregister(&self, id: u64) {
        let mut sessions = self
            .sessions
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if sessions.remove(&id).is_none() {
            return;
        }
        let active = sessions.len();
        drop(sessions);
        self.active.send_replace(active);
    }

    /// Resolves once no session is registered, or when the registry is gone.
    pub async fn drained(&self) {
        let mut active = self.active.subscribe();
        while *active.borrow_and_update() > 0 {
            if active.changed().await.is_err() {
                return;
            }
        }
    }

    /// Asks every still-registered session to perform its controlled close, and reports how many
    /// were asked.  It returns immediately and never waits: the close is the session's own
    /// protocol close, not a force.
    pub fn controlled_close_all(&self) -> usize {
        let sessions = self
            .sessions
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let closed = sessions.len();
        for close in sessions.values() {
            close.cancel();
        }
        closed
    }
}

/// How a drain ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DrainOutcome {
    /// Every registered session completed on its own before the deadline.
    Drained,
    /// The deadline arrived with sessions still registered.  They are closed at this point, not
    /// before it.
    DeadlineReached { remaining: usize },
}

/// What one ordered shutdown observed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ShutdownReport {
    pub outcome: DrainOutcome,
    /// Sessions that were issued a controlled close at the deadline.
    pub controlled_closes: usize,
    /// Whether the archival writer held no record when the shared deadline passed.
    pub history_flushed: bool,
}

/// Everything shutdown coordinates, owned by the application rather than by any one task.
///
/// The lifetime here is the process's: it is created before the router exists and outlives the
/// listener, so a component built during startup can hold it and the shutdown sequence has the
/// same view of the gate and the registry that those components do.
#[derive(Debug)]
pub struct RuntimeLifecycle {
    gate: Arc<AdmissionGate>,
    drain: Arc<DrainRegistry>,
    /// Stops the listener from accepting new connections.
    listening: CancellationToken,
    /// The hard stop. It ends maintenance tasks and abandons any session still running.
    stopping: CancellationToken,
    grace: Duration,
    /// The archival writer's counters, once the archive that owns them exists.  Registered by
    /// `AppState` because only it knows whether a writer was ever started.
    history: std::sync::OnceLock<Arc<HistoryWriterMetrics>>,
}

impl RuntimeLifecycle {
    /// A lifecycle with its own signals, for a deployment that has nothing outside it to stop.
    pub fn new(grace: Duration) -> Arc<Self> {
        Arc::new(Self {
            gate: AdmissionGate::open(),
            drain: DrainRegistry::new(),
            listening: CancellationToken::new(),
            stopping: CancellationToken::new(),
            grace,
            history: std::sync::OnceLock::new(),
        })
    }

    /// The lifecycle a deployment's own configuration describes.
    ///
    /// This is the only reader of `shutdown.grace_ms`, so the process and every router built from
    /// the same configuration agree on the deadline instead of each deriving one.
    pub fn from_config(config: &crate::config::AppConfig) -> Arc<Self> {
        Self::new(Duration::from_millis(config.shutdown.grace_ms))
    }

    pub fn gate(&self) -> &Arc<AdmissionGate> {
        &self.gate
    }

    pub fn drain(&self) -> &Arc<DrainRegistry> {
        &self.drain
    }

    /// Cancelled when the process stops accepting new connections.
    pub fn listening(&self) -> &CancellationToken {
        &self.listening
    }

    /// Cancelled once the ordered shutdown has finished what it can.
    pub fn stopping(&self) -> &CancellationToken {
        &self.stopping
    }

    pub fn grace(&self) -> Duration {
        self.grace
    }

    /// Publishes the archival writer's counters so shutdown can observe a flush.
    ///
    /// A deployment with capture off never registers any, and its shutdown then has nothing to
    /// wait for — which is the correct answer, not a missing step.
    pub fn observe_history_writer(&self, metrics: Arc<HistoryWriterMetrics>) {
        let _ = self.history.set(metrics);
    }

    /// The one transition every ordered shutdown starts from: no new work, no new connections.
    ///
    /// Returns whether this call closed the gate.  It is safe to call more than once, so a signal
    /// and a test may both drive it without either having to know about the other.
    pub fn begin_shutdown(&self) -> bool {
        self.listening.cancel();
        self.gate.close()
    }

    /// The whole ordered shutdown: close the gate, drain to one shared deadline, controlled-close
    /// whatever is left, then stop.
    ///
    /// The drain and the archival flush wait on the *same* deadline, so the flush can only ever use
    /// time the drain did not already spend and can never extend the shutdown.
    pub async fn shutdown(&self) -> ShutdownReport {
        let closed_gate = self.begin_shutdown();
        if closed_gate {
            tracing::info!(
                event = "admission_gate_closed",
                "The application admission and work gate is closed; no new connection, admission or \
                 tool round will start"
            );
        }
        let deadline = Instant::now() + self.grace;
        let outcome = self.drain_until(deadline).await;
        let controlled_closes = match outcome {
            DrainOutcome::Drained => 0,
            DrainOutcome::DeadlineReached { .. } => {
                let closed = self.drain.controlled_close_all();
                tracing::warn!(
                    event = "session_drain_deadline_reached",
                    remaining = closed,
                    "The shutdown grace deadline passed with Voice Sessions still open; issuing a \
                     controlled close to each"
                );
                closed
            }
        };
        // Only what the drain did not spend, on the same deadline. It runs after the closes are
        // issued rather than beside the drain, because a session that is still draining is still
        // enqueueing records and a writer measured against a moving target settles by accident.
        let history_flushed = self.flush_until(deadline).await;
        self.stopping.cancel();
        ShutdownReport {
            outcome,
            controlled_closes,
            history_flushed,
        }
    }

    /// Waits for every registered session to finish on its own, or returns the sessions still open
    /// at `deadline`.
    async fn drain_until(&self, deadline: Instant) -> DrainOutcome {
        if self.drain.active() == 0 {
            return DrainOutcome::Drained;
        }
        tokio::select! {
            _ = self.drain.drained() => DrainOutcome::Drained,
            _ = tokio::time::sleep_until(deadline) => match self.drain.active() {
                0 => DrainOutcome::Drained,
                remaining => DrainOutcome::DeadlineReached { remaining },
            },
        }
    }

    /// Best-effort archival flush, bounded by the caller's deadline.
    ///
    /// There is no retry and no extension: an archive that cannot settle inside the deadline the
    /// rest of the shutdown is already using loses the records it had not written, which is the
    /// same outcome the writer's own bounded queue produces at any other time.  Those records are
    /// counted once under their own bounded class, so an operator sees them go rather than seeing
    /// the archive quietly stop.
    async fn flush_until(&self, deadline: Instant) -> bool {
        let Some(metrics) = self.history.get() else {
            // No writer was ever started, so there is nothing that could be outstanding.
            return true;
        };
        loop {
            if metrics.counters().is_settled() {
                return true;
            }
            let now = Instant::now();
            if now >= deadline {
                metrics.record_shutdown_drop();
                return false;
            }
            tokio::time::sleep(HISTORY_FLUSH_POLL.min(deadline - now)).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::GenerationGate;

    fn registry_with(registry: &Arc<DrainRegistry>, sessions: usize) -> Vec<DrainRegistration> {
        let handles = (0..sessions).map(|_| registry.register()).collect();
        assert_eq!(registry.active(), sessions);
        handles
    }

    /// The gate is what shutdown depends on, so it must be closed by the application alone.  A
    /// session that never sees a shutdown signal still cannot start new work afterwards.
    #[test]
    fn a_closed_gate_refuses_new_work_forever() {
        let gate = AdmissionGate::open();
        assert!(gate.is_open());
        assert!(gate.close(), "the first close is the transition");
        assert!(!gate.is_open());
        assert!(!gate.close(), "closing again is not a second transition");
        assert!(!gate.is_open());
    }

    #[tokio::test]
    async fn a_registry_counts_a_session_until_its_handle_drops() {
        let registry = DrainRegistry::new();
        let mut handles = registry_with(&registry, 2);
        drop(handles.pop());
        assert_eq!(registry.active(), 1);
        drop(handles);
        assert_eq!(registry.active(), 0);
        registry.drained().await;
    }

    /// A completed session deregisters itself, so shutdown can observe drain completion rather
    /// than wait out the whole grace deadline on a process that is already empty.
    #[tokio::test]
    async fn a_drain_that_finishes_early_never_reaches_the_deadline() {
        let lifecycle = RuntimeLifecycle::new(Duration::from_secs(30));
        let session = lifecycle.drain().register();
        let drained = {
            let drain = Arc::clone(lifecycle.drain());
            tokio::spawn(async move { drain.drained().await })
        };
        drop(session);
        tokio::time::timeout(Duration::from_secs(5), drained)
            .await
            .expect("an emptied registry wakes its waiter")
            .expect("the drain task completes");
    }

    /// The deadline is a backstop, not a schedule: a session still open at it is closed rather
    /// than waited for.  The listener has already stopped accepting by then, and the hard-stop
    /// signal is only raised once the drain and the flush have both finished with it.
    #[tokio::test(start_paused = true)]
    async fn the_deadline_reports_the_sessions_still_open() {
        let lifecycle = RuntimeLifecycle::new(Duration::from_secs(30));
        let session = lifecycle.drain().register();

        let owned = Arc::clone(&lifecycle);
        let shutdown = tokio::spawn(async move { owned.shutdown().await });
        // Past the grace deadline with the session still registered.
        tokio::time::advance(Duration::from_secs(31)).await;
        let report = shutdown
            .await
            .expect("the ordered shutdown returns at its deadline");

        assert_eq!(
            report.outcome,
            DrainOutcome::DeadlineReached { remaining: 1 }
        );
        assert_eq!(report.controlled_closes, 1);
        assert!(
            report.history_flushed,
            "a capture-off process has no archival record to flush"
        );
        assert!(
            lifecycle.listening().is_cancelled(),
            "no new connection is accepted once shutdown begins"
        );
        assert!(lifecycle.stopping().is_cancelled());
        assert!(!lifecycle.gate().is_open());
        assert!(
            session.close_signal().is_cancelled(),
            "the session still open at the deadline is the one that was signalled"
        );
        assert_eq!(
            lifecycle.drain().active(),
            1,
            "and it is still registered until it finishes"
        );
        drop(session);
    }

    /// A process with nothing open shuts down on its own terms, without waiting out the grace.
    #[tokio::test(start_paused = true)]
    async fn an_empty_process_drains_without_reaching_the_deadline() {
        let lifecycle = RuntimeLifecycle::new(Duration::from_secs(600));
        let started = Instant::now();
        let report = lifecycle.shutdown().await;
        assert_eq!(report.outcome, DrainOutcome::Drained);
        assert_eq!(report.controlled_closes, 0);
        assert!(report.history_flushed);
        assert_eq!(
            started.elapsed(),
            Duration::ZERO,
            "an empty drain returns on drain completion; only the clock advancing moves it"
        );
    }

    /// The controlled close reaches the sessions that were still open at the deadline, and only
    /// those: it is a signal each session performs itself, not a broadcast with side effects.
    #[tokio::test]
    async fn the_deadline_signals_only_the_sessions_still_open() {
        let registry = DrainRegistry::new();
        let mut handles = registry_with(&registry, 2);
        let finished = handles.pop().expect("two registered sessions");
        let open = handles.pop().expect("one still-registered session");
        let finished_signal = finished.close_signal();
        drop(finished);

        assert_eq!(registry.controlled_close_all(), 1);
        assert!(open.close_signal().is_cancelled());
        assert!(
            !finished_signal.is_cancelled(),
            "a session that already finished is not asked to close again"
        );
    }

    /// The gate is the only thing new work asks, and the generation gate a session uses for its
    /// own turn ordering is a different decision with a different owner.  Closing the lifecycle
    /// gate must not disturb a session's own outbound ordering.
    #[test]
    fn the_admission_gate_is_not_a_session_generation_gate() {
        let lifecycle_gate = AdmissionGate::open();
        let generation_gate = GenerationGate::new();
        assert!(lifecycle_gate.is_open());
        assert!(generation_gate.admits(1));

        lifecycle_gate.close();

        assert!(!lifecycle_gate.is_open());
        assert!(
            generation_gate.admits(1),
            "a session's turn ordering is untouched by process shutdown"
        );
        assert!(
            !lifecycle_gate.is_open(),
            "and the lifecycle gate is still closed afterwards"
        );
    }

    /// A writer that has already stored everything it accepted settles immediately, so a shutdown
    /// is never held up by an archive that has nothing left to do.
    #[tokio::test(start_paused = true)]
    async fn an_archived_writer_is_flushed_without_waiting_for_the_deadline() {
        let lifecycle = RuntimeLifecycle::new(Duration::from_secs(600));
        let metrics = Arc::new(HistoryWriterMetrics::default());
        metrics.record_enqueued();
        metrics.record_written();
        lifecycle.observe_history_writer(Arc::clone(&metrics));

        let started = Instant::now();
        let report = lifecycle.shutdown().await;

        assert!(report.history_flushed);
        assert_eq!(
            started.elapsed(),
            Duration::ZERO,
            "a settled archive is read once, not waited on"
        );
    }

    /// An archive that cannot finish inside the deadline loses the records it had not written.
    /// That is the same outcome the writer's own bounded queue produces at any other time, and it
    /// is reported rather than waited out: the flush is best-effort and never extends shutdown.
    #[tokio::test(start_paused = true)]
    async fn a_stuck_writer_costs_records_at_the_deadline_and_nothing_more() {
        let grace = Duration::from_secs(30);
        let lifecycle = RuntimeLifecycle::new(grace);
        // Enqueued with no outcome: the writer holds the record and never will produce one.
        let metrics = Arc::new(HistoryWriterMetrics::default());
        metrics.record_enqueued();
        lifecycle.observe_history_writer(Arc::clone(&metrics));

        let started = Instant::now();
        let owned = Arc::clone(&lifecycle);
        let shutdown = tokio::spawn(async move { owned.shutdown().await });
        tokio::time::advance(grace).await;
        let report = shutdown
            .await
            .expect("the flush returns at the shared deadline");

        assert!(
            !report.history_flushed,
            "a writer still holding records at the deadline is reported as unflushed"
        );
        assert!(
            started.elapsed() >= grace,
            "the flush waits for the shared deadline and not a moment past it: {:?}",
            started.elapsed()
        );
        // Resolving without any further clock movement is the proof it never extended: a flush that
        // retried or kept waiting would still be pending here.
        let counters = metrics.counters();
        assert_eq!(
            counters.enqueued, 1,
            "the record was accepted by the hand-off"
        );
        assert_eq!(
            counters.dropped_shutdown, 1,
            "and it is reported under its own bounded class rather than silently lost"
        );
    }
}
