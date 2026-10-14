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
    retention_backlog: bool,
) {
    let mut tick = tokio::time::interval(Duration::from_millis(
        engine.config.background.limits.scheduler_scan_ms,
    ));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let maintenance = tokio::time::sleep(Duration::from_secs(if retention_backlog {
        1
    } else {
        3600
    }));
    tokio::pin!(maintenance);
    let mut jobs = tokio::task::JoinSet::new();
    loop {
        tokio::select! {
            biased;
            _=stop.cancelled()=>break,
            _=stopping.cancelled()=>break,
            _=tick.tick()=>{},
            _=&mut maintenance=>{
                let delay = match now() {
                    Ok(timestamp) => match super::artifacts::expire_terminal(&engine.database, timestamp).await {
                        Ok(100) => 1,
                        Ok(_) => 3600,
                        Err(error) => { tracing::warn!(code=%error, "Background retention failed"); 60 }
                    },
                    Err(error) => { tracing::warn!(code=%error, "Background retention clock failed"); 60 }
                };
                maintenance.as_mut().reset(tokio::time::Instant::now()+Duration::from_secs(delay));
            },
            _=engine.wake.notified()=>{},
            Some(result)=jobs.join_next()=>{if let Err(error)=result { tracing::warn!(event="background_task_failed",%error); }},
        }
        if engine.models.is_some() {
            match engine.database.queued_jobs().await {
                Ok(queued) => {
                    for job in queued {
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
                                jobs.spawn(super::orchestration::execute(
                                    engine.clone(),
                                    job.id,
                                    cancellation,
                                ));
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
    let unreaped:i64=sqlx::query_scalar("SELECT COUNT(*) FROM job_steps WHERE worker_instance_id IS NOT NULL AND attempt_state='unknown_outcome'").fetch_one(&mut *tx).await?;
    if unreaped != 0 {
        return Err(JobError::RuntimeUnavailable);
    }
    let snapshot: JobSnapshot =
        serde_json::from_str(&job.snapshot_json).map_err(|_| JobError::ConfigurationChanged)?;
    for previous in [&snapshot.orchestrator, &snapshot.worker] {
        if &providers::pin(&mut tx, &engine.config, &previous.reference).await? != previous {
            return Err(JobError::ConfigurationChanged);
        }
    }
    let choices: Vec<serde_json::Value> = serde_json::from_str(&job.plugin_choices_json)
        .map_err(|_| JobError::DatabaseUnavailable)?;
    for choice in choices {
        let installed: Option<i64> = sqlx::query_scalar(
            "SELECT 1 FROM plugin_installations WHERE plugin_id=? AND version=? AND package_digest=? AND installation_digest=? AND status='ready'",
        )
        .bind(choice["plugin_id"].as_str())
        .bind(choice["version"].as_str())
        .bind(choice["package_digest"].as_str())
        .bind(choice["installation_digest"].as_str())
        .fetch_optional(&mut *tx)
        .await?;
        if installed.is_none() {
            return Err(JobError::CapabilityUnavailable);
        }
    }
    if !engine.runtime_ready() {
        return Err(JobError::RuntimeUnavailable);
    }
    tx.commit().await?;
    Ok(())
}
