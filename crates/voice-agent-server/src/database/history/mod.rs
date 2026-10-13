//! Optional Persistent Transcript: the archival writer a Voice Session hands records to, and the
//! retention that keeps the archive bounded.
//!
//! Those are two policies, and this module keeps them apart on purpose.  **Capture** is opt-in and
//! decides whether anything is enqueued at all: with it off there is no writer, no queue and no
//! task, so a deployment that never asked for a transcript has no archival path to leak through.
//! **Retention** is a property of the archive rather than of capture, so it runs whenever the
//! database does — turning capture off must not turn whatever is already archived into unbounded
//! retention.  Only the cleaner can touch an existing record, and all it can do is delete it.
//!
//! The archive is a copy, never an authority.  Nothing in this module is read back into a
//! Conversational Turn: a Voice Session keeps its RAM Dialogue History whatever this does, so a
//! full queue, a stopped writer or a database failure costs at most the record that carried them.
//! For the same reason the write hand-off is `try_send` and never an await — a session must not
//! be able to wait on SQLite, and a record must not be able to fail a turn.
//!
//! Attribution is decided where the text exists.  A record carries the admission identity of its
//! Voice Session and the Template that session was running at that moment, so an admin mutation or
//! a later Template switch cannot rewrite what a stored record claims about itself, and nothing has
//! to reconstruct it from the archive afterwards.
//!
//! Read and purge are deliberately absent here: those are the Admin API's surfaces and live in
//! `app::admin::history`, which owns its own typed query and its audit transaction.

pub(crate) mod queries;

use crate::config::DatabaseHistoryConfig;
use std::{
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tokio::{sync::mpsc, task::JoinHandle};
use tokio_util::sync::CancellationToken;

use super::{Database, DatabaseError, map_sqlx_error};

/// Records the archival writer actually persisted.
pub const HISTORY_WRITTEN_TOTAL: &str = "history_written_total";
/// Records the archival writer could not persist, labelled by [`HistoryDrop`].
pub const HISTORY_DROPPED_TOTAL: &str = "history_dropped_total";

/// What one archived text may be: between one byte and [`MAX_TRANSCRIPT_TEXT_BYTES`] of text that
/// says something.
///
/// The ceiling is the point — a Voice Session bounds how much text it keeps in RAM, and an unbounded
/// record would let one turn's response write an unbounded row.  A text past it is refused as a whole
/// rather than truncated, because a truncated archive entry is indistinguishable from a short
/// answer.  The floor is the same contract read from the other side: a record exists to carry text
/// somebody said, so blank text is not a shorter record but no record at all.  Both seams that
/// produce a record already guarantee the floor, so in practice the ceiling is what this refuses.
pub const MAX_TRANSCRIPT_TEXT_BYTES: usize = 16 * 1024;

/// Retention is maintenance, not realtime work: one cleanup at startup and then one per day.
pub const RETENTION_INTERVAL: Duration = Duration::from_secs(24 * 60 * 60);
const MILLIS_PER_DAY: i64 = 24 * 60 * 60 * 1_000;

/// Which side of a record's text this archive stores.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HistoryRole {
    /// The final user text the session accepted for the turn.
    User,
    /// The Delivered Assistant Response the writer closed the turn with.
    Assistant,
}

impl HistoryRole {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::User => "user",
            Self::Assistant => "assistant",
        }
    }

    /// The only two words a query filter may name, so no other role can reach the archive.
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "user" => Some(Self::User),
            "assistant" => Some(Self::Assistant),
            _ => None,
        }
    }
}

/// The bounded classes one dropped record can report.
///
/// They are a closed set for the same reason the External MCP telemetry classes are: a metric here
/// carries no session id, Device identity, Template id or transcript text, so nothing an operator
/// says, a device sends or a model generates can become a label.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HistoryDrop {
    /// The bounded hand-off was full: the writer is behind and this record would have waited.
    QueueFull,
    /// The writer task has stopped, so no record can be accepted at all.
    WriterClosed,
    /// The archive could not store the record.
    Database,
    /// The text is outside what one archived record may hold.
    TextOutOfBounds,
    /// The archival writer still held records when the shutdown deadline passed.  It is its own
    /// class because the record was never refused by the queue or the database — the process simply
    /// stopped before it could be written, which is a different fact with a different fix.
    Shutdown,
}

