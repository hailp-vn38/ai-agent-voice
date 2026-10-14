use super::*;
use crate::services::background::plugins::FrozenChoice;
use serde_json::Value;

impl Database {
    #[cfg(test)]
    pub(crate) async fn test_plan_one_step(
        &self,
        job_id: &str,
        step_key: &str,
        choice: &FrozenChoice,
        arguments: &Value,
    ) -> Result<(), JobError> {
        let mut tx = self.pool().begin_with("BEGIN IMMEDIATE").await?;
        let row: JobRow = sqlx::query_as("SELECT * FROM jobs WHERE id=? AND status='queued'")
            .bind(job_id)
            .fetch_optional(&mut *tx)
            .await?
            .ok_or(JobError::Terminal)?;
        let choices: Vec<Value> = serde_json::from_str(&row.plugin_choices_json)
            .map_err(|_| JobError::DatabaseUnavailable)?;
        if !choices.iter().any(|v| {
            v["plugin_id"] == choice.plugin_id
                && v["operation"] == choice.operation
                && v["package_digest"] == choice.package_digest
                && v["installation_digest"] == choice.installation_digest
        }) {
            return Err(JobError::CapabilityUnavailable);
        }
        let now = crate::database::unix_seconds().ok_or(JobError::ClockUnavailable)?;
        sqlx::query("UPDATE jobs SET status='running',plan_revision=1,updated_at=? WHERE id=? AND status='queued'")
            .bind(now).bind(job_id).execute(&mut *tx).await?;
        sqlx::query("INSERT INTO job_steps(job_id,step_key,plan_revision,plugin_id,plugin_version,package_digest,installation_digest,operation,arguments_json,status,created_at,updated_at) VALUES(?, ?,1,?,?,?,?,?,?,'pending',?,?)")
            .bind(job_id).bind(step_key).bind(&choice.plugin_id).bind(&choice.version)
            .bind(&choice.package_digest).bind(&choice.installation_digest).bind(&choice.operation)
            .bind(serde_json::to_string(arguments).map_err(|_| JobError::InvalidArguments)?)
            .bind(now).bind(now).execute(&mut *tx).await?;
        tx.commit().await?;
        self.job_changes.publish_committed(job_id);
        Ok(())
    }

    pub(crate) async fn reserve_plugin_attempt(
        &self,
        job_id: &str,
        step_key: &str,
        attempt_id: &str,
    ) -> Result<(), JobError> {
        let now = crate::database::unix_seconds().ok_or(JobError::ClockUnavailable)?;
        let mut tx = self.pool().begin_with("BEGIN IMMEDIATE").await?;
        let updated = sqlx::query("UPDATE job_steps SET status='running',attempt=1,attempt_id=?,attempt_state='reserved',updated_at=? WHERE job_id=? AND step_key=? AND status='pending' AND attempt=0 AND EXISTS(SELECT 1 FROM jobs WHERE id=? AND status='running' AND deadline_at>?)")
            .bind(attempt_id).bind(now).bind(job_id).bind(step_key).bind(job_id).bind(now)
            .execute(&mut *tx).await?;
        if updated.rows_affected() != 1 {
            return Err(JobError::Terminal);
        }
        let step_path = format!(
            "$.steps.{}.attempts",
            serde_json::to_string(step_key).map_err(|_| JobError::InvalidArguments)?
        );
        let ledger=sqlx::query("UPDATE jobs SET budget_used_json=json_set(budget_used_json,'$.worker_attempts',COALESCE(json_extract(budget_used_json,'$.worker_attempts'),0)+1,?,1) WHERE id=? AND status='running' AND COALESCE(json_extract(budget_used_json,'$.worker_attempts'),0)<json_extract(budget_limit_json,'$.max_steps')")
            .bind(step_path).bind(job_id).execute(&mut *tx).await?;
        if ledger.rows_affected() != 1 {
            return Err(JobError::CapacityUnavailable);
        }
        events::append(
            &mut tx,
            job_id,
            "step_started",
            Some(step_key),
            &serde_json::json!({"attempt_state":"reserved"}),
        )
        .await?;
        tx.commit().await?;
        self.job_changes.publish_committed(job_id);
        Ok(())
    }

