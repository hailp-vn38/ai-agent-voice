use super::{JobEngine, JobError};
use std::sync::Arc;
use tokio_util::sync::CancellationToken;

pub(super) async fn execute(engine: Arc<JobEngine>, id: String, cancellation: CancellationToken) {
    use futures_util::FutureExt;
    let outcome = std::panic::AssertUnwindSafe(async {
        super::agent_loop::run_root(&engine, &id, cancellation.clone()).await
    })
    .catch_unwind()
    .await
    .unwrap_or(Err(JobError::RuntimeUnavailable));
    if let Err(error) = outcome {
        let code = if cancellation.is_cancelled() && error == JobError::Cancelled {
            if let Ok(row) = engine.database.job_row(&id).await {
                super::scheduler::eligible(&engine, &row, super::engine::now().unwrap_or(i64::MAX))
                    .await
                    .err()
                    .unwrap_or(JobError::Cancelled)
                    .code()
            } else {
                "interrupted"
            }
        } else {
            error.code()
        };
        loop {
            match tokio::time::timeout(
                std::time::Duration::from_secs(5),
                engine
                    .database
                    .finish_job(&engine.config, &id, "failed", Some(code), None),
            )
            .await
            .unwrap_or(Err(JobError::DatabaseUnavailable))
            {
                Ok(()) | Err(JobError::Terminal) => break,
                Err(error) => {
                    tracing::warn!(event="background_terminal_checkpoint_failed", job_id=%id, code=%error.code());
                    // Retain the execution owner while retrying only SQLite publication.
                    if !engine.running.load(std::sync::atomic::Ordering::Acquire) {
                        tracing::warn!(event="background_terminal_checkpoint_deferred", job_id=%id, "Restart recovery will terminalize this Job");
                        break;
                    }
                    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                }
            }
        }
    }
    engine
        .cancellations
        .lock()
        .expect("Job cancellations")
        .remove(&id);
    engine.wake.notify_one();
}