impl HistoryDrop {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::QueueFull => "queue_full",
            Self::WriterClosed => "writer_closed",
            Self::Database => "database",
            Self::TextOutOfBounds => "text_out_of_bounds",
            Self::Shutdown => "shutdown",
        }
    }
}

/// One Persistent Transcript record, snapshotted where the Voice Session produced it.
pub struct HistoryWrite {
    pub session_id: String,
    pub device_id: i64,
    pub agent_id: i64,
    /// The Template that was active at this moment, or `None` for server defaults.  A switch that
    /// is armed but not yet applied is still the old Template, which is what the turn ran on.
    pub template_id: Option<i64>,
    /// The session's own monotonic counter, so `UNIQUE(session_id, sequence)` holds without the
    /// writer ever asking the database for the next number.
    ///
    /// It is a counter, not a position: a record the archive or the hand-off refused has already
    /// taken a number, so the stored rows can have a gap where one was dropped.  What can never
    /// happen is a reuse, which is the half `UNIQUE` actually cares about.
    pub sequence: i64,
    pub turn_id: String,
    pub role: HistoryRole,
    pub text: String,
    /// Unix milliseconds UTC: the same unit the retention cutoff is computed in.
    pub created_at: i64,
}

/// Bounded counters for the archival writer.
///
/// Every field counts one fixed class of event, so a process that runs for months accumulates a
/// fixed number of them no matter how many sessions it serves.  `enqueued` is what the bounded
/// hand-off accepted and `written` is what the archive stored, so their difference is exactly the
/// records the writer is still holding.
#[derive(Debug, Default)]
pub struct HistoryWriterMetrics {
    enqueued: AtomicU64,
    written: AtomicU64,
    queue_full: AtomicU64,
    writer_closed: AtomicU64,
    database: AtomicU64,
    text_out_of_bounds: AtomicU64,
    shutdown: AtomicU64,
}

/// One reading of the counters, so a caller never sees a half-updated set.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct HistoryWriterCounters {
    pub enqueued: u64,
    pub written: u64,
    pub dropped: u64,
    pub dropped_queue_full: u64,
    pub dropped_writer_closed: u64,
    pub dropped_database: u64,
    pub dropped_text_out_of_bounds: u64,
    pub dropped_shutdown: u64,
}

impl HistoryWriterCounters {
    /// Whether the writer holds no records: everything the bounded hand-off accepted has been
    /// stored or dropped by the archive.
    ///
    /// A record the hand-off refused never reached the writer, so it is not part of this question —
    /// `enqueued` already excludes it, which is what makes this a "has it finished?" check rather
    /// than a count.
    pub fn is_settled(&self) -> bool {
        self.written + self.dropped_database == self.enqueued
    }
}

impl HistoryWriterMetrics {
    pub fn counters(&self) -> HistoryWriterCounters {
        let queue_full = self.queue_full.load(Ordering::Relaxed);
        let writer_closed = self.writer_closed.load(Ordering::Relaxed);
        let database = self.database.load(Ordering::Relaxed);
        let text_out_of_bounds = self.text_out_of_bounds.load(Ordering::Relaxed);
        let shutdown = self.shutdown.load(Ordering::Relaxed);
        HistoryWriterCounters {
            enqueued: self.enqueued.load(Ordering::Relaxed),
            written: self.written.load(Ordering::Relaxed),
            dropped: queue_full + writer_closed + database + text_out_of_bounds + shutdown,
            dropped_queue_full: queue_full,
            dropped_writer_closed: writer_closed,
            dropped_database: database,
            dropped_text_out_of_bounds: text_out_of_bounds,
            dropped_shutdown: shutdown,
        }
    }

    pub(crate) fn record_enqueued(&self) {
        self.enqueued.fetch_add(1, Ordering::Relaxed);
    }

    pub(crate) fn record_written(&self) {
        self.written.fetch_add(1, Ordering::Relaxed);
    }

