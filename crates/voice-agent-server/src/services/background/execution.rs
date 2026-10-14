use super::{
    JobEngine, JobError, JobView,
    plugins::{FrozenChoice, Invocation, PluginError},
};
use serde_json::Value;
use sha2::{Digest, Sha256};
use tokio::io::AsyncWriteExt;

impl JobEngine {
    pub(crate) async fn open_artifact(
        &self,
        job_id: &str,
        artifact_id: &str,
    ) -> Result<(tokio::fs::File, u64), JobError> {
        if uuid::Uuid::parse_str(job_id).is_err() || uuid::Uuid::parse_str(artifact_id).is_err() {
            return Err(JobError::InvalidArguments);
        }
        let row: Option<(String, i64, String)> = sqlx::query_as(
            "SELECT relative_path,size_bytes,sha256 FROM job_artifacts WHERE job_id=? AND id=?",
        )
        .bind(job_id)
        .bind(artifact_id)
        .fetch_optional(self.database.pool())
        .await?;
        let (relative, size, sha) = row.ok_or(JobError::NotFound)?;
        if relative != format!("{job_id}/{artifact_id}") || size < 0 {
            return Err(JobError::DatabaseUnavailable);
        }
        let file = super::artifacts::open_checked(
            super::artifacts::root(&self.database)?.join(relative),
            self.config.background.limits.artifact_bytes_per_job,
            size as u64,
            sha,
        )
        .await
        .map_err(|_| JobError::DatabaseUnavailable)?;
        Ok((tokio::fs::File::from_std(file), size as u64))
    }
    pub(crate) async fn read_artifact(
        &self,
        job_id: &str,
        artifact_id: &str,
    ) -> Result<Vec<u8>, JobError> {
        if uuid::Uuid::parse_str(job_id).is_err() || uuid::Uuid::parse_str(artifact_id).is_err() {
            return Err(JobError::InvalidArguments);
        }
        let row: Option<(String, i64, String)> = sqlx::query_as(
            "SELECT relative_path,size_bytes,sha256 FROM job_artifacts WHERE job_id=? AND id=?",
        )
        .bind(job_id)
        .bind(artifact_id)
        .fetch_optional(self.database.pool())
        .await?;
        let (relative, size, sha) = row.ok_or(JobError::NotFound)?;
        if relative != format!("{job_id}/{artifact_id}")
            || size < 0
            || size as u64 > self.config.background.limits.artifact_bytes_per_job
        {
            return Err(JobError::DatabaseUnavailable);
        }
        let root = super::artifacts::root(&self.database)?;
        let path = root.join(relative);
        let bytes = super::artifacts::read_regular(
            path,
            self.config.background.limits.artifact_bytes_per_job,
        )
        .await
        .map_err(|_| JobError::NotFound)?;
        let checked: String = Sha256::digest(&bytes)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        if bytes.len() != size as usize || checked != sha {
            return Err(JobError::DatabaseUnavailable);
        }
        Ok(bytes)
    }

