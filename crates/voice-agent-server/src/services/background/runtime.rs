use super::{JobEngine, JobError, engine::now, scheduler};
use crate::{config::AppConfig, database::Database};
use std::{
    fs::{File, OpenOptions},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};
use tokio_util::sync::CancellationToken;

pub struct BackgroundRuntime {
    pub engine: Arc<JobEngine>,
    stop: CancellationToken,
    task: tokio::sync::Mutex<Option<tokio::task::JoinHandle<()>>>,
}
impl BackgroundRuntime {
    pub async fn open(
        database: Arc<Database>,
        config: Arc<AppConfig>,
        stopping: CancellationToken,
    ) -> Result<Arc<Self>, JobError> {
        Self::start(database, config, stopping, false).await
    }
    async fn start(
        database: Arc<Database>,
        config: Arc<AppConfig>,
        stopping: CancellationToken,
        foundation_harness: bool,
    ) -> Result<Arc<Self>, JobError> {
        if !config.background.limits.valid() {
            return Err(JobError::InvalidArguments);
        }
        let lock = deployment_lock(database.durable_path().ok_or(JobError::DeploymentLock)?)?;
        database.recover_jobs(now()?).await?;
        if !foundation_harness {
            // An old enable bit must never activate an incomplete P0 deployment.
            let _publication = database.tool_security.publication.write().await;
            sqlx::query("UPDATE background_settings SET enabled=0,revision=revision+1 WHERE singleton=1 AND enabled=1").execute(database.pool()).await?;
        }
        let engine = Arc::new(JobEngine {
            submission_gate: Default::default(),
            active_submissions: Default::default(),
            database,
            config,
            wake: Default::default(),
            running: AtomicBool::new(true),
            foundation_harness,
        });
        scheduler::scan(&engine).await?;
        let stop = CancellationToken::new();
        let task = tokio::spawn(scheduler::run(engine.clone(), stop.clone(), stopping, lock));
        Ok(Arc::new(Self {
            engine,
            stop,
            task: tokio::sync::Mutex::new(Some(task)),
        }))
    }
    /// Explicit terminal acknowledgement, used by shutdown/restart qualification.
    pub async fn shutdown(&self) {
        self.engine.running.store(false, Ordering::Release);
        self.stop.cancel();
        let mut owner = self.task.lock().await;
        let task = owner.take();
        if let Some(task) = task {
            let _ = task.await;
        }
    }
    #[cfg(test)]
    pub(super) async fn foundation_harness(
        database: Arc<Database>,
        config: Arc<AppConfig>,
    ) -> Result<Arc<Self>, JobError> {
        Self::start(database, config, CancellationToken::new(), true).await
    }
}
impl Drop for BackgroundRuntime {
    fn drop(&mut self) {
        self.engine.running.store(false, Ordering::Release);
        self.stop.cancel();
    }
}

fn deployment_lock(database: &std::path::Path) -> Result<File, JobError> {
    let mut name = database.as_os_str().to_owned();
    name.push(".background.lock");
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(std::path::PathBuf::from(name))
        .map_err(|_| JobError::DeploymentLock)?;
    file.try_lock().map_err(|_| JobError::DeploymentLock)?;
    Ok(file)
}