    /// A pure counter: it is what the bounded metrics *are*, and knowing which class fired is the
    /// whole value, so the log line that explains a drop is raised where the drop was decided.
    pub(crate) fn record_drop(&self, drop: HistoryDrop) {
        match drop {
            HistoryDrop::QueueFull => {
                self.queue_full.fetch_add(1, Ordering::Relaxed);
            }
            HistoryDrop::WriterClosed => {
                self.writer_closed.fetch_add(1, Ordering::Relaxed);
            }
            HistoryDrop::Database => {
                self.database.fetch_add(1, Ordering::Relaxed);
            }
            HistoryDrop::TextOutOfBounds => {
                self.text_out_of_bounds.fetch_add(1, Ordering::Relaxed);
            }
            HistoryDrop::Shutdown => {
                self.shutdown.fetch_add(1, Ordering::Relaxed);
            }
        }
    }

    /// Counts, once, everything the writer still held when the shutdown deadline passed.
    ///
    /// `is_settled` already answers how many records are outstanding; recording the same fact again
    /// under its own class is what lets an operator see those records go rather than seeing the
    /// archive quietly stop writing.
    pub(crate) fn record_shutdown_drop(&self) {
        let counters = self.counters();
        let outstanding = counters
            .enqueued
            .saturating_sub(counters.written)
            .saturating_sub(counters.dropped_database);
        if outstanding == 0 {
            return;
        }
        self.shutdown.fetch_add(outstanding, Ordering::Relaxed);
        tracing::info!(
            event = "history_record_dropped",
            metric = HISTORY_DROPPED_TOTAL,
            reason = HistoryDrop::Shutdown.as_str(),
            dropped = outstanding,
            "The shutdown deadline passed with Persistent Transcript records still unwritten; they \
             are dropped, which is the archive's best-effort contract"
        );
    }
}

/// The bounded hand-off between a Voice Session and the archival writer task.
///
/// A session holds this instead of a database handle.  `try_send` is the entire contract: a full
/// queue or a writer that has stopped drops that one record and counts it.  There is no await, no
/// retry, no backpressure into the session and no outcome a Conversational Turn can fail on.
#[derive(Clone)]
pub struct HistoryWriter {
    records: mpsc::Sender<HistoryWrite>,
    metrics: Arc<HistoryWriterMetrics>,
}

impl HistoryWriter {
    /// Offers one record to the archival writer, best-effort.
    pub fn try_send(&self, record: HistoryWrite) {
        match self.records.try_send(record) {
            Ok(()) => {
                self.metrics.record_enqueued();
                tracing::debug!(
                    event = "history_record_written",
                    metric = HISTORY_WRITTEN_TOTAL,
                    "A Persistent Transcript record reached the bounded hand-off"
                );
            }
            Err(mpsc::error::TrySendError::Full(_)) => self.drop_record(HistoryDrop::QueueFull),
            Err(mpsc::error::TrySendError::Closed(_)) => {
                self.drop_record(HistoryDrop::WriterClosed)
            }
        }
    }

    /// Counts one record the archive will not have, and says why.
    ///
    /// The line carries the class and nothing else.  This hand-off deliberately owns no session
    /// identity, and a drop is one bounded class rather than one per session — so the counters are
    /// the operator's signal, and a log line that named a session here would be a label the archive
    /// never needed to keep.
    fn drop_record(&self, reason: HistoryDrop) {
        self.metrics.record_drop(reason);
        tracing::debug!(
            event = "history_record_dropped",
            metric = HISTORY_DROPPED_TOTAL,
            reason = reason.as_str(),
            "A Persistent Transcript record was dropped; the archive is best-effort and the voice \
             turn is unaffected"
        );
    }

    /// The bounded counters of this writer.  They are the only thing about it an operator or a test
    /// can observe, which is the point: the writer has no state a session can reach back into.
    pub fn metrics(&self) -> &Arc<HistoryWriterMetrics> {
        &self.metrics
    }
}

