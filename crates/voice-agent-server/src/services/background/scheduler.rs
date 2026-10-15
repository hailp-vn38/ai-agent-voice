use super::{JobEngine, JobError, engine::now};
use crate::database::jobs::JobRow;
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
    let mut jobs = tokio::task::JoinSet::new();
    loop {
        tokio::select! {
            biased;
            _=stop.cancelled()=>break,
            _=stopping.cancelled()=>break,
            _=tick.tick()=>{},
            _=engine.wake.notified()=>{},
            Some(result)=jobs.join_next()=>{if let Err(error)=result { tracing::warn!(event="background_task_failed",%error); }},
        }
        {
            match engine.database.queued_jobs().await {
                Ok(queued) => {
                    for job in queued {
                        // Foundation test harnesses deliberately have no LLM runtime.
                        // Only plugin-only Jobs can run without models; do not steal
                        // historical Agent Jobs from their deterministic test setup.
                        if job.execution_mode != "plugin_operation_v3" && engine.models.is_none() {
                            continue;
                        }
                        if jobs.len() >= engine.config.background.limits.max_active_jobs as usize {
                            break;
                        }
                        if eligible(
                            &engine,
                            &job,
                            match now() {
                                Ok(now) => now,
                                Err(_) => continue,
                            },
                        )
                        .await
                        .is_err()
                        {
                            continue;
                        }
                        match engine
                            .database
                            .claim_job(&job.id, engine.config.background.limits.max_active_jobs)
                            .await
                        {
                            Ok(true) => {
                                tracing::info!(event="background_job_claimed", job_id=%job.id, queue_wait_ms=now().unwrap_or(job.created_at).saturating_sub(job.created_at).saturating_mul(1000));
                                let cancellation = CancellationToken::new();
                                engine
                                    .cancellations
                                    .lock()
                                    .expect("Job cancellations")
                                    .insert(job.id.clone(), cancellation.clone());
                                let plugin_only = job.execution_mode == "plugin_operation_v3";
                                let engine = engine.clone();
                                jobs.spawn(async move {
                                    if plugin_only {
                                        super::manual::execute(engine, job.id, cancellation).await;
                                    } else {
                                        super::orchestration::execute(engine, job.id, cancellation)
                                            .await;
                                    }
                                });
                            }
                            Ok(false) => {}
                            Err(error) => tracing::warn!(code=%error,"Background claim failed"),
                        }
                    }
                }
                Err(error) => tracing::warn!(code=%error,"Background queue failed"),
            }
        }
        let active: Vec<String> = engine
            .cancellations
            .lock()
            .expect("Job cancellations")
            .keys()
            .cloned()
            .collect();
        for id in active {
            if let Ok(row) = engine.database.job_row(&id).await {
                let reason = if row.status == "cancelling" {
                    Some(JobError::Cancelled)
                } else {
                    eligible(
                        &engine,
                        &row,
                        match now() {
                            Ok(now) => now,
                            Err(_) => continue,
                        },
                    )
                    .await
                    .err()
                };
                if reason.is_some_and(|reason| reason != JobError::DatabaseUnavailable)
                    && let Some(token) = engine
                        .cancellations
                        .lock()
                        .expect("Job cancellations")
                        .get(&id)
                {
                    token.cancel();
                }
            }
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
    for token in engine
        .cancellations
        .lock()
        .expect("Job cancellations")
        .values()
    {
        token.cancel();
    }
    while jobs.join_next().await.is_some() {}
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
pub(super) async fn eligible(
    engine: &JobEngine,
    job: &JobRow,
    timestamp: i64,
) -> Result<(), JobError> {
    engine
        .database
        .job_eligible(
            &engine.config,
            job,
            timestamp,
            if job.execution_mode == "plugin_operation_v3" {
                engine.running.load(Ordering::Acquire)
            } else {
                engine.runtime_ready()
            },
        )
        .await
}