    pub(crate) async fn request_plugin_start(
        &self,
        job_id: &str,
        step_key: &str,
        attempt_id: &str,
        choice: &FrozenChoice,
    ) -> Result<(), JobError> {
        let mut tx = self.pool().begin_with("BEGIN IMMEDIATE").await?;
        let job: JobRow = sqlx::query_as("SELECT * FROM jobs WHERE id=?")
            .bind(job_id)
            .fetch_optional(&mut *tx)
            .await?
            .ok_or(JobError::NotFound)?;
        let now = crate::database::unix_seconds().ok_or(JobError::ClockUnavailable)?;
        if job.status != "running" || job.deadline_at <= now {
            return Err(JobError::DeadlineExceeded);
        }
        let choices: Vec<Value> = serde_json::from_str(&job.plugin_choices_json)
            .map_err(|_| JobError::DatabaseUnavailable)?;
        let selected = choices.iter().any(|value| {
            value["plugin_id"] == choice.plugin_id
                && value["version"] == choice.version
                && value["package_digest"] == choice.package_digest
                && value["installation_digest"] == choice.installation_digest
                && value["operation"] == choice.operation
        });
        let step: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM job_steps WHERE job_id=? AND step_key=? AND plugin_id=? AND plugin_version=? AND package_digest=? AND installation_digest=? AND operation=? AND attempt_id=? AND attempt_state='reserved'")
            .bind(job_id).bind(step_key).bind(&choice.plugin_id).bind(&choice.version)
            .bind(&choice.package_digest).bind(&choice.installation_digest).bind(&choice.operation)
            .bind(attempt_id).fetch_one(&mut *tx).await?;
        if !selected || step != 1 {
            return Err(JobError::CapabilityUnavailable);
        }
        authorize(&mut tx, &job.requester()?).await?;
        let enabled: i64 =
            sqlx::query_scalar("SELECT enabled FROM background_settings WHERE singleton=1")
                .fetch_one(&mut *tx)
                .await?;
        if enabled != 1 {
            return Err(JobError::Disabled);
        }
        let live: Option<i64> = sqlx::query_scalar(
            "SELECT 1 FROM plugin_installations WHERE plugin_id=? AND version=? AND package_digest=? AND installation_digest=? AND status='ready'",
        )
        .bind(&choice.plugin_id).bind(&choice.version)
        .bind(&choice.package_digest).bind(&choice.installation_digest)
        .fetch_optional(&mut *tx).await?;
        if live.is_none() {
            return Err(JobError::CapabilityUnavailable);
        }
        let updated = sqlx::query("UPDATE job_steps SET attempt_state='start_requested',worker_instance_id=?,updated_at=? WHERE job_id=? AND step_key=? AND status='running' AND attempt=1 AND attempt_id=? AND attempt_state='reserved'")
            .bind(format!("agent-job-{attempt_id}")).bind(now).bind(job_id).bind(step_key).bind(attempt_id)
            .execute(&mut *tx).await?;
        if updated.rows_affected() != 1 {
            return Err(JobError::Terminal);
        }
        tx.commit().await?;
        self.job_changes.publish_committed(job_id);
        Ok(())
    }

    pub(crate) async fn mark_plugin_started(
        &self,
        job_id: &str,
        step_key: &str,
        attempt_id: &str,
    ) -> Result<(), JobError> {
        let now = crate::database::unix_seconds().ok_or(JobError::ClockUnavailable)?;
        let updated = sqlx::query("UPDATE job_steps SET attempt_state='started',started_at=?,updated_at=? WHERE job_id=? AND step_key=? AND attempt_id=? AND status='running' AND attempt_state='start_requested' AND EXISTS(SELECT 1 FROM jobs WHERE id=? AND status='running')")
            .bind(now).bind(now).bind(job_id).bind(step_key).bind(attempt_id).bind(job_id)
            .execute(self.pool()).await?;
        if updated.rows_affected() != 1 {
            return Err(JobError::Terminal);
        }
        self.job_changes.publish_committed(job_id);
        Ok(())
    }

