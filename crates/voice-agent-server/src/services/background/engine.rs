use super::model::*;
use crate::{config::AppConfig, database::Database};
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicUsize, Ordering},
};
use tokio::sync::Notify;

pub use super::manual_admission::ArtifactInputBinding;

pub struct JobEngine {
    pub plugins: super::plugins::PluginStore,
    pub(crate) submission_gate: tokio::sync::RwLock<()>,
    pub(crate) active_submissions: AtomicUsize,
    pub(crate) database: Arc<Database>,
    pub(crate) config: Arc<AppConfig>,
    pub(crate) wake: Notify,
    pub(crate) running: AtomicBool,
    // Only the P0/P2 cfg(test) persistence harness bypasses model execution.
    pub(crate) foundation_harness: bool,
    pub(crate) models: Option<Arc<dyn super::models::JobModels>>,
    pub(crate) cancellations:
        std::sync::Mutex<std::collections::HashMap<String, tokio_util::sync::CancellationToken>>,
    pub(crate) voice_submissions: Arc<tokio::sync::Semaphore>,
}
impl JobEngine {
    pub fn active_submissions(&self) -> usize {
        self.active_submissions.load(Ordering::Acquire)
    }
    pub fn runtime_ready(&self) -> bool {
        (self.foundation_harness || self.models.is_some()) && self.running.load(Ordering::Acquire)
    }
    pub async fn settings(&self) -> Result<BackgroundSettings, JobError> {
        let (settings, revision) = self.database.background_settings().await?;
        let llm_providers = self
            .database
            .background_provider_options(&self.config)
            .await?;
        Ok(BackgroundSettings {
            settings,
            revision,
            llm_providers,
            runtime_ready: self.runtime_ready(),
        })
    }
    pub async fn update_settings(
        &self,
        input: BackgroundSettingsUpdate,
        expected: i64,
    ) -> Result<BackgroundSettings, JobError> {
        if expected <= 0 {
            return Err(JobError::InvalidArguments);
        }
        if input.enabled && !self.runtime_ready() {
            return Err(JobError::RuntimeUnavailable);
        }
        let mut pins = Vec::new();
        for reference in [&input.orchestrator_llm_provider, &input.worker_llm_provider]
            .into_iter()
            .flatten()
        {
            pins.push(
                self.database
                    .background_provider_pin(&self.config, reference)
                    .await?,
            );
        }
        if input.enabled
            && (input.orchestrator_llm_provider.is_none() || input.worker_llm_provider.is_none())
        {
            return Err(JobError::ProviderUnavailable);
        }
        if input.enabled && self.config.limits.llm_concurrency <= 1 {
            return Err(JobError::CapacityUnavailable);
        }
        if input.enabled
            && let Some(models) = &self.models
        {
            for pin in pins {
                models.ready(&pin, now()?.saturating_add(5)).await?;
            }
        }
        self.database
            .save_background_settings(&input, expected)
            .await?;
        self.wake.notify_one();
        self.settings().await
    }
    pub async fn submit(
        &self,
        input: JobRequest,
        requester: &JobRequester,
        key: Option<&str>,
    ) -> Result<JobReceipt, JobError> {
        let input = input.normalize()?;
        if key.is_some_and(|k| {
            k.is_empty() || k.len() > 128 || !k.bytes().all(|b| b.is_ascii_graphic())
        }) {
            return Err(JobError::InvalidArguments);
        }
        let _submission = self.submission_gate.read().await;
        self.active_submissions.fetch_add(1, Ordering::AcqRel);
        let _activity = SubmissionActivity(&self.active_submissions);
        // Replay is checked before live preflight so a lost committed ACK remains readable.
        if let Some(key) = key
            && let Some(old) = self.database.receipt_by_key(requester, key).await?
        {
            if self
                .database
                .job_row(&old.job.id)
                .await?
                .request_fingerprint
                != digest(&encode(&input)?)
            {
                return Err(JobError::IdempotencyConflict);
            }
            return Ok(old);
        }
        let pair = self.validate_pair().await?;
        let receipt = self
            .database
            .submit_job(
                &self.config,
                input,
                requester,
                crate::database::jobs::SubmissionIdentity {
                    key,
                    retry_of: None,
                    validated_pair: pair.as_ref(),
                    #[cfg(test)]
                    test_execution_mode: None,
                },
                now()?,
                || self.runtime_ready(),
            )
            .await?;
        self.wake.notify_one();
        Ok(receipt)
    }
    pub async fn get(&self, id: &str) -> Result<JobView, JobError> {
        self.database.job_view(id).await
    }
    pub async fn retry(
        &self,
        id: &str,
        confirm_safe_read_only: bool,
        key: Option<&str>,
    ) -> Result<JobReceipt, JobError> {
        if !confirm_safe_read_only
            || key.is_some_and(|key| {
                key.is_empty()
                    || key.len() > 128
                    || !key.bytes().all(|byte| byte.is_ascii_graphic())
            })
        {
            return Err(JobError::InvalidArguments);
        }
        let old = self.get(id).await?;
        if old.execution_mode == "plugin_operation_v3" {
            // "Run again" is a fresh admission, never an implicit Agent retry.
            return Err(JobError::InvalidArguments);
        }
        if !matches!(old.status.as_str(), "failed" | "cancelled") {
            return Err(JobError::Terminal);
        }
        let generated = uuid::Uuid::new_v4().to_string();
        let _submission = self.submission_gate.read().await;
        self.active_submissions.fetch_add(1, Ordering::AcqRel);
        let _activity = SubmissionActivity(&self.active_submissions);
        if let Some(key) = key
            && let Some(old) = self
                .database
                .receipt_by_key(&JobRequester::Admin, key)
                .await?
        {
            if old.job.retry_of_job_id.as_deref() != Some(id) {
                return Err(JobError::IdempotencyConflict);
            }
            return Ok(old);
        }
        let pair = self.validate_pair().await?;
        let receipt = self
            .database
            .submit_job(
                &self.config,
                old.request,
                &JobRequester::Admin,
                crate::database::jobs::SubmissionIdentity {
                    key: Some(key.unwrap_or(&generated)),
                    retry_of: Some(id),
                    validated_pair: pair.as_ref(),
                    #[cfg(test)]
                    test_execution_mode: None,
                },
                now()?,
                || self.runtime_ready(),
            )
            .await?;
        self.wake.notify_one();
        Ok(receipt)
    }
    async fn validate_pair(&self) -> Result<Option<[ProviderPin; 2]>, JobError> {
        let Some(models) = &self.models else {
            return Ok(None);
        };
        let (settings, _) = self.database.background_settings().await?;
        if !settings.enabled {
            return Err(JobError::Disabled);
        }
        let orchestrator = self
            .database
            .background_provider_pin(
                &self.config,
                settings
                    .orchestrator_llm_provider
                    .as_deref()
                    .ok_or(JobError::ProviderUnavailable)?,
            )
            .await?;
        let worker = self
            .database
            .background_provider_pin(
                &self.config,
                settings
                    .worker_llm_provider
                    .as_deref()
                    .ok_or(JobError::ProviderUnavailable)?,
            )
            .await?;
        let pair = [orchestrator, worker];
        for pin in &pair {
            models.ready(pin, now()?.saturating_add(5)).await?;
        }
        Ok(Some(pair))
    }
    pub async fn list(
        &self,
        page: u32,
        size: u32,
        status: Option<&str>,
        q: Option<&str>,
    ) -> Result<(Vec<JobView>, i64), JobError> {
        if page == 0
            || !(1..=100).contains(&size)
            || q.is_some_and(|q| q.len() > 128)
            || status.is_some_and(|s| {
                !matches!(
                    s,
                    "queued"
                        | "planning"
                        | "running"
                        | "cancelling"
                        | "succeeded"
                        | "partial_success"
                        | "failed"
                        | "cancelled"
                )
            })
        {
            return Err(JobError::InvalidArguments);
        }
        self.database.list_jobs(page, size, status, q).await
    }
    /// Stop remaining owned work before deleting a terminal Job; retries survive independently.
    pub async fn delete(&self, id: &str) -> Result<(), JobError> {
        let job = self.database.job_row(id).await?;
        if !matches!(
            job.status.as_str(),
            "succeeded" | "partial_success" | "failed" | "cancelled"
        ) {
            return Err(JobError::DeleteConflict);
        }
        let cancellation = self
            .cancellations
            .lock()
            .expect("Job cancellations")
            .get(id)
            .cloned();
        tokio::time::timeout(std::time::Duration::from_secs(15), async {
            if let Some(cancellation) = cancellation {
                cancellation.cancel();
                while self
                    .cancellations
                    .lock()
                    .expect("Job cancellations")
                    .contains_key(id)
                {
                    tokio::time::sleep(std::time::Duration::from_millis(25)).await;
                }
            }
            self.plugins.stop_job_workers(id).await
        })
        .await
        .map_err(|_| JobError::DeleteConflict)??;
        self.database.delete_terminal_job(id).await?;
        if let Err(error) = super::artifacts::remove_deleted_job(&self.database, id).await {
            // DB committed: do not report a rollback. Startup orphan repair can retry.
            tracing::warn!(event="background_job_artifact_cleanup_failed",job_id=%id,code=%error.code());
        }
        Ok(())
    }

    pub async fn cancel(&self, id: &str) -> Result<JobView, JobError> {
        let job = self.database.cancel_job(id, now()?).await?;
        if let Some(token) = self
            .cancellations
            .lock()
            .expect("Job cancellations")
            .get(id)
        {
            token.cancel();
        }
        self.wake.notify_one();
        Ok(job)
    }
}
struct SubmissionActivity<'a>(&'a AtomicUsize);
impl Drop for SubmissionActivity<'_> {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::AcqRel);
    }
}
pub(crate) fn now() -> Result<i64, JobError> {
    crate::database::unix_seconds().ok_or(JobError::ClockUnavailable)
}