    /// P3's planner supplies persisted Steps. P2's test harness supplies exactly one.
    pub(crate) async fn execute_plugin_step(
        &self,
        job_id: &str,
        step_key: &str,
        choice: &FrozenChoice,
        arguments: Value,
    ) -> Result<JobView, JobError> {
        self.execute_plugin_step_cancel(
            job_id,
            step_key,
            choice,
            arguments,
            tokio_util::sync::CancellationToken::new(),
        )
        .await
    }
    pub(crate) async fn execute_plugin_step_cancel(
        &self,
        job_id: &str,
        step_key: &str,
        choice: &FrozenChoice,
        arguments: Value,
        cancellation: tokio_util::sync::CancellationToken,
    ) -> Result<JobView, JobError> {
        let stored: String = sqlx::query_scalar("SELECT arguments_json FROM job_steps WHERE job_id=? AND step_key=? AND status='pending'").bind(job_id).bind(step_key).fetch_optional(self.database.pool()).await?.ok_or(JobError::Terminal)?;
        if serde_json::from_str::<Value>(&stored).map_err(|_| JobError::DatabaseUnavailable)?
            != arguments
        {
            return Err(JobError::InvalidPlan);
        }
        let refs: String = sqlx::query_scalar(
            "SELECT input_refs_json FROM job_steps WHERE job_id=? AND step_key=?",
        )
        .bind(job_id)
        .bind(step_key)
        .fetch_one(self.database.pool())
        .await?;
        let refs: Vec<String> =
            serde_json::from_str(&refs).map_err(|_| JobError::DatabaseUnavailable)?;
        let mut inputs = serde_json::Map::new();
        for key in refs {
            let raw:Option<String>=sqlx::query_scalar("SELECT result_json FROM job_steps WHERE job_id=? AND step_key=? AND status='succeeded'").bind(job_id).bind(&key).fetch_optional(self.database.pool()).await?.flatten();
            inputs.insert(
                key,
                serde_json::from_str(&raw.ok_or(JobError::InvalidPlan)?)
                    .map_err(|_| JobError::DatabaseUnavailable)?,
            );
        }
        let inputs = Value::Object(inputs);
        if inputs.to_string().len() > 32768 {
            return Err(JobError::CapacityUnavailable);
        }
        let attempt = uuid::Uuid::new_v4().to_string();
        let deadline_at = self.get(job_id).await?.deadline_at;
        self.database
            .reserve_plugin_attempt(job_id, step_key, &attempt)
            .await?;
        let result = self
            .plugins
            .invoke(
                Invocation {
                    inputs,
                    choice,
                    arguments,
                    job_id,
                    attempt_id: &attempt,
                    timeout: std::time::Duration::from_secs(
                        self.config.background.limits.step_timeout_seconds.into(),
                    ),
                    deadline_at,
                    cancellation,
                },
                || async {
                    self.database
                        .request_plugin_start(job_id, step_key, &attempt, choice)
                        .await
                        .map_err(PluginError::JobRejected)
                },
                || async {
                    self.database
                        .mark_plugin_started(job_id, step_key, &attempt)
                        .await
                        .map_err(PluginError::JobRejected)
                },
            )
            .await;
        match result {
            Ok(output) => {
                sqlx::query("UPDATE job_steps SET worker_instance_id=NULL WHERE job_id=? AND step_key=? AND attempt_id=?").bind(job_id).bind(step_key).bind(&attempt).execute(self.database.pool()).await?;
                let root = super::artifacts::root(&self.database)?.join(job_id);
                tokio::fs::create_dir_all(&root)
                    .await
                    .map_err(|_| JobError::DatabaseUnavailable)?;
                let mut published = Vec::new();
                let mut staged_files = Vec::new();
                let mut records = Vec::new();
                let publication = async {
                    let mut total = 0u64;
                    for artifact in output.artifacts {
                        total = total
                            .checked_add(artifact.bytes.len() as u64)
                            .ok_or(JobError::CapacityUnavailable)?;
                        if total > self.config.background.limits.artifact_bytes_per_job {
                            return Err(JobError::CapacityUnavailable);
                        }
                        let id = uuid::Uuid::new_v4().to_string();
                        let target = root.join(&id);
                        let staged = root.join(format!(".{id}.staging"));
                        let sha: String = Sha256::digest(&artifact.bytes)
                            .iter()
                            .map(|byte| format!("{byte:02x}"))
                            .collect();
                        let mut file = tokio::fs::OpenOptions::new()
                            .write(true)
                            .create_new(true)
                            .open(&staged)
                            .await
                            .map_err(|_| JobError::DatabaseUnavailable)?;
                        staged_files.push(staged.clone());
                        file.write_all(&artifact.bytes)
                            .await
                            .map_err(|_| JobError::DatabaseUnavailable)?;
                        file.sync_all()
                            .await
                            .map_err(|_| JobError::DatabaseUnavailable)?;
                        drop(file);
                        tokio::fs::rename(&staged, &target)
                            .await
                            .map_err(|_| JobError::DatabaseUnavailable)?;
                        published.push(target);
                        records.push((id, artifact.media_type, artifact.bytes.len() as i64, sha));
                    }
                    super::artifacts::sync_directory(root.clone())
                        .await
                        .map_err(|_| JobError::DatabaseUnavailable)?;
                    self.database
                        .finish_plugin_success(
                            job_id,
                            step_key,
                            &attempt,
                            &output.result,
                            &records,
                            (
                                self.config.background.limits.artifact_bytes_per_job,
                                self.foundation_harness,
                            ),
                        )
                        .await
                }
                .await;
                if let Err(error) = publication {
                    for path in staged_files {
                        let _ = tokio::fs::remove_file(path).await;
                    }
                    for path in published {
                        let _ = tokio::fs::remove_file(path).await;
                    }
                    self.database
                        .finish_plugin_failure(
                            job_id,
                            step_key,
                            &attempt,
                            error.code(),
                            false,
                            self.foundation_harness,
                        )
                        .await?;
                }
            }
            Err(error) => {
                if error == PluginError::WorkerUnreaped {
                    self.running
                        .store(false, std::sync::atomic::Ordering::Release);
                }
                if error != PluginError::WorkerUnreaped {
                    sqlx::query("UPDATE job_steps SET worker_instance_id=NULL WHERE job_id=? AND step_key=? AND attempt_id=?").bind(job_id).bind(step_key).bind(&attempt).execute(self.database.pool()).await?;
                }
                let code = error.code();
                let health_result = if matches!(
                    error,
                    PluginError::PluginRuntimeError | PluginError::WorkerProtocolError
                ) {
                    self.plugins.fail_package(choice, code).await
                } else {
                    Ok(())
                };
                self.database
                    .finish_plugin_failure(
                        job_id,
                        step_key,
                        &attempt,
                        code,
                        matches!(
                            error,
                            PluginError::WorkerTimeout
                                | PluginError::WorkerDeadlineExceeded
                                | PluginError::SandboxUnavailable
                                | PluginError::WorkerUnreaped
                                | PluginError::JobRejected(JobError::Cancelled)
                        ),
                        self.foundation_harness,
                    )
                    .await?;
                health_result.map_err(|_| JobError::DatabaseUnavailable)?;
            }
        }
        self.get(job_id).await
    }
}
