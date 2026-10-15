use super::*;
use sqlx::{Sqlite, Transaction};

/// One FK-safe deletion sequence for Admin deletion and bounded retention.
async fn delete_rows(tx: &mut Transaction<'_, Sqlite>, id: &str) -> Result<(), JobError> {
    for statement in [
        "DELETE FROM job_events WHERE job_id=?",
        "DELETE FROM job_artifacts WHERE job_id=?",
        "DELETE FROM job_steps WHERE job_id=?",
        "DELETE FROM job_agent_runs WHERE job_id=? AND parent_agent_run_id IS NOT NULL",
        "DELETE FROM job_agent_runs WHERE job_id=?",
        "DELETE FROM job_agent_messages WHERE session_id IN (SELECT session_id FROM job_agent_sessions WHERE job_id=?)",
        "DELETE FROM job_agent_sessions WHERE job_id=? AND parent_session_id IS NOT NULL",
        "DELETE FROM job_agent_sessions WHERE job_id=?",
        "DELETE FROM jobs WHERE id=?",
    ] {
        sqlx::query(statement).bind(id).execute(&mut **tx).await?;
    }
    Ok(())
}

impl Database {
    pub(crate) async fn delete_terminal_job(&self, id: &str) -> Result<(), JobError> {
        let mut tx = self.pool().begin_with("BEGIN IMMEDIATE").await?;
        let status: Option<String> = sqlx::query_scalar("SELECT status FROM jobs WHERE id=?")
            .bind(id)
            .fetch_optional(&mut *tx)
            .await?;
        match status.as_deref() {
            None => return Err(JobError::NotFound),
            Some("succeeded" | "partial_success" | "failed" | "cancelled") => {}
            _ => return Err(JobError::DeleteConflict),
        }
        let retries: Vec<String> = sqlx::query_scalar(
            "UPDATE jobs SET retry_of_job_id=NULL WHERE retry_of_job_id=? RETURNING id",
        )
        .bind(id)
        .fetch_all(&mut *tx)
        .await?;
        delete_rows(&mut tx, id).await?;
        tx.commit().await?;
        self.job_changes.publish_committed(id);
        for retry in retries {
            self.job_changes.publish_committed(&retry);
        }
        Ok(())
    }

    /// Returns deleted Job IDs for scoped filesystem cleanup after the DB commit.
    #[cfg(test)]
    pub(crate) async fn expire_terminal_jobs(&self, cutoff: i64) -> Result<Vec<String>, JobError> {
        let mut tx = self.pool().begin_with("BEGIN IMMEDIATE").await?;
        let ids: Vec<String> = sqlx::query_scalar(
            "SELECT id FROM jobs WHERE status IN ('succeeded','partial_success','failed','cancelled') AND updated_at<? AND NOT EXISTS (SELECT 1 FROM jobs AS child WHERE child.retry_of_job_id=jobs.id) ORDER BY updated_at,id LIMIT 100",
        )
        .bind(cutoff)
        .fetch_all(&mut *tx)
        .await?;
        for id in &ids {
            delete_rows(&mut tx, id).await?;
        }
        tx.commit().await?;
        for id in &ids {
            self.job_changes.publish_committed(id);
        }
        Ok(ids)
    }
}
