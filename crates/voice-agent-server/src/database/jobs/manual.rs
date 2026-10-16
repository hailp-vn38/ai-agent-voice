//! Dedicated no-LLM admission and single-Step transition. Agent Job admission
//! and Root/Child Sessions remain unchanged.
use super::*;
use crate::config::AppConfig;
use crate::services::background::{JobReceipt, model::encode};
use serde_json::{Value, json};

/// Already-staged file ownership is published atomically with the queued Job.
pub(crate) struct ImportedArtifact {
    pub id: String,
    pub filename: String,
    pub media_type: String,
    pub size_bytes: i64,
    pub sha256: String,
}

impl Database {
    /// One authoritative read for the Plugin Runs tab; Core catalog names
    /// map to their frozen group operation prefix (plugin_id is always "core").
    pub(crate) async fn list_plugin_runs(
        &self,
        plugin_key: &str,
        operation: Option<&str>,
        page: u32,
        size: u32,
    ) -> Result<(Vec<JobView>, i64), JobError> {
        if page == 0 || !(1..=100).contains(&size) || operation.is_some_and(|name| name.len() > 64)
        {
            return Err(JobError::InvalidArguments);
        }
        let (id, prefix) = if crate::tools::core::known_group(plugin_key) {
            (
                "core",
                Some(format!("{}.%", plugin_key.trim_start_matches("core."))),
            )
        } else {
            (plugin_key, None)
        };
        let mut tx = self.pool().begin().await?;
        let total: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM jobs j JOIN job_steps s ON j.id=s.job_id
             WHERE j.execution_mode='plugin_operation_v3' AND s.step_key='operation_0'
               AND s.plugin_id=? AND (? IS NULL OR s.operation LIKE ?)
               AND (? IS NULL OR s.operation=?)",
        )
        .bind(id)
        .bind(&prefix)
        .bind(&prefix)
        .bind(operation)
        .bind(operation)
        .fetch_one(&mut *tx)
        .await?;
        let rows: Vec<JobRow> = sqlx::query_as(
            "SELECT j.* FROM jobs j JOIN job_steps s ON j.id=s.job_id
             WHERE j.execution_mode='plugin_operation_v3' AND s.step_key='operation_0'
               AND s.plugin_id=? AND (? IS NULL OR s.operation LIKE ?)
               AND (? IS NULL OR s.operation=?)
             ORDER BY j.created_at DESC,j.id DESC LIMIT ? OFFSET ?",
        )
        .bind(id)
        .bind(&prefix)
        .bind(&prefix)
        .bind(operation)
        .bind(operation)
        .bind(size)
        .bind(i64::from(page - 1) * i64::from(size))
        .fetch_all(&mut *tx)
        .await?;
        tx.commit().await?;
        let jobs = rows
            .into_iter()
            .map(|row| row.view(Vec::new(), Vec::new()))
            .collect::<Result<Vec<_>, _>>()?;
        Ok((jobs, total))
    }

    pub(crate) async fn submit_plugin_operation(
        &self,
        config: &AppConfig,
        key: &str,
        choice: &Value,
        arguments: &Value,
        output_schema: &Value,
        now: i64,
        id: &str,
        imports: &[ImportedArtifact],
        fingerprint: &str,
        idempotency_key: Option<&str>,
    ) -> Result<JobReceipt, JobError> {
        let operation = choice["operation"]
            .as_str()
            .ok_or(JobError::InvalidArguments)?;
        let admin = JobRequester::Admin;
        let _publication = self.tool_security.publication.read().await;
        let mut tx = self.pool().begin_with("BEGIN IMMEDIATE").await?;
        authorize(&mut tx, &admin).await?;
        // Lock serializes same-key admission and prevents two concurrent runs.
        if let Some(key) = idempotency_key {
            let existing: Option<JobRow> = sqlx::query_as(
                "SELECT * FROM jobs WHERE requester_scope='admin:deployment' AND idempotency_key=?",
            )
            .bind(key)
            .fetch_optional(&mut *tx)
            .await?;
            if let Some(existing) = existing {
                if existing.execution_mode != "plugin_operation_v3"
                    || existing.request_fingerprint != fingerprint
                {
                    return Err(JobError::IdempotencyConflict);
                }
                let view = existing.view(Vec::new(), Vec::new())?;
                return Ok(JobReceipt {
                    job: view,
                    replayed: true,
                });
            }
        }
        let currently_unreaped: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM job_steps WHERE worker_instance_id IS NOT NULL AND attempt_state='unknown_outcome'"
        ).fetch_one(&mut *tx).await?;
        if currently_unreaped > 0 {
            return Err(JobError::RuntimeUnavailable);
        }
        let used: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM jobs WHERE status='queued'")
            .fetch_one(&mut *tx)
            .await?;
        if used >= i64::from(config.background.limits.max_queued_jobs) {
            return Err(JobError::QueueFull);
        }
        if choice["kind"] == "core" {
            let group =
                crate::tools::core::group_for(operation).ok_or(JobError::CapabilityUnavailable)?;
            if group != key {
                return Err(JobError::CapabilityUnavailable);
            }
            let enabled: Option<i64> =
                sqlx::query_scalar("SELECT enabled FROM core_plugin_settings WHERE plugin_id=?")
                    .bind(group)
                    .fetch_optional(&mut *tx)
                    .await?;
            if enabled != Some(1) {
                return Err(JobError::CorePluginDisabled);
            }
            crate::tools::core::decode(choice).map_err(JobError::from_plugin)?;
        } else {
            if key == "core" || key.starts_with("core.") || choice["plugin_id"] != key {
                return Err(JobError::CapabilityUnavailable);
            }
            let present: Option<i64> = sqlx::query_scalar(
                "SELECT 1 FROM plugin_installations WHERE plugin_id=? AND version=?
                 AND package_digest=? AND installation_digest=? AND status='ready'",
            )
            .bind(key)
            .bind(choice["version"].as_str())
            .bind(choice["package_digest"].as_str())
            .bind(choice["installation_digest"].as_str())
            .fetch_optional(&mut *tx)
            .await?;
            if present.is_none() {
                return Err(JobError::CapabilityUnavailable);
            }
        }
        let tts_pin = if key == "core.tts" {
            Some(
                tts_provider::configured_pin(&mut tx, config)
                    .await?
                    .ok_or(JobError::CapabilityUnavailable)?,
            )
        } else {
            None
        };
        let config_revision: Option<i64> =
            sqlx::query_scalar("SELECT current_revision FROM plugin_configs WHERE plugin_key=?")
                .bind(key)
                .fetch_optional(&mut *tx)
                .await?;
        let settings_revision: i64 =
            sqlx::query_scalar("SELECT revision FROM background_settings WHERE singleton=1")
                .fetch_one(&mut *tx)
                .await?;
        let request = crate::services::background::JobRequest {
            goal: format!("Run {key}/{operation}"),
            output_format: "json".into(),
            allow_partial: false,
            required_sources: vec![],
            required_capabilities: vec![],
        };
        let request_json = encode(&request)?;
        let snapshot = JobSnapshot::plugin_operation(config.background.limits.clone());
        let snapshot_json = encode(&snapshot)?;
        let choices = encode(&json!([choice]))?;
        let args = encode(arguments)?;
        if args.len() > 16_384 {
            return Err(JobError::InvalidArguments);
        }
        let schema = encode(output_schema)?;
        let deadline = now
            .checked_add(i64::from(config.background.limits.job_deadline_seconds))
            .ok_or(JobError::ClockUnavailable)?;
        sqlx::query(
            "INSERT INTO jobs(
                id,requester_scope,request_json,request_fingerprint,snapshot_json,
                plugin_choices_json,settings_revision,budget_limit_json,execution_mode,status,
                deadline_at,created_at,updated_at,plan_revision,idempotency_key
            ) VALUES(?,'admin:deployment',?,?,?,?,?,?,'plugin_operation_v3','queued',?,?,?,1,?)",
        )
        .bind(id)
        .bind(request_json)
        .bind(fingerprint)
        .bind(snapshot_json)
        .bind(choices)
        .bind(settings_revision)
        .bind(encode(&snapshot.limits)?)
        .bind(deadline)
        .bind(now)
        .bind(now)
        .bind(idempotency_key)
        .execute(&mut *tx)
        .await?;
        sqlx::query(
            "INSERT INTO job_steps(
                job_id,step_key,plan_revision,plugin_id,plugin_version,package_digest,
                installation_digest,operation,arguments_json,output_schema_json,
                status,created_at,updated_at,instruction
            ) VALUES(?,'operation_0',1,?,?,?,?,?, ?,?,'pending',?,?,?)",
        )
        .bind(id)
        .bind(choice["plugin_id"].as_str())
        .bind(if choice["kind"] == "core" {
            choice["contract_revision"].as_str()
        } else {
            choice["version"].as_str()
        })
        .bind(choice["package_digest"].as_str().unwrap_or(""))
        .bind(choice["installation_digest"].as_str().unwrap_or(""))
        .bind(operation)
        .bind(args)
        .bind(schema)
        .bind(now)
        .bind(now)
        .bind(format!("Run {key}/{operation}"))
        .execute(&mut *tx)
        .await?;
        // Inputs are Job-owned descriptors without an output Step key. Their
        // verified bytes were staged and renamed before this DB transaction.
        for item in imports {
            sqlx::query(
                "INSERT INTO job_artifacts(
                id,job_id,step_key,relative_path,media_type,size_bytes,sha256,filename,created_at
            ) VALUES(?,?,NULL,?,?,?,?,?,?)",
            )
            .bind(&item.id)
            .bind(id)
            .bind(format!("{id}/{}", item.id))
            .bind(&item.media_type)
            .bind(item.size_bytes)
            .bind(&item.sha256)
            .bind(&item.filename)
            .bind(now)
            .execute(&mut *tx)
            .await?;
        }
        if let Some(revision) = config_revision {
            sqlx::query(
                "INSERT INTO job_plugin_config_pins(job_id,plugin_key,revision)
                VALUES(?,?,?)",
            )
            .bind(id)
            .bind(key)
            .bind(revision)
            .execute(&mut *tx)
            .await?;
        }
        if let Some(pin) = &tts_pin {
            tts_provider::insert_pin(&mut tx, id, pin).await?;
        }
        let row: JobRow = sqlx::query_as("SELECT * FROM jobs WHERE id=?")
            .bind(id)
            .fetch_one(&mut *tx)
            .await?;
        let view = row.view(Vec::new(), Vec::new())?;
        tx.commit().await?;
        self.job_changes.publish_committed(&id);
        Ok(JobReceipt {
            job: view,
            replayed: false,
        })
    }

    /// Scheduler already claimed queued -> planning. No model call or Agent Run
    /// exists in this mode, so move to running and publish the prepared Step.
    pub(crate) async fn start_plugin_operation(&self, id: &str) -> Result<(), JobError> {
        let now = crate::database::unix_seconds().ok_or(JobError::ClockUnavailable)?;
        let changed = sqlx::query(
            "UPDATE jobs SET status='running',updated_at=?
             WHERE id=? AND execution_mode='plugin_operation_v3'
                AND status='planning' AND deadline_at>?",
        )
        .bind(now)
        .bind(id)
        .bind(now)
        .execute(self.pool())
        .await?;
        if changed.rows_affected() != 1 {
            return Err(JobError::Terminal);
        }
        self.job_changes.publish_committed(id);
        Ok(())
    }
}