/// One Voice Session's handle on the Persistent Transcript.
///
/// It carries the admission identity every record of that session is attributed to, plus the
/// session's own sequence counter, so a session needs neither a repository nor a pool to archive a
/// turn.  A session admitted without a database identity is never bound: an archive row belongs to
/// its Device and Agent by foreign key, and inventing an identity for one would turn a missing
/// attribution into a write.
pub struct TranscriptCapture {
    writer: HistoryWriter,
    session_id: String,
    device_id: i64,
    agent_id: i64,
    sequence: i64,
}

impl TranscriptCapture {
    /// Binds one session to the archive, or `None` when it has no database identity to attribute
    /// a record to.
    pub fn new(
        writer: &HistoryWriter,
        session_id: &str,
        device_id: i64,
        agent_id: i64,
    ) -> Option<Self> {
        if device_id <= 0 || agent_id <= 0 {
            return None;
        }
        Some(Self {
            writer: writer.clone(),
            session_id: session_id.to_owned(),
            device_id,
            agent_id,
            sequence: 0,
        })
    }

    /// Archives one accepted text.
    ///
    /// This never waits, never retries and cannot fail the caller.  The Template is passed in
    /// rather than read here so the session decides attribution at the boundary that owns its
    /// switch state, and the text is bounded here so the archive's own ceiling cannot be widened
    /// from outside.
    pub fn record(
        &mut self,
        role: HistoryRole,
        template_id: Option<i64>,
        turn_id: &str,
        text: &str,
    ) {
        if text.trim().is_empty() || text.len() > MAX_TRANSCRIPT_TEXT_BYTES {
            self.writer.drop_record(HistoryDrop::TextOutOfBounds);
            return;
        }
        // Saturating rather than wrapping: a session that somehow reached the ceiling keeps
        // archiving under one sequence instead of reusing a number an earlier record owns.
        self.sequence = self.sequence.saturating_add(1);
        self.writer.try_send(HistoryWrite {
            session_id: self.session_id.clone(),
            device_id: self.device_id,
            agent_id: self.agent_id,
            template_id,
            sequence: self.sequence,
            turn_id: turn_id.to_owned(),
            role,
            text: text.to_owned(),
            created_at: unix_millis_now(),
        });
    }
}

/// The retention of the Persistent Transcript archive: one cleanup at startup, then one a day.
///
/// This is the part of the archive that outlives capture.  It exists whenever the database does,
/// because how long the archive keeps what it holds is a property of the archive and not of whether
/// anything is being added to it right now — turning capture off must not turn whatever is already
/// stored into unbounded retention.
///
/// It is also the only part of the archive that can touch an existing record, and the only thing it
/// can do with one is delete it.  There is no way to hand a record to a cleaner.
pub struct RetentionCleaner {
    task: JoinHandle<()>,
}

impl RetentionCleaner {
    /// Starts the scheduled cleanup.  Requires a Tokio runtime.
    ///
    /// `interval` is [`RETENTION_INTERVAL`] in production and is a parameter only so a test can
    /// watch the schedule fire without waiting a day for it.
    pub fn start(
        database: &Database,
        retention_days: u32,
        interval: Duration,
        shutdown: CancellationToken,
    ) -> Self {
        // The cleaner owns the database, not a borrow of it: a `Database` clone is an `Arc` over the
        // same pool, so the task can outlive this call without extending a lifetime into it.
        let database = database.clone();
        let task = tokio::spawn(async move {
            let mut schedule = tokio::time::interval(interval.max(Duration::from_millis(1)));
            // A run that overran its schedule waits for the next one instead of firing a burst of
            // cleanup passes back to back.
            schedule.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                tokio::select! {
                    _ = shutdown.cancelled() => break,
                    // The first tick is already due, so this is the startup run rather than a wait.
                    _ = schedule.tick() => {
                        if let Err(error) = database.purge_history_expired(retention_days).await {
                            tracing::warn!(
                                event = "history_retention_skipped",
                                retention_days,
                                reason = %error,
                                "A Persistent Transcript retention run was abandoned; the next \
                                 scheduled run still happens"
                            );
                        }
                    }
                }
            }
        });
        Self { task }
    }
}

