use super::*;
use crate::config::AppConfig;

impl Database {
    /// Scheduler eligibility is an authoritative read under the existing publication guard.
    pub(crate) async fn job_eligible(
        &self,
        config: &AppConfig,
        job: &JobRow,
        timestamp: i64,
        runtime_ready: bool,
    ) -> Result<(), JobError> {
        if timestamp >= job.deadline_at {
            return Err(JobError::DeadlineExceeded);
        }
        let _publication = self.tool_security.publication.read().await;
        let mut tx = self.pool().begin().await?;
        authorize(&mut tx, &job.requester()?).await?;
        let enabled: i64 =
            sqlx::query_scalar("SELECT enabled FROM background_settings WHERE singleton=1")
                .fetch_one(&mut *tx)
                .await?;
        if enabled != 1 {
            return Err(JobError::Disabled);
        }
        let unreaped: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM job_steps WHERE worker_instance_id IS NOT NULL AND attempt_state='unknown_outcome'"
        ).fetch_one(&mut *tx).await?;
        if unreaped != 0 {
            return Err(JobError::RuntimeUnavailable);
        }
        let snapshot: JobSnapshot =
            serde_json::from_str(&job.snapshot_json).map_err(|_| JobError::ConfigurationChanged)?;
        for expected in [&snapshot.orchestrator, &snapshot.worker] {
            if &providers::pin(&mut tx, config, &expected.reference).await? != expected {
                return Err(JobError::ConfigurationChanged);
            }
        }
        let choices: Vec<serde_json::Value> = serde_json::from_str(&job.plugin_choices_json)
            .map_err(|_| JobError::DatabaseUnavailable)?;
        for choice in choices {
            if choice["kind"] == "core" {
                crate::tools::core::decode(&choice).map_err(JobError::from_plugin)?;
                continue;
            }
            let installed: Option<i64> = sqlx::query_scalar(
                "SELECT 1 FROM plugin_installations WHERE plugin_id=? AND version=? AND package_digest=? AND installation_digest=? AND status='ready'"
            ).bind(choice["plugin_id"].as_str())
            .bind(choice["version"].as_str())
            .bind(choice["package_digest"].as_str())
            .bind(choice["installation_digest"].as_str())
            .fetch_optional(&mut *tx).await?;
            if installed.is_none() {
                return Err(JobError::CapabilityUnavailable);
            }
        }
        if !runtime_ready {
            return Err(JobError::RuntimeUnavailable);
        }
        tx.commit().await?;
        Ok(())
    }
}
