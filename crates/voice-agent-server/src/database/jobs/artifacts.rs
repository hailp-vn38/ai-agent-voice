//! Artifact metadata and plugin execution inputs owned by the database.
use crate::{database::Database, services::background::JobError};
use serde_json::Value;

impl Database {
    pub(crate) async fn job_uncertain_plugin_attempts(
        &self,
        job_id: &str,
    ) -> Result<Vec<(String, String)>, JobError> {
        // Recovery may clear worker pins while an external attempt remains uncertain.
        Ok(sqlx::query_as(
            "SELECT step_key,attempt_id FROM job_steps WHERE job_id=? AND installation_digest<>'' AND attempt_id IS NOT NULL AND (worker_instance_id IS NOT NULL OR attempt_state IN ('start_requested','started','unknown_outcome'))",
        )
        .bind(job_id)
        .fetch_all(self.pool())
        .await?)
    }

    pub(crate) async fn job_artifact_record(
        &self,
        job_id: &str,
        artifact_id: &str,
    ) -> Result<Option<(String, i64, String)>, JobError> {
        let row: Option<(String, i64, String)> = sqlx::query_as(
            "SELECT relative_path,size_bytes,sha256 FROM job_artifacts WHERE job_id=? AND id=?",
        )
        .bind(job_id)
        .bind(artifact_id)
        .fetch_optional(self.pool())
        .await?;
        Ok(row)
    }
    pub(crate) async fn validate_plugin_step_arguments(
        &self,
        job_id: &str,
        step_key: &str,
        arguments: &Value,
    ) -> Result<(), JobError> {
        let stored: String = sqlx::query_scalar("SELECT arguments_json FROM job_steps WHERE job_id=? AND step_key=? AND status='pending'").bind(job_id).bind(step_key).fetch_optional(self.pool()).await?.ok_or(JobError::Terminal)?;
        if serde_json::from_str::<Value>(&stored).map_err(|_| JobError::DatabaseUnavailable)?
            != *arguments
        {
            return Err(JobError::InvalidPlan);
        }
        Ok(())
    }
    pub(crate) async fn clear_plugin_worker(
        &self,
        job_id: &str,
        step_key: &str,
        attempt: &str,
    ) -> Result<(), JobError> {
        sqlx::query("UPDATE job_steps SET worker_instance_id=NULL WHERE job_id=? AND step_key=? AND attempt_id=?")
            .bind(job_id).bind(step_key).bind(attempt).execute(self.pool()).await?;
        Ok(())
    }
    pub(crate) async fn job_artifact_exists(
        &self,
        job_id: &str,
        artifact_id: &str,
    ) -> Result<bool, JobError> {
        let retained: i64 = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM job_artifacts WHERE job_id=? AND id=?)",
        )
        .bind(job_id)
        .bind(artifact_id)
        .fetch_one(self.pool())
        .await?;
        Ok(retained != 0)
    }
    /// Clear terminal bookkeeping only; the Runner still owns possibly live process groups.
    pub(crate) async fn clear_terminal_plugin_workers(
        &self,
    ) -> Result<(), crate::services::background::plugins::PluginError> {
        sqlx::query("UPDATE job_steps SET worker_instance_id=NULL WHERE worker_instance_id IS NOT NULL AND job_id IN (SELECT id FROM jobs WHERE status IN ('failed','cancelled','succeeded','partial_success'))")
            .execute(self.pool()).await?;
        Ok(())
    }
}