impl Drop for RetentionCleaner {
    /// The cleaner stops with the archive that owns it.  A `Database` clone keeps the pool alive
    /// independently, so without this a dropped archive would leave a task deleting rows nothing
    /// else is still responsible for.
    fn drop(&mut self) {
        self.task.abort();
    }
}

/// The process-owned Persistent Transcript: the retention of the archive, plus the archival writer
/// when capture is on.
///
/// The two halves have different existences, and this type is where that is decided.  The retention
/// is unconditional — the database is enough.  The writer is not: it exists only when the deployment
/// asked for capture, so a process that never opted in has no archival queue and no archival task at
/// all, rather than an idle one waiting for a record that can never come.
pub struct HistoryArchive {
    /// Held for its `Drop`, which is the whole point: the cleaner's lifetime is the archive's, so a
    /// dropped archive leaves no task behind it still deleting rows.
    _retention: RetentionCleaner,
    writer: Option<HistoryWriter>,
}

impl HistoryArchive {
    /// Starts the retention, and the archival writer when capture is enabled.  Requires a Tokio
    /// runtime: both halves are tasks.
    pub fn start(
        database: &Database,
        config: &DatabaseHistoryConfig,
        shutdown: CancellationToken,
    ) -> Self {
        let retention = RetentionCleaner::start(
            database,
            config.retention_days,
            RETENTION_INTERVAL,
            shutdown,
        );
        let writer = config.enabled.then(|| start_writer(database, config));
        Self {
            _retention: retention,
            writer,
        }
    }

    /// The archival writer, or `None` when capture is off.  `None` is the whole opt-in boundary: a
    /// session that cannot be handed a writer has no way to enqueue a record.
    pub fn writer(&self) -> Option<&HistoryWriter> {
        self.writer.as_ref()
    }
}

/// Starts the one archival writer and its task.  Separate from [`HistoryArchive`] so the capture
/// decision stays a single `then` rather than something a caller can half-apply.
fn start_writer(database: &Database, config: &DatabaseHistoryConfig) -> HistoryWriter {
    let metrics = Arc::new(HistoryWriterMetrics::default());
    let (records, received) = mpsc::channel(config.queue_capacity);
    let writer = HistoryWriter {
        records,
        metrics: Arc::clone(&metrics),
    };
    tokio::spawn(writer_loop(
        database.clone(),
        received,
        Arc::clone(&metrics),
    ));
    writer
}

/// The one task every session's `try_send` reaches.
///
/// A database failure counts and drops the record; it never stops the loop, because an archive that
/// recovers on its own is the point of a best-effort side effect.  The loop ends only when the last
/// session handle is gone, which is what turns a finished shutdown into a closed writer.  It is
/// deliberately *not* tied to the shutdown token: a session still draining inside its grace deadline
/// can keep archiving, which is the flush ADR 0066 allows.
async fn writer_loop(
    database: Database,
    mut received: mpsc::Receiver<HistoryWrite>,
    metrics: Arc<HistoryWriterMetrics>,
) {
    while let Some(record) = received.recv().await {
        match database.append_history(&record).await {
            Ok(()) => metrics.record_written(),
            Err(_) => {
                metrics.record_drop(HistoryDrop::Database);
                // The one drop whose record is still in hand, so this is the one drop that can be
                // correlated back to the session and turn that produced it.  It never carries the
                // text: an archive failure is not a reason to write the conversation anywhere else.
                tracing::debug!(
                    event = "history_record_dropped",
                    metric = HISTORY_DROPPED_TOTAL,
                    reason = HistoryDrop::Database.as_str(),
                    session_id = %record.session_id,
                    turn_id = %record.turn_id,
                    role = record.role.as_str(),
                    "The Persistent Transcript archive could not store a record"
                );
            }
        }
    }
}

/// Unix milliseconds UTC, the unit this archive stamps and prunes with.
pub fn unix_millis_now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis().min(i64::MAX as u128) as i64)
        .unwrap_or_default()
}

/// The absolute cutoff a retention run deletes at.
pub fn retention_cutoff(now_millis: i64, retention_days: u32) -> i64 {
    now_millis.saturating_sub(i64::from(retention_days).saturating_mul(MILLIS_PER_DAY))
}

