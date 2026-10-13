use super::{
    JobEngine, JobError, JobView,
    plugins::{FrozenChoice, Invocation, PluginError},
};
use serde_json::Value;
use sha2::{Digest, Sha256};
use tokio::io::AsyncWriteExt;

impl JobEngine {
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
        let attempt = uuid::Uuid::new_v4().to_string();
        let deadline_at = self.get(job_id).await?.deadline_at;
        self.database
            .reserve_plugin_attempt(job_id, step_key, &attempt)
            .await?;
        let result = self
            .plugins
            .invoke(
                Invocation {
                    choice,
                    arguments,
                    job_id,
                    attempt_id: &attempt,
                    timeout: std::time::Duration::from_secs(
                        self.config.background.limits.step_timeout_seconds.into(),
                    ),
                    deadline_at,
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
                            self.config.background.limits.artifact_bytes_per_job,
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
                        .finish_plugin_failure(job_id, step_key, &attempt, error.code(), false)
                        .await?;
                }
            }
            Err(error) => {
                let code = error.code();
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
                        ),
                    )
                    .await?;
                if matches!(
                    error,
                    PluginError::PluginRuntimeError | PluginError::WorkerProtocolError
                ) {
                    sqlx::query("UPDATE plugin_installations SET status='error',last_error_code=?,revision=revision+1,updated_at=? WHERE plugin_id=? AND package_digest=? AND status='ready'")
                        .bind(code).bind(crate::database::unix_seconds()).bind(&choice.plugin_id)
                        .bind(&choice.package_digest).execute(self.database.pool()).await?;
                }
            }
        }
        self.get(job_id).await
    }
}
