use super::{model::*, providers};
use crate::{config::AppConfig, database::Database};
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicUsize, Ordering},
};
use tokio::sync::Notify;

pub struct JobEngine {
    pub plugins: super::plugins::PluginStore,
    pub(crate) submission_gate: tokio::sync::RwLock<()>,
    pub(crate) active_submissions: AtomicUsize,
    pub(crate) database: Arc<Database>,
    pub(crate) config: Arc<AppConfig>,
    pub(crate) wake: Notify,
    pub(crate) running: AtomicBool,
    // Only crate-local cfg(test) harness construction can set this. P3 must replace
    // it with actual planner/worker/runner readiness before enabling production.
    pub(crate) foundation_harness: bool,
}
impl JobEngine {
    pub fn active_submissions(&self) -> usize {
        self.active_submissions.load(Ordering::Acquire)
    }
    pub fn runtime_ready(&self) -> bool {
        self.foundation_harness && self.running.load(Ordering::Acquire)
    }
    pub async fn settings(&self) -> Result<BackgroundSettings, JobError> {
        let (settings, revision) = self.database.background_settings().await?;
        let mut connection = self.database.pool().acquire().await?;
        let llm_providers = providers::options(&mut connection, &self.config).await?;
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
        let mut connection = self.database.pool().acquire().await?;
        for reference in [&input.orchestrator_llm_provider, &input.worker_llm_provider]
            .into_iter()
            .flatten()
        {
            providers::pin(&mut connection, &self.config, reference).await?;
        }
        if input.enabled
            && (input.orchestrator_llm_provider.is_none() || input.worker_llm_provider.is_none())
        {
            return Err(JobError::ProviderUnavailable);
        }
        if input.enabled && self.config.limits.llm_concurrency <= 1 {
            return Err(JobError::CapacityUnavailable);
        }
        drop(connection);
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
        let receipt = self
            .database
            .submit_job(&self.config, input, requester, key, now()?, || {
                self.runtime_ready()
            })
            .await?;
        self.wake.notify_one();
        Ok(receipt)
    }
    pub async fn get(&self, id: &str) -> Result<JobView, JobError> {
        self.database.job_view(id).await
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
    pub async fn cancel(&self, id: &str) -> Result<JobView, JobError> {
        self.database.cancel_job(id, now()?).await
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