impl Database {
    /// Appends one already-validated record.
    ///
    /// The caller has already decided that losing it is acceptable, so this reports the failure
    /// and nothing more: no retry, and nothing the Voice Session is waiting on.
    pub async fn append_history(&self, record: &HistoryWrite) -> Result<(), DatabaseError> {
        sqlx::query(
            "INSERT INTO history_messages \
             (session_id, device_id, agent_id, template_id, sequence, turn_id, role, text, created_at) \
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&record.session_id)
        .bind(record.device_id)
        .bind(record.agent_id)
        .bind(record.template_id)
        .bind(record.sequence)
        .bind(&record.turn_id)
        .bind(record.role.as_str())
        .bind(&record.text)
        .bind(record.created_at)
        .execute(&self.pool)
        .await
        .map(|_| ())
        .map_err(map_sqlx_error)
    }

    /// Deletes everything the archive holds that is older than the configured retention.
    ///
    /// The cutoff is absolute UTC rather than a scan, so a run that was skipped or that started
    /// late still deletes exactly the same set.  The row count is maintenance telemetry, not a
    /// client-visible number.
    pub async fn purge_history_expired(&self, retention_days: u32) -> Result<u64, DatabaseError> {
        let cutoff = retention_cutoff(unix_millis_now(), retention_days);
        sqlx::query("DELETE FROM history_messages WHERE created_at < ?")
            .bind(cutoff)
            .execute(&self.pool)
            .await
            .map(|result| result.rows_affected())
            .map_err(map_sqlx_error)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(sequence: i64, text: &str) -> HistoryWrite {
        HistoryWrite {
            session_id: "session".to_owned(),
            device_id: 1,
            agent_id: 1,
            template_id: None,
            sequence,
            turn_id: "1".to_owned(),
            role: HistoryRole::User,
            text: text.to_owned(),
            created_at: unix_millis_now(),
        }
    }

    /// A writer with nothing draining it, so the queue's own bound is what a record meets.  The
    /// receiver is returned so a caller that keeps it alive sees full-queue drops rather than the
    /// closed-writer drops a dropped receiver would produce.
    fn stalled_writer(capacity: usize) -> (HistoryWriter, mpsc::Receiver<HistoryWrite>) {
        let (records, received) = mpsc::channel(capacity);
        (
            HistoryWriter {
                records,
                metrics: Arc::new(HistoryWriterMetrics::default()),
            },
            received,
        )
    }

    #[test]
    fn a_full_queue_drops_one_record_and_never_waits() {
        let (writer, _received) = stalled_writer(1);
        let metrics = Arc::clone(&writer.metrics);

        writer.try_send(record(1, "first"));
        // The second `try_send` returns on a full queue rather than awaiting capacity.
        writer.try_send(record(2, "second"));

        assert_eq!(
            metrics.counters(),
            HistoryWriterCounters {
                enqueued: 1,
                dropped: 1,
                dropped_queue_full: 1,
                ..Default::default()
            },
            "accepted and stored are counted apart, so `written` keeps meaning the archive stored it"
        );
    }

    #[test]
    fn a_stopped_writer_drops_every_later_record() {
        let (records, received) = mpsc::channel(4);
        let metrics = Arc::new(HistoryWriterMetrics::default());
        let writer = HistoryWriter {
            records,
            metrics: Arc::clone(&metrics),
        };
        drop(received);

        writer.try_send(record(1, "first"));

        assert_eq!(
            metrics.counters(),
            HistoryWriterCounters {
                dropped: 1,
                dropped_writer_closed: 1,
                ..Default::default()
            }
        );
    }

    #[test]
    fn text_outside_the_archive_bound_is_refused_rather_than_truncated() {
        let (writer, _received) = stalled_writer(8);
        let metrics = Arc::clone(&writer.metrics);
        let mut capture =
            TranscriptCapture::new(&writer, "session", 1, 1).expect("a database identity binds");

        capture.record(HistoryRole::User, None, "1", "   ");
        capture.record(
            HistoryRole::User,
            None,
            "1",
            &"x".repeat(MAX_TRANSCRIPT_TEXT_BYTES + 1),
        );
        capture.record(HistoryRole::Assistant, None, "1", "kept");

        let counters = metrics.counters();
        assert_eq!(counters.dropped_text_out_of_bounds, 2);
        assert_eq!(
            counters.enqueued, 1,
            "the one archivable text is still offered to the writer"
        );
    }

    #[test]
    fn a_session_without_a_database_identity_is_never_bound_to_the_archive() {
        let (writer, _received) = stalled_writer(1);
        assert!(
            TranscriptCapture::new(&writer, "session", 0, 1).is_none(),
            "a session admitted without the database has no Device to attribute a record to"
        );
        assert!(TranscriptCapture::new(&writer, "session", 1, 0).is_none());
        assert!(TranscriptCapture::new(&writer, "session", -1, 1).is_none());
        assert!(TranscriptCapture::new(&writer, "session", 1, 1).is_some());
    }

    #[test]
    fn one_session_archives_under_a_sequence_that_only_moves_forward() {
        let (writer, mut received) = stalled_writer(8);
        let mut capture =
            TranscriptCapture::new(&writer, "session", 1, 1).expect("a database identity binds");

        capture.record(HistoryRole::User, Some(7), "1", "utterance");
        capture.record(HistoryRole::Assistant, Some(7), "1", "answer");

        assert_eq!(Arc::clone(&writer.metrics).counters().enqueued, 2);
        let mut stored = Vec::new();
        while let Ok(record) = received.try_recv() {
            stored.push((record.sequence, record.role, record.template_id));
        }
        assert_eq!(
            stored,
            vec![
                (1, HistoryRole::User, Some(7)),
                (2, HistoryRole::Assistant, Some(7))
            ],
            "a text the archive refuses is never offered, so it also never takes a number; only a \
             record the hand-off drops after that can leave a gap"
        );
    }

    #[test]
    fn the_writer_is_settled_only_once_it_holds_no_records() {
        assert!(
            HistoryWriterCounters {
                enqueued: 2,
                written: 1,
                dropped: 1,
                dropped_database: 1,
                ..Default::default()
            }
            .is_settled(),
            "a stored record and an archive-refused one together account for everything accepted"
        );
        assert!(
            !HistoryWriterCounters {
                enqueued: 2,
                written: 1,
                ..Default::default()
            }
            .is_settled(),
            "a record the writer still holds is not an outcome"
        );
        assert!(
            HistoryWriterCounters {
                enqueued: 2,
                written: 2,
                dropped: 1,
                dropped_queue_full: 1,
                ..Default::default()
            }
            .is_settled(),
            "a record the hand-off refused never reached the writer, so it does not unsettle it"
        );
    }

    #[test]
    fn a_role_has_exactly_the_two_words_the_archive_stores() {
        assert_eq!(HistoryRole::User.as_str(), "user");
        assert_eq!(HistoryRole::Assistant.as_str(), "assistant");
        assert_eq!(HistoryRole::parse("user"), Some(HistoryRole::User));
        assert_eq!(
            HistoryRole::parse("assistant"),
            Some(HistoryRole::Assistant)
        );
        assert_eq!(HistoryRole::parse("system"), None);
        assert_eq!(HistoryRole::parse("User"), None);
    }

    #[test]
    fn retention_deletes_only_what_is_older_than_its_utc_cutoff() {
        let now = 1_800_000_000_000_i64;
        assert_eq!(retention_cutoff(now, 1), now - MILLIS_PER_DAY);
        assert_eq!(retention_cutoff(now, 30), now - 30 * MILLIS_PER_DAY);
        assert_eq!(
            retention_cutoff(now, 365),
            now - 365 * MILLIS_PER_DAY,
            "the widest retention the configuration allows still has an absolute cutoff"
        );
        assert_eq!(
            retention_cutoff(MILLIS_PER_DAY, 2),
            -MILLIS_PER_DAY,
            "a clock behind the whole retention window yields a cutoff no stored row can precede"
        );
    }
}
