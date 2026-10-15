use super::*;
use crate::services::background::model::encode;
use serde_json::{Value, json};

impl Database {
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
        run: &crate::services::background::model::LlmRunContext,
        pin: &crate::services::background::ProviderPin,
    ) -> Result<(), JobError> {
        let id = run.job_id.as_str();
        let step = run.step_key.as_deref();
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
        let agent_id = run.agent_run_id.as_deref().ok_or(JobError::InvalidPlan)?;
        let active:i64=sqlx::query_scalar(
                "SELECT COUNT(*) FROM job_agent_runs WHERE job_id=? AND agent_run_id=? AND status='running'"
            ).bind(id).bind(agent_id).fetch_one(&mut *tx).await?;
        if active != 1 {
            return Err(JobError::Terminal);
        }
        let global = used["llm_calls"]
            .as_u64()
            .unwrap_or(0)
            .checked_add(1)
            .ok_or(JobError::CapacityUnavailable)?;
        if global > u64::from(snapshot.limits.max_total_llm_calls) {
            return Err(JobError::CapacityUnavailable);
        }
        let agent_calls = used["agent_runs"][agent_id]["llm_calls"]
            .as_u64()
            .unwrap_or(0)
            .checked_add(1)
            .ok_or(JobError::CapacityUnavailable)?;
        if run.role == "worker" && agent_calls > u64::from(snapshot.limits.max_worker_rounds) {
            return Err(JobError::CapacityUnavailable);
        }
        used["llm_calls"] = json!(global);
        used["agent_runs"][agent_id]["llm_calls"] = json!(agent_calls);
        sqlx::query(
            "UPDATE jobs SET budget_used_json=? WHERE id=? AND status IN ('planning','running')",
        )
        .bind(encode(&used)?)
        .bind(id)
        .execute(&mut *tx)
        .await?;
        let mut payload = json!({
            "run_id":run.run_id,"role":run.role,"phase":run.phase,
            "attempt_no":run.attempt_no,"plan_revision":run.plan_revision,
            "provider_ref":pin.reference,
            "provider_source":if pin.reference.starts_with("config:") {"config"} else {"database"},
            "pin_revision":pin.revision,"pin_incarnation":pin.incarnation
        });
        if let Some(agent_id) = &run.agent_run_id {
            payload["agent_run_id"] = json!(agent_id);
            payload["turn_id"] = json!(run.run_id);
        }
        events::append(&mut tx, id, "llm_run_started", step, &payload).await?;
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
            } else if row.execution_mode != "plugin_operation_v3" && enabled != 1 {
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
                if row.execution_mode != "plugin_operation_v3" {
                    for old in [&pins.orchestrator, &pins.worker] {
                        match providers::pin(&mut tx, config, &old.reference).await {
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
                }
                let choices: Vec<Value> = serde_json::from_str(&row.plugin_choices_json)
                    .map_err(|_| JobError::DatabaseUnavailable)?;
                for choice in choices {
                    if choice["kind"] == "core" {
                        if crate::tools::core::decode(&choice).is_err() {
                            status = "failed";
                            error = Some("capability_unavailable");
                        }
                        continue;
                    }
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
        let root:Option<String>=sqlx::query_scalar(
                "SELECT agent_run_id FROM job_agent_runs WHERE job_id=? AND parent_agent_run_id IS NULL AND status='running'"
            ).bind(id).fetch_optional(&mut *tx).await?;
        sqlx::query("UPDATE job_agent_runs SET status=CASE WHEN status IN ('queued','running') THEN 'interrupted' ELSE status END, error_code=COALESCE(error_code,?),finished_at=COALESCE(finished_at,?) WHERE job_id=? AND status IN ('queued','running') AND parent_agent_run_id IS NOT NULL")
                .bind(error.unwrap_or("interrupted")).bind(now).bind(id).execute(&mut *tx).await?;
        if let Some(root) = root {
            let run_state = if status == "succeeded" {
                "succeeded"
            } else if status == "cancelled" {
                "cancelled"
            } else {
                "failed"
            };
            sqlx::query("UPDATE job_agent_runs SET status=?,error_code=?,finished_at=? WHERE job_id=? AND agent_run_id=? AND status='running'")
                    .bind(run_state).bind(error).bind(now).bind(id).bind(&root)
                    .execute(&mut *tx).await?;
            events::append(
                &mut tx,
                id,
                "agent_run_finished",
                None,
                &json!({"agent_run_id":root,"status":run_state,"error_code":error}),
            )
            .await?;
        }
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
