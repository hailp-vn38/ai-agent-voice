use super::*;
use crate::services::background::{model::encode, plan::JobPlan};
use serde_json::{Value, json};

impl Database {
    pub(crate) async fn mark_llm_started(
        &self,
        id: &str,
        step: &str,
        attempt: &str,
    ) -> Result<(), JobError> {
        let now = crate::services::background::engine::now()?;
        let updated = sqlx::query("UPDATE job_steps SET attempt_state='started',started_at=?,updated_at=? WHERE job_id=? AND step_key=? AND attempt_id=? AND status='running' AND EXISTS(SELECT 1 FROM jobs WHERE id=? AND status='running')")
            .bind(now).bind(now).bind(id).bind(step).bind(attempt).bind(id)
            .execute(self.pool()).await?;
        if updated.rows_affected() != 1 {
            return Err(JobError::Terminal);
        }
        self.job_changes.publish_committed(id);
        Ok(())
    }

    pub(crate) async fn claim_job(&self, id: &str, maximum: u32) -> Result<bool, JobError> {
        let mut tx = self.pool().begin_with("BEGIN IMMEDIATE").await?;
        let now = crate::services::background::engine::now()?;
        let active: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM jobs WHERE status IN ('planning','running','cancelling')",
        )
        .fetch_one(&mut *tx)
        .await?;
        if active >= i64::from(maximum) {
            return Ok(false);
        }
        let updated = sqlx::query("UPDATE jobs SET status='planning',scheduler_seen_at=COALESCE(scheduler_seen_at,?),updated_at=? WHERE id=? AND status='queued' AND deadline_at>?")
            .bind(now).bind(now).bind(id).bind(now).execute(&mut *tx).await?;
        tx.commit().await?;
        if updated.rows_affected() == 1 {
            self.job_changes.publish_committed(id);
        }
        Ok(updated.rows_affected() == 1)
    }

    pub(crate) async fn reserve_model_call(
        &self,
        id: &str,
        step: Option<&str>,
    ) -> Result<(), JobError> {
        let mut tx = self.pool().begin_with("BEGIN IMMEDIATE").await?;
        let job = sqlx::query_as::<_, JobRow>(
            "SELECT * FROM jobs WHERE id=? AND status IN ('planning','running')",
        )
        .bind(id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(JobError::Terminal)?;
        if job.deadline_at <= crate::services::background::engine::now()? {
            return Err(JobError::DeadlineExceeded);
        }
        let snapshot: JobSnapshot =
            serde_json::from_str(&job.snapshot_json).map_err(|_| JobError::DatabaseUnavailable)?;
        let raw: String = sqlx::query_scalar("SELECT budget_used_json FROM jobs WHERE id=?")
            .bind(id)
            .fetch_one(&mut *tx)
            .await?;
        let mut used: Value =
            serde_json::from_str(&raw).map_err(|_| JobError::DatabaseUnavailable)?;
        let count = if let Some(step) = step {
            &mut used["steps"][step]["llm_calls"]
        } else {
            &mut used["planner_calls"]
        };
        let next = count
            .as_u64()
            .unwrap_or(0)
            .checked_add(1)
            .ok_or(JobError::CapacityUnavailable)?;
        let limit = if step.is_some() {
            snapshot.limits.max_worker_rounds
        } else {
            (1 + snapshot.limits.max_replans) * (1 + snapshot.limits.max_plan_repairs)
        };
        if next > u64::from(limit) {
            return Err(JobError::CapacityUnavailable);
        }
        *count = json!(next);
        sqlx::query(
            "UPDATE jobs SET budget_used_json=? WHERE id=? AND status IN ('planning','running')",
        )
        .bind(encode(&used)?)
        .bind(id)
        .execute(&mut *tx)
        .await?;
        if step.is_none() {
            events::append(&mut tx,id,"plan_attempt",None,&json!({
                "attempt":next,"phase":if job.plan_revision == 0 {"initial_or_repair"}else{"replan_or_repair"}
            })).await?;
        }
        tx.commit().await?;
        self.job_changes.publish_committed(id);
        Ok(())
    }

    pub(crate) async fn publish_job_plan(
        &self,
        id: &str,
        plan: &JobPlan,
        previous: Option<&JobPlan>,
    ) -> Result<(), JobError> {
        let mut tx = self.pool().begin_with("BEGIN IMMEDIATE").await?;
        let job: JobRow =
            sqlx::query_as("SELECT * FROM jobs WHERE id=? AND status IN ('planning','running')")
                .bind(id)
                .fetch_optional(&mut *tx)
                .await?
                .ok_or(JobError::Terminal)?;
        let now = crate::services::background::engine::now()?;
        if now >= job.deadline_at {
            return Err(JobError::DeadlineExceeded);
        }
        let revision = job.plan_revision + 1;
        let live:i64=sqlx::query_scalar("SELECT COUNT(*) FROM job_steps WHERE job_id=? AND (status IN ('pending','running') OR worker_instance_id IS NOT NULL OR attempt_state='unknown_outcome')").bind(id).fetch_one(&mut *tx).await?;
        if previous.is_some() && live != 0 {
            return Err(JobError::InvalidPlan);
        }
        if previous.is_none() && job.plan_revision != 0 {
            return Err(JobError::InvalidPlan);
        }
        let view = job.view(Vec::new(), Vec::new())?;
        // Revalidate at the publication boundary; the model cannot supply pinned identity.
        let plan = JobPlan::parse(&encode(plan)?, &view)?;
        if let Some(previous) = previous {
            if revision > i64::from(view.snapshot.limits.max_replans) + 1
                || plan.steps.len() <= previous.steps.len()
            {
                return Err(JobError::InvalidPlan);
            }
            for old in &previous.steps {
                if plan.steps.iter().find(|step| step.step_key == old.step_key) != Some(old) {
                    return Err(JobError::InvalidPlan);
                }
            }
            for step in &plan.steps {
                if !previous
                    .steps
                    .iter()
                    .any(|old| old.step_key == step.step_key)
                    && step.plugin_id.is_some()
                    && previous.steps.iter().any(|old| {
                        old.plugin_id == step.plugin_id && old.operation == step.operation
                    })
                {
                    return Err(JobError::InvalidPlan);
                }
            }
        }
        let step_count = plan.steps.len();
        for step in plan.steps {
            if previous.is_some_and(|previous| {
                previous
                    .steps
                    .iter()
                    .any(|old| old.step_key == step.step_key)
            }) {
                continue;
            }
            let choice = step.plugin_id.as_ref().and_then(|plugin| {
                let operation = step.operation.as_ref()?;
                view.plugin_choices.as_array()?.iter().find(|choice| {
                    choice["plugin_id"] == *plugin && choice["operation"] == *operation
                })
            });
            sqlx::query("INSERT INTO job_steps(job_id,step_key,plan_revision,plugin_id,plugin_version,package_digest,installation_digest,operation,arguments_json,input_refs_json,output_schema_json,dependencies_json,required,instruction,status,created_at,updated_at) VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?,?,'pending',?,?)")
                .bind(id).bind(&step.step_key).bind(revision).bind(&step.plugin_id)
                .bind(choice.and_then(|v| v["version"].as_str())).bind(choice.and_then(|v| v["package_digest"].as_str())).bind(choice.and_then(|v| v["installation_digest"].as_str()))
                .bind(&step.operation).bind(encode(&step.arguments)?).bind(encode(&step.input_refs)?).bind(encode(&step.output_schema)?).bind(encode(&step.dependencies)?).bind(step.required)
                .bind(&step.instruction).bind(now).bind(now).execute(&mut *tx).await?;
        }
        let changed=sqlx::query("UPDATE jobs SET status='running',plan_revision=?,updated_at=? WHERE id=? AND status IN ('planning','running') AND plan_revision=?").bind(revision).bind(now).bind(id).bind(revision-1).execute(&mut *tx).await?;
        if changed.rows_affected() != 1 {
            return Err(JobError::Terminal);
        }
        events::append(
            &mut tx,
            id,
            "plan_accepted",
            None,
            &json!({"plan_revision":revision,"step_count":step_count}),
        )
        .await?;
        tx.commit().await?;
        self.job_changes.publish_committed(id);
        Ok(())
    }

    pub(crate) async fn skip_dependency_step(&self, id: &str, step: &str) -> Result<(), JobError> {
        let mut tx = self.pool().begin_with("BEGIN IMMEDIATE").await?;
        let now = crate::services::background::engine::now()?;
        let updated=sqlx::query("UPDATE job_steps SET status='skipped',error_code='dependency_failed',updated_at=? WHERE job_id=? AND step_key=? AND status='pending'")
            .bind(now).bind(id).bind(step).execute(&mut *tx).await?;
        if updated.rows_affected() != 1 {
            return Err(JobError::Terminal);
        }
        events::append(
            &mut tx,
            id,
            "step_finished",
            Some(step),
            &json!({"status":"skipped","error_code":"dependency_failed"}),
        )
        .await?;
        tx.commit().await?;
        self.job_changes.publish_committed(id);
        Ok(())
    }
    pub(crate) async fn checkpoint_llm_step(
        &self,
        id: &str,
        step: &str,
        attempt: &str,
        outcome: Result<&Value, JobError>,
    ) -> Result<(), JobError> {
        let now = crate::services::background::engine::now()?;
        let (status, result, error) = match outcome {
            Ok(value) => ("succeeded", Some(encode(value)?), None),
            Err(error) => ("failed", None, Some(error.code())),
        };
        let mut tx = self.pool().begin_with("BEGIN IMMEDIATE").await?;
        let updated = sqlx::query("UPDATE job_steps SET status=?,result_json=?,error_code=?,attempt_state='finished',finished_at=?,updated_at=? WHERE job_id=? AND step_key=? AND attempt_id=? AND status='running' AND EXISTS(SELECT 1 FROM jobs WHERE id=? AND status='running' AND deadline_at>?)")
            .bind(status).bind(result).bind(error).bind(now).bind(now).bind(id).bind(step).bind(attempt).bind(id).bind(now).execute(&mut *tx).await?;
        if updated.rows_affected() != 1 {
            // Stay on the same connection while BEGIN IMMEDIATE is held.
            let (deadline_at, current_status): (i64, String) =
                sqlx::query_as("SELECT deadline_at,status FROM jobs WHERE id=?")
                    .bind(id)
                    .fetch_one(&mut *tx)
                    .await?;
            if deadline_at <= now {
                return Err(JobError::DeadlineExceeded);
            }
            if current_status == "cancelling" {
                return Err(JobError::Cancelled);
            }
            return Err(JobError::Terminal);
        }
        events::append(
            &mut tx,
            id,
            "step_finished",
            Some(step),
            &json!({"status":status,"error_code":error}),
        )
        .await?;
        tx.commit().await?;
        self.job_changes.publish_committed(id);
        Ok(())
    }

    pub(crate) async fn finish_job(
        &self,
        config: &crate::config::AppConfig,
        id: &str,
        status: &str,
        error: Option<&str>,
        result: Option<&Value>,
    ) -> Result<(), JobError> {
        let _publication = self.tool_security.publication.read().await;
        let mut tx = self.pool().begin_with("BEGIN IMMEDIATE").await?;
        let now = crate::services::background::engine::now()?;
        let current: String = sqlx::query_scalar("SELECT status FROM jobs WHERE id=?")
            .bind(id)
            .fetch_one(&mut *tx)
            .await?;
        let (status, error) = if current == "cancelling" {
            ("cancelled", Some("cancelled"))
        } else {
            (status, error)
        };
        let mut status = status;
        let mut error = error;
        if matches!(status, "succeeded" | "partial_success") {
            let row: JobRow = sqlx::query_as("SELECT * FROM jobs WHERE id=?")
                .bind(id)
                .fetch_one(&mut *tx)
                .await?;
            let enabled: i64 =
                sqlx::query_scalar("SELECT enabled FROM background_settings WHERE singleton=1")
                    .fetch_one(&mut *tx)
                    .await?;
            let rejected = if row.deadline_at <= now {
                Some(JobError::DeadlineExceeded)
            } else if enabled != 1 {
                Some(JobError::Disabled)
            } else {
                authorize(&mut tx, &row.requester()?).await.err()
            };
            if let Some(reason) = rejected {
                status = "failed";
                error = Some(reason.code());
            }
            if matches!(status, "succeeded" | "partial_success") {
                let pins: JobSnapshot = serde_json::from_str(&row.snapshot_json)
                    .map_err(|_| JobError::DatabaseUnavailable)?;
                for old in [&pins.orchestrator, &pins.worker] {
                    match providers::pin(
                        &mut tx,
                        config,
                        &old.reference,
                    )
                    .await
                    {
                        Ok(pin) if &pin == old => {}
                        Ok(_) => {
                            status = "failed";
                            error = Some("configuration_changed");
                        }
                        Err(reason) => {
                            status = "failed";
                            error = Some(reason.code());
                        }
                    }
                }
                let choices: Vec<Value> = serde_json::from_str(&row.plugin_choices_json)
                    .map_err(|_| JobError::DatabaseUnavailable)?;
                for choice in choices {
                    let installed: Option<i64> = sqlx::query_scalar(
                        "SELECT 1 FROM plugin_installations WHERE plugin_id=? AND version=? AND package_digest=? AND installation_digest=? AND status='ready'",
                    )
                    .bind(choice["plugin_id"].as_str())
                    .bind(choice["version"].as_str())
                    .bind(choice["package_digest"].as_str())
                    .bind(choice["installation_digest"].as_str())
                    .fetch_optional(&mut *tx)
                    .await?;
                    if installed.is_none() {
                        status = "failed";
                        error = Some("capability_unavailable");
                    }
                }
            }
        }
        if !matches!(current.as_str(), "planning" | "running" | "cancelling") {
            return Err(JobError::Terminal);
        }
        sqlx::query("UPDATE job_steps SET status=CASE WHEN status='running' THEN 'failed' ELSE 'skipped' END,attempt_state=CASE WHEN attempt_state IN ('start_requested','started') THEN 'unknown_outcome' ELSE attempt_state END,error_code=COALESCE(error_code,?),finished_at=?,updated_at=? WHERE job_id=? AND status IN ('pending','running')")
            .bind(error.unwrap_or("dependency_failed")).bind(now).bind(now).bind(id).execute(&mut *tx).await?;
        sqlx::query("UPDATE jobs SET status=?,error_code=?,result_json=?,updated_at=? WHERE id=? AND status IN ('planning','running','cancelling')")
            .bind(status).bind(error).bind(result.map(encode).transpose()?).bind(now).bind(id).execute(&mut *tx).await?;
        events::append(
            &mut tx,
            id,
            "job_finished",
            None,
            &json!({"status":status,"error_code":error}),
        )
        .await?;
        tx.commit().await?;
        self.job_changes.publish_committed(id);
        Ok(())
    }
}
