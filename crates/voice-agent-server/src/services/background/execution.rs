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
        let row = self
            .database
            .job_artifact_record(job_id, artifact_id)
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
        let row = self
            .database
            .job_artifact_record(job_id, artifact_id)
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

    #[cfg(test)]
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
        self.database
            .validate_plugin_step_arguments(job_id, step_key, &arguments)
            .await?;
        let attempt = uuid::Uuid::new_v4().to_string();
        let deadline_at = self.get(job_id).await?.deadline_at;
        self.database
            .reserve_plugin_attempt(job_id, step_key, &attempt)
            .await?;
        let result = if crate::tools::core::is_core(choice) {
            // Same durable Attempt as External, but no container, package or image preflight.
            async {
                let _core_slot = tokio::select! {
                    _ = cancellation.cancelled() => {
                        return Err(PluginError::JobRejected(JobError::Cancelled));
                    }
                    permit = self.plugins.acquire_core_slot(deadline_at) => permit?,
                };
                if cancellation.is_cancelled() {
                    return Err(PluginError::JobRejected(JobError::Cancelled));
                }
                self.database.request_plugin_start(job_id, step_key, &attempt, choice)
                    .await.map_err(PluginError::JobRejected)?;
                self.database.mark_plugin_started(job_id, step_key, &attempt)
                    .await.map_err(PluginError::JobRejected)?;
                let remaining = deadline_at - crate::database::unix_seconds()
                    .ok_or(PluginError::JobRejected(JobError::ClockUnavailable))?;
                if remaining <= 0 {
                    return Err(PluginError::WorkerDeadlineExceeded);
                }
                let limit = std::time::Duration::from_secs(
                    self.config.background.limits.step_timeout_seconds.into()
                ).min(std::time::Duration::from_secs(remaining as u64));
                tokio::select! {
                    _ = cancellation.cancelled() => Err(PluginError::JobRejected(JobError::Cancelled)),
                    outcome = tokio::time::timeout(
                        limit,
                        crate::tools::core::invoke(self, choice, arguments, job_id)
                    ) => outcome.map_err(|_| PluginError::WorkerTimeout)?,
                }
            }.await
        } else {
            self.plugins
                .invoke(
                    Invocation {
                        inputs: serde_json::json!({}),
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
                .await
        };
        match result {
            Ok(output) => {
                self.database
                    .clear_plugin_worker(job_id, step_key, &attempt)
                    .await?;
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
                    self.database
                        .clear_plugin_worker(job_id, step_key, &attempt)
                        .await?;
                }
                let code = error.code();
                let health_result = if matches!(
                    error,
                    PluginError::PluginRuntimeError | PluginError::WorkerProtocolError
                ) {
                    if crate::tools::core::is_core(choice) {
                        Ok(())
                    } else {
                        self.plugins.fail_package(choice, code).await
                    }
                } else {
                    Ok(())
                };
                self.database
                    .finish_plugin_failure(
                        job_id,
                        step_key,
                        &attempt,
                        code,
                        !crate::tools::core::is_core(choice)
                            && matches!(
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
