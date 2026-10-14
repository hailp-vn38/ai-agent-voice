use super::*;

impl Database {
    /// No disappeared in-memory ACK is needed. Unknown attempts and worker identities
    /// remain durable even after the Job releases queue capacity.
    pub(crate) async fn recover_jobs(&self, now: i64) -> Result<(), JobError> {
        let _publication = self.tool_security.publication.write().await;
        let mut tx = self.pool().begin_with("BEGIN IMMEDIATE").await?;
        let recovering: Vec<(String, String)> = sqlx::query_as(
            "SELECT id,status FROM jobs WHERE status IN ('planning','running','cancelling')",
        )
        .fetch_all(&mut *tx)
        .await?;
        sqlx::query("UPDATE job_steps SET attempt_state=CASE WHEN attempt_state IN ('start_requested','started','unknown_outcome') THEN 'unknown_outcome' ELSE attempt_state END,status=CASE WHEN attempt_state IN ('start_requested','started','unknown_outcome') THEN 'failed' WHEN (SELECT status FROM jobs WHERE id=job_id)='cancelling' THEN 'cancelled' ELSE 'skipped' END,error_code=CASE WHEN attempt_state IN ('start_requested','started','unknown_outcome') THEN 'unknown_outcome' ELSE 'interrupted' END,finished_at=?,updated_at=? WHERE job_id IN (SELECT id FROM jobs WHERE status IN ('planning','running','cancelling')) AND status IN ('pending','running')")
            .bind(now).bind(now).execute(&mut *tx).await?;
        sqlx::query("UPDATE jobs SET status=CASE WHEN status='cancelling' THEN 'cancelled' ELSE 'failed' END,error_code=CASE WHEN status='cancelling' THEN 'cancelled_after_restart' ELSE 'interrupted' END,updated_at=? WHERE status IN ('planning','running','cancelling')")
            .bind(now).execute(&mut *tx).await?;
        let interrupted:Vec<(String,String)>=sqlx::query_as(
            "SELECT job_id,agent_run_id FROM job_agent_runs WHERE status IN ('queued','running') AND job_id IN (SELECT id FROM jobs WHERE status IN ('failed','cancelled'))"
        ).fetch_all(&mut *tx).await?;
        sqlx::query("UPDATE job_agent_runs SET status='interrupted',error_code='interrupted',finished_at=? WHERE status IN ('queued','running') AND job_id IN (SELECT id FROM jobs WHERE status IN ('failed','cancelled'))")
            .bind(now).execute(&mut *tx).await?;
        for (job_id, run_id) in interrupted {
            events::append(&mut tx,&job_id,"agent_run_finished",None,
                &serde_json::json!({"agent_run_id":run_id,"status":"interrupted","error_code":"interrupted"})).await?;
        }
        for (id, prior) in &recovering {
            let cancelled = prior == "cancelling";
            events::append(&mut tx,id,"job_finished",None,&serde_json::json!({"status":if cancelled {"cancelled"}else{"failed"},"error_code":if cancelled {"cancelled_after_restart"}else{"interrupted"}})).await?;
        }
        tx.commit().await?;
        for (id, _) in &recovering {
            self.job_changes.publish_committed(id);
        }
        Ok(())
    }
    pub(crate) async fn fail_queued_job(
        &self,
        id: &str,
        reason: JobError,
        now: i64,
    ) -> Result<(), JobError> {
        let mut tx = self.pool().begin_with("BEGIN IMMEDIATE").await?;
        let update=sqlx::query("UPDATE jobs SET status='failed',error_code=?,updated_at=? WHERE id=? AND status='queued'").bind(reason.code()).bind(now).bind(id).execute(&mut *tx).await?;
        if update.rows_affected() == 1 {
            events::append(
                &mut tx,
                id,
                "job_finished",
                None,
                &serde_json::json!({"status":"failed","error_code":reason.code()}),
            )
            .await?;
        }
        tx.commit().await?;
        if update.rows_affected() == 1 {
            self.job_changes.publish_committed(id);
        }
        Ok(())
    }
    pub(crate) async fn observe_queued_job(&self, id: &str, now: i64) -> Result<(), JobError> {
        let updated = sqlx::query("UPDATE jobs SET scheduler_seen_at=? WHERE id=? AND status='queued' AND scheduler_seen_at IS NULL")
            .bind(now).bind(id).execute(self.pool()).await?;
        if updated.rows_affected() == 1 {
            self.job_changes.publish_committed(id);
        }
        Ok(())
    }
    pub(crate) async fn cancel_job(&self, id: &str, now: i64) -> Result<JobView, JobError> {
        let _publication = self.tool_security.publication.write().await;
        let mut tx = self.pool().begin_with("BEGIN IMMEDIATE").await?;
        let status: Option<String> = sqlx::query_scalar("SELECT status FROM jobs WHERE id=?")
            .bind(id)
            .fetch_optional(&mut *tx)
            .await?;
        let mut changed = false;
        match status.as_deref() {
            None => return Err(JobError::NotFound),
            Some("queued") => {
                // P0 queued work has no worker to acknowledge; both transitions commit together.
                sqlx::query("UPDATE jobs SET status='cancelling',updated_at=? WHERE id=? AND status='queued'").bind(now).bind(id).execute(&mut *tx).await?;
                sqlx::query("UPDATE jobs SET status='cancelled',updated_at=? WHERE id=? AND status='cancelling'").bind(now).bind(id).execute(&mut *tx).await?;
                events::append(
                    &mut tx,
                    id,
                    "job_finished",
                    None,
                    &serde_json::json!({"status":"cancelled","error_code":"cancelled"}),
                )
                .await?;
                changed = true;
            }
            Some("cancelling") => {}
            Some("planning" | "running") => {
                sqlx::query("UPDATE jobs SET status='cancelling',updated_at=? WHERE id=? AND status IN ('planning','running')").bind(now).bind(id).execute(&mut *tx).await?;
                changed = true;
            }
            _ => return Err(JobError::Terminal),
        }
        tx.commit().await?;
        if changed {
            self.job_changes.publish_committed(id);
        }
        self.job_view(id).await
    }
}
