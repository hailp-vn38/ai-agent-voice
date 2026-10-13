use super::*;
use crate::config::AppConfig;
use crate::services::background::{
    JobReceipt,
    model::{digest, encode},
    providers,
};

impl Database {
    /// BEGIN IMMEDIATE serializes replay, live checks, snapshots and capacity admission.
    /// No model work or container operation is performed under the transaction.
    pub(crate) async fn submit_job(
        &self,
        config: &AppConfig,
        request: JobRequest,
        requester: &JobRequester,
        key: Option<&str>,
        now: i64,
        runtime_ready: impl Fn() -> bool,
    ) -> Result<JobReceipt, JobError> {
        let request_json = encode(&request)?;
        if request_json.len() > 16384 {
            return Err(JobError::InvalidArguments);
        }
        let fingerprint = digest(&request_json);
        let scope = requester.scope();
        let _publication = self.tool_security.publication.read().await;
        let mut tx = self.pool().begin_with("BEGIN IMMEDIATE").await?;
        if let Some(key) = key {
            let old: Option<JobRow> =
                sqlx::query_as("SELECT * FROM jobs WHERE requester_scope=? AND idempotency_key=?")
                    .bind(&scope)
                    .bind(key)
                    .fetch_optional(&mut *tx)
                    .await?;
            if let Some(old) = old {
                if old.request_fingerprint != fingerprint {
                    return Err(JobError::IdempotencyConflict);
                }
                let job = old.view(Vec::new(), Vec::new())?;
                tx.commit().await?;
                return Ok(JobReceipt {
                    job,
                    replayed: true,
                });
            }
        }
        if !runtime_ready() {
            return Err(JobError::RuntimeUnavailable);
        }
        let (enabled,orchestrator,worker,revision):(i64,Option<String>,Option<String>,i64)=sqlx::query_as("SELECT enabled,orchestrator_llm_provider,worker_llm_provider,revision FROM background_settings WHERE singleton=1")
            .fetch_one(&mut *tx).await?;
        if enabled != 1 {
            return Err(JobError::Disabled);
        }
        authorize(&mut tx, requester).await?;
        if config.limits.llm_concurrency <= 1 {
            return Err(JobError::CapacityUnavailable);
        }
        let orchestrator = providers::pin(
            &mut tx,
            config,
            orchestrator
                .as_deref()
                .ok_or(JobError::ProviderUnavailable)?,
        )
        .await?;
        let worker = providers::pin(
            &mut tx,
            config,
            worker.as_deref().ok_or(JobError::ProviderUnavailable)?,
        )
        .await?;
        let plugin_choices =
            crate::services::background::plugins::frozen_choices(&mut tx, &request).await?;
        let used: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM jobs WHERE status='queued'")
            .fetch_one(&mut *tx)
            .await?;
        if used >= i64::from(config.background.limits.max_queued_jobs) {
            return Err(JobError::QueueFull);
        }
        let unresolved:i64=sqlx::query_scalar("SELECT COUNT(*) FROM job_steps WHERE worker_instance_id IS NOT NULL AND attempt_state IN ('start_requested','started','unknown_outcome')").fetch_one(&mut *tx).await?;
        if unresolved != 0 {
            return Err(JobError::RuntimeUnavailable);
        }
        let deadline = now
            .checked_add(i64::from(config.background.limits.job_deadline_seconds))
            .ok_or(JobError::ClockUnavailable)?;
        let id = uuid::Uuid::new_v4().to_string();
        let snapshot = JobSnapshot {
            orchestrator,
            worker,
            limits: config.background.limits.clone(),
        };
        let (device, agent) = requester.incarnations();
        sqlx::query("INSERT INTO jobs(id,requester_scope,device_incarnation,agent_incarnation,request_json,request_fingerprint,snapshot_json,plugin_choices_json,settings_revision,budget_limit_json,status,deadline_at,idempotency_key,created_at,updated_at) VALUES(?,?,?,?,?,?,?,?,?,?,'queued',?,?,?,?)")
            .bind(&id).bind(scope).bind(device).bind(agent).bind(request_json).bind(fingerprint).bind(encode(&snapshot)?)
            .bind(plugin_choices)
            .bind(revision).bind(encode(&snapshot.limits)?).bind(deadline).bind(key).bind(now).bind(now).execute(&mut *tx).await?;
        let row: JobRow = sqlx::query_as("SELECT * FROM jobs WHERE id=?")
            .bind(&id)
            .fetch_one(&mut *tx)
            .await?;
        let job = row.view(Vec::new(), Vec::new())?;
        if !runtime_ready() {
            return Err(JobError::RuntimeUnavailable);
        }
        tx.commit().await?;
        Ok(JobReceipt {
            job,
            replayed: false,
        })
    }
}
