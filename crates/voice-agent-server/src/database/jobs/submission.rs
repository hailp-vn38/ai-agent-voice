use super::*;
use crate::config::AppConfig;
use crate::services::background::{
    JobReceipt,
    model::{digest, encode},
};

#[derive(Default)]
pub(crate) struct SubmissionIdentity<'a> {
    pub key: Option<&'a str>,
    pub retry_of: Option<&'a str>,
    pub validated_pair: Option<&'a [crate::services::background::ProviderPin; 2]>,
}

impl Database {
    pub(crate) async fn receipt_by_key(
        &self,
        requester: &JobRequester,
        key: &str,
    ) -> Result<Option<JobReceipt>, JobError> {
        let row: Option<JobRow> =
            sqlx::query_as("SELECT * FROM jobs WHERE requester_scope=? AND idempotency_key=?")
                .bind(requester.scope())
                .bind(key)
                .fetch_optional(self.pool())
                .await?;
        row.map(|row| {
            Ok(JobReceipt {
                job: row.view(Vec::new(), Vec::new())?,
                replayed: true,
            })
        })
        .transpose()
    }
    /// BEGIN IMMEDIATE serializes replay, live checks, snapshots and capacity admission.
    /// No model work or container operation is performed under the transaction.
    pub(crate) async fn submit_job(
        &self,
        config: &AppConfig,
        request: JobRequest,
        requester: &JobRequester,
        identity: SubmissionIdentity<'_>,
        now: i64,
        runtime_ready: impl Fn() -> bool,
    ) -> Result<JobReceipt, JobError> {
        let SubmissionIdentity {
            key,
            retry_of,
            validated_pair,
        } = identity;
        let request_json = encode(&request)?;
        if request_json.len() > 16384 {
            return Err(JobError::InvalidArguments);
        }
        let fingerprint = digest(
            &retry_of
                .map(|id| format!("retry:{id}\n{request_json}"))
                .unwrap_or_else(|| request_json.clone()),
        );
        let scope = requester.scope();
        let _publication = self.tool_security.publication.read().await;
        let commit_started = std::time::Instant::now();
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
        if let Some(id) = retry_of {
            let previous: Option<String> = sqlx::query_scalar("SELECT status FROM jobs WHERE id=?")
                .bind(id)
                .fetch_optional(&mut *tx)
                .await?;
            match previous.as_deref() {
                Some("failed" | "cancelled") => {}
                None => return Err(JobError::NotFound),
                _ => return Err(JobError::Terminal),
            }
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
        if validated_pair.is_some_and(|pair| pair[0] != orchestrator || pair[1] != worker) {
            return Err(JobError::ConfigurationChanged);
        }
        let plugin_choices =
            crate::services::background::plugins::frozen_choices(&mut tx, &request).await?;
        let used: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM jobs WHERE status='queued'")
            .fetch_one(&mut *tx)
            .await?;
        if used >= i64::from(config.background.limits.max_queued_jobs) {
            return Err(JobError::QueueFull);
        }
        let unresolved:i64=sqlx::query_scalar("SELECT COUNT(*) FROM job_steps s JOIN jobs j ON j.id=s.job_id WHERE s.worker_instance_id IS NOT NULL AND (s.attempt_state='unknown_outcome' OR (s.attempt_state IN ('start_requested','started') AND j.status IN ('succeeded','partial_success','failed','cancelled')))").fetch_one(&mut *tx).await?;
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
        sqlx::query("INSERT INTO jobs(id,requester_scope,device_incarnation,agent_incarnation,request_json,request_fingerprint,snapshot_json,plugin_choices_json,settings_revision,budget_limit_json,status,deadline_at,idempotency_key,retry_of_job_id,created_at,updated_at) VALUES(?,?,?,?,?,?,?,?,?,?,'queued',?,?,?,?,?)")
            .bind(&id).bind(scope).bind(device).bind(agent).bind(request_json).bind(fingerprint).bind(encode(&snapshot)?)
            .bind(plugin_choices)
            .bind(revision).bind(encode(&snapshot.limits)?).bind(deadline).bind(key).bind(retry_of).bind(now).bind(now).execute(&mut *tx).await?;
        let row: JobRow = sqlx::query_as("SELECT * FROM jobs WHERE id=?")
            .bind(&id)
            .fetch_one(&mut *tx)
            .await?;
        let job = row.view(Vec::new(), Vec::new())?;
        if !runtime_ready() {
            return Err(JobError::RuntimeUnavailable);
        }
        tx.commit().await?;
        self.job_changes.publish_committed(&id);
        tracing::info!(event="background_job_committed", job_id=%id, db_commit_ms=commit_started.elapsed().as_millis() as u64);
        Ok(JobReceipt {
            job,
            replayed: false,
        })
    }
}
