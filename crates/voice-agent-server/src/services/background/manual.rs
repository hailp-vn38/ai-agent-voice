//! One durable Plugin Step, no provider pins and no Agent Session.
use super::{JobEngine, JobError};
use std::sync::Arc;
use tokio_util::sync::CancellationToken;

pub(super) async fn execute(engine: Arc<JobEngine>, id: String, cancellation: CancellationToken) {
    use futures_util::FutureExt;
    let outcome = std::panic::AssertUnwindSafe(async {
        if cancellation.is_cancelled() {
            return Err(JobError::Cancelled);
        }
        engine.database.start_plugin_operation(&id).await?;
        let job = engine.get(&id).await?;
        if job.execution_mode != "plugin_operation_v3" {
            return Err(JobError::InvalidPlan);
        }
        let raw = job
            .plugin_choices
            .as_array()
            .and_then(|choices| (choices.len() == 1).then(|| &choices[0]))
            .ok_or(JobError::InvalidPlan)?;
        let choice = super::plugins::decode_choice(raw).map_err(JobError::from_plugin)?;
        let step = job
            .steps
            .iter()
            .find(|step| step.step_key == "operation_0")
            .ok_or(JobError::InvalidPlan)?;
        engine
            .execute_plugin_step_cancel(
                &id,
                "operation_0",
                &choice,
                step.arguments.clone(),
                cancellation.clone(),
            )
            .await?;
        Ok::<(), JobError>(())
    })
    .catch_unwind()
    .await
    .unwrap_or(Err(JobError::RuntimeUnavailable));
    if let Err(error) = outcome {
        let code = error.code();
        // Result publication is mode-aware and terminalizes the Step+Job in
        // one transaction; here we only handle pre-Step or transient failure.
        // Three attempts only: a persistent SQLite failure must not keep an
        // active Job task alive indefinitely and starve scheduler capacity.
        // If durable publication cannot be acknowledged, stop claiming work;
        // the next boot's existing recover_jobs() will fail the uncertain Job.
        let mut committed = false;
        for attempt in 0..3 {
            match tokio::time::timeout(
                std::time::Duration::from_secs(5),
                engine
                    .database
                    .finish_job(&engine.config, &id, "failed", Some(code), None),
            )
            .await
            {
                Ok(Ok(())) | Ok(Err(JobError::Terminal)) => {
                    committed = true;
                    break;
                }
                result => {
                    tracing::warn!(
                        event="plugin_operation_terminal_checkpoint_failed",
                        job_id=%id,
                        attempt=attempt + 1,
                        outcome=?result,
                    );
                    if !engine.running.load(std::sync::atomic::Ordering::Acquire) {
                        break;
                    }
                    if attempt < 2 {
                        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                    }
                }
            }
        }
        if !committed {
            tracing::error!(
                event="plugin_operation_terminal_checkpoint_exhausted",
                job_id=%id,
                "Manual Job terminal state requires startup recovery"
            );
            engine
                .running
                .store(false, std::sync::atomic::Ordering::Release);
        }
    }
    engine
        .cancellations
        .lock()
        .expect("Job cancellations")
        .remove(&id);
    engine.wake.notify_one();
}