    pub(crate) async fn finish_plugin_failure(
        &self,
        job_id: &str,
        step_key: &str,
        attempt_id: &str,
        code: &str,
        unknown: bool,
        finish_job: bool,
    ) -> Result<(), JobError> {
        let now = crate::database::unix_seconds().ok_or(JobError::ClockUnavailable)?;
        let mut tx = self.pool().begin_with("BEGIN IMMEDIATE").await?;
        let updated = sqlx::query("UPDATE job_steps SET status='failed',attempt_state=?,error_code=?,exit_reason=?,finished_at=?,updated_at=? WHERE job_id=? AND step_key=? AND status='running' AND attempt_id=?")
            .bind(if unknown { "unknown_outcome" } else { "finished" })
            .bind(code).bind(code).bind(now).bind(now).bind(job_id).bind(step_key).bind(attempt_id)
            .execute(&mut *tx).await?;
        if updated.rows_affected() != 1 {
            return Err(JobError::Terminal);
        }
        let job = if finish_job {
            Some(sqlx::query("UPDATE jobs SET status='failed',error_code=?,updated_at=? WHERE id=? AND status='running'")
            .bind(code).bind(now).bind(job_id).execute(&mut *tx).await?
        )
        } else {
            None
        };
        if job.is_some_and(|job| job.rows_affected() != 1) {
            return Err(JobError::Terminal);
        }
        events::append(
            &mut tx,
            job_id,
            "step_finished",
            Some(step_key),
            &serde_json::json!({"status":"failed","error_code":code}),
        )
        .await?;
        if finish_job {
            events::append(
                &mut tx,
                job_id,
                "job_finished",
                None,
                &serde_json::json!({"status":"failed","error_code":code}),
            )
            .await?;
        }
        tx.commit().await?;
        self.job_changes.publish_committed(job_id);
        Ok(())
    }

    pub(crate) async fn finish_plugin_success(
        &self,
        job_id: &str,
        step_key: &str,
        attempt_id: &str,
        result: &Value,
        artifacts: &[(String, String, i64, String)],
        policy: (u64, bool),
    ) -> Result<(), JobError> {
        let now = crate::database::unix_seconds().ok_or(JobError::ClockUnavailable)?;
        let (max_artifact_bytes, finish_job) = policy;
        let encoded = serde_json::to_string(result).map_err(|_| JobError::InvalidArguments)?;
        if encoded.len() > 65536 {
            return Err(JobError::InvalidArguments);
        }
        let mut tx = self.pool().begin_with("BEGIN IMMEDIATE").await?;
        let existing_job: i64 = sqlx::query_scalar(
            "SELECT COALESCE(SUM(size_bytes),0) FROM job_artifacts WHERE job_id=?",
        )
        .bind(job_id)
        .fetch_one(&mut *tx)
        .await?;
        let existing_global: i64 =
            sqlx::query_scalar("SELECT COALESCE(SUM(size_bytes),0) FROM job_artifacts")
                .fetch_one(&mut *tx)
                .await?;
        let incoming = artifacts
            .iter()
            .try_fold(0u64, |sum, (_, _, size, _)| {
                u64::try_from(*size)
                    .ok()
                    .and_then(|value| sum.checked_add(value))
            })
            .ok_or(JobError::CapacityUnavailable)?;
        if (existing_job as u64).saturating_add(incoming) > max_artifact_bytes
            || (existing_global as u64).saturating_add(incoming) > 256 * 1024 * 1024
        {
            return Err(JobError::CapacityUnavailable);
        }
        let updated = sqlx::query("UPDATE job_steps SET status='succeeded',attempt_state='finished',result_json=?,finished_at=?,updated_at=? WHERE job_id=? AND step_key=? AND status='running' AND attempt_id=? AND EXISTS(SELECT 1 FROM jobs WHERE id=? AND status='running' AND deadline_at>?)")
            .bind(&encoded).bind(now).bind(now).bind(job_id).bind(step_key).bind(attempt_id).bind(job_id).bind(now)
            .execute(&mut *tx).await?;
        if updated.rows_affected() != 1 {
            return Err(JobError::DeadlineExceeded);
        }
        for (id, media_type, size, sha) in artifacts {
            sqlx::query("INSERT INTO job_artifacts(id,job_id,step_key,relative_path,media_type,size_bytes,sha256,created_at) VALUES(?,?,?,?,?,?,?,?)")
                .bind(id).bind(job_id).bind(step_key).bind(format!("{job_id}/{id}"))
                .bind(media_type).bind(size).bind(sha).bind(now).execute(&mut *tx).await?;
        }
        if finish_job {
            let final_job = sqlx::query("UPDATE jobs SET status='succeeded',result_json=?,updated_at=? WHERE id=? AND status='running' AND deadline_at>?")
                .bind(&encoded).bind(now).bind(job_id).bind(now).execute(&mut *tx).await?;
            if final_job.rows_affected() != 1 {
                return Err(JobError::Terminal);
            }
        }
        events::append(
            &mut tx,
            job_id,
            "step_finished",
            Some(step_key),
            &serde_json::json!({"status":"succeeded"}),
        )
        .await?;
        if finish_job {
            events::append(
                &mut tx,
                job_id,
                "job_finished",
                None,
                &serde_json::json!({"status":"succeeded"}),
            )
            .await?;
        }
        tx.commit().await?;
        self.job_changes.publish_committed(job_id);
        Ok(())
    }
}
