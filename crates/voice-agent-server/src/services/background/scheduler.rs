use super::{JobEngine, JobError, JobSnapshot, engine::now, providers};
use crate::database::jobs::{JobRow, authorize};
use std::{
    fs::File,
    sync::{Arc, atomic::Ordering},
    time::Duration,
};
use tokio_util::sync::CancellationToken;

pub(super) async fn run(
    engine: Arc<JobEngine>,
    stop: CancellationToken,
    stopping: CancellationToken,
    _deployment_lock: File,
) {
    let mut tick = tokio::time::interval(Duration::from_millis(
        engine.config.background.limits.scheduler_scan_ms,
    ));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        tokio::select! {
            biased;
            _=stop.cancelled()=>break,
            _=stopping.cancelled()=>break,
            _=tick.tick()=>{},
            _=engine.wake.notified()=>{},
        }
        // Cancellation also bounds waiting for publication/SQLite. Dropping SQLx's
        // transaction rolls it back; this task never owns external side effects.
        tokio::select! {
            _=stop.cancelled()=>break,
            _=stopping.cancelled()=>break,
            result=scan(&engine)=>if let Err(cause)=result {tracing::warn!(code=%cause,"Background scan failed");},
        }
    }
    engine.running.store(false, Ordering::Release);
    // Keep the deployment File alive until every submit that crossed admission
    // has committed or rolled back. A successor must never race an old writer.
    let _drained = engine.submission_gate.write().await;
}
pub(super) async fn scan(engine: &JobEngine) -> Result<(), JobError> {
    let timestamp = now()?;
    for job in engine.database.queued_jobs().await? {
        match eligible(engine, &job, timestamp).await {
            Ok(()) => {
                engine
                    .database
                    .observe_queued_job(&job.id, timestamp)
                    .await?
            }
            Err(JobError::DatabaseUnavailable) => return Err(JobError::DatabaseUnavailable),
            Err(reason) => {
                engine
                    .database
                    .fail_queued_job(&job.id, reason, timestamp)
                    .await?
            }
        }
    }
    Ok(())
}
async fn eligible(engine: &JobEngine, job: &JobRow, timestamp: i64) -> Result<(), JobError> {
    if timestamp >= job.deadline_at {
        return Err(JobError::DeadlineExceeded);
    }
    let _publication = engine.database.tool_security.publication.read().await;
    let mut tx = engine.database.pool().begin().await?;
    authorize(&mut tx, &job.requester()?).await?;
    let enabled: i64 =
        sqlx::query_scalar("SELECT enabled FROM background_settings WHERE singleton=1")
            .fetch_one(&mut *tx)
            .await?;
    if enabled != 1 {
        return Err(JobError::Disabled);
    }
    let snapshot: JobSnapshot =
        serde_json::from_str(&job.snapshot_json).map_err(|_| JobError::ConfigurationChanged)?;
    for previous in [&snapshot.orchestrator, &snapshot.worker] {
        if &providers::pin(&mut tx, &engine.config, &previous.reference).await? != previous {
            return Err(JobError::ConfigurationChanged);
        }
    }
    if job.plugin_choices_json != "[]" {
        return Err(JobError::CapabilityUnavailable);
    }
    if !engine.runtime_ready() {
        return Err(JobError::RuntimeUnavailable);
    }
    tx.commit().await?;
    Ok(())
}
