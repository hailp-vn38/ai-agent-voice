//! Append-only Job metadata timeline; mutations share the owner's SQLite transaction.
use super::*;
use serde::Serialize;
use serde_json::{Value, json};

#[derive(Debug, Serialize)]
pub(crate) struct JobEvent {
    pub sequence: i64,
    pub event_type: String,
    pub created_at: String,
    pub step_key: Option<String>,
    pub payload: Value,
}
pub(super) async fn append(
    connection: &mut SqliteConnection,
    job_id: &str,
    event_type: &str,
    step_key: Option<&str>,
    payload: &Value,
) -> Result<(), JobError> {
    let serialized = payload.to_string();
    if serialized.len() > 4096 {
        return Err(JobError::CapacityUnavailable);
    }
    let now = crate::database::unix_seconds().ok_or(JobError::ClockUnavailable)?;
    sqlx::query("INSERT INTO job_events(job_id,sequence,event_type,created_at,step_key,payload_json) VALUES(?,(SELECT COALESCE(MAX(sequence),0)+1 FROM job_events WHERE job_id=?),?,?,?,?)")
        .bind(job_id).bind(job_id).bind(event_type).bind(now).bind(step_key).bind(serialized)
        .execute(&mut *connection).await?;
    Ok(())
}
impl Database {
    pub(crate) async fn job_events(
        &self,
        id: &str,
        after: i64,
        limit: u32,
    ) -> Result<Vec<JobEvent>, JobError> {
        if after < 0 || !(1..=100).contains(&limit) {
            return Err(JobError::InvalidArguments);
        }
        // Explicit existence check also distinguishes unknown Jobs from empty old timelines.
        self.job_row(id).await?;
        let rows: Vec<(i64, String, i64, Option<String>, String)> =
            sqlx::query_as("SELECT sequence,event_type,created_at,step_key,payload_json FROM job_events WHERE job_id=? AND sequence>? ORDER BY sequence LIMIT ?")
                .bind(id).bind(after).bind(i64::from(limit)).fetch_all(self.pool()).await?;
        rows.into_iter()
            .map(|(sequence, event_type, created_at, step_key, raw)| {
                Ok(JobEvent {
                    sequence,
                    event_type,
                    created_at: chrono::DateTime::from_timestamp(created_at, 0)
                        .ok_or(JobError::ClockUnavailable)?
                        .to_rfc3339(),
                    step_key,
                    payload: serde_json::from_str(&raw)
                        .map_err(|_| JobError::DatabaseUnavailable)?,
                })
            })
            .collect()
    }
    pub(crate) async fn reject_plan(
        &self,
        id: &str,
        attempt: u32,
        diagnostic: &crate::services::background::plan::PlanValidationError,
        run_id: &str,
    ) -> Result<(), JobError> {
        let mut tx = self.pool().begin_with("BEGIN IMMEDIATE").await?;
        let status: Option<String> = sqlx::query_scalar("SELECT status FROM jobs WHERE id=?")
            .bind(id)
            .fetch_optional(&mut *tx)
            .await?;
        if !matches!(status.as_deref(), Some("planning" | "running")) {
            return Err(JobError::Terminal);
        }
        append(&mut tx,id,"plan_rejected",None,&json!({"attempt":attempt,"code":diagnostic.code,"path":diagnostic.path,"message":diagnostic.message,"run_id":run_id})).await?;
        tx.commit().await?;
        self.job_changes.publish_committed(id);
        Ok(())
    }
}

impl Database {
    pub(crate) async fn record_llm_activity(
        &self,
        run: &crate::services::background::model::LlmRunContext,
        event_type: &'static str,
        detail: Value,
    ) -> Result<(), JobError> {
        if !matches!(
            event_type,
            "llm_provider_acquired"
                | "llm_request_started"
                | "llm_response_received"
                | "llm_run_finished"
        ) {
            return Err(JobError::InvalidArguments);
        }
        let mut payload = json!({"run_id":run.run_id});
        if let Some(agent_id) = &run.agent_run_id {
            payload["agent_run_id"] = json!(agent_id);
            payload["turn_id"] = json!(run.run_id);
        }
        let fields = detail.as_object().ok_or(JobError::InvalidArguments)?;
        for (key, value) in fields {
            if key == "run_id"
                || (value.is_string() && value.as_str().is_some_and(|s| s.len() > 256))
            {
                return Err(JobError::InvalidArguments);
            }
            payload[key] = value.clone();
        }
        let mut tx = self.pool().begin_with("BEGIN IMMEDIATE").await?;
        append(
            &mut tx,
            &run.job_id,
            event_type,
            run.step_key.as_deref(),
            &payload,
        )
        .await?;
        tx.commit().await?;
        self.job_changes.publish_committed(&run.job_id);
        Ok(())
    }
}

impl Database {
    /// Persist a validated action or a safe rejection diagnostic independently
    /// of the LLM transport status. No raw model output is written to SQLite.
    pub(crate) async fn record_agent_decision(
        &self,
        run: &crate::services::background::model::LlmRunContext,
        accepted: bool,
        detail: Value,
    ) -> Result<(), JobError> {
        let fields = detail.as_object().ok_or(JobError::InvalidArguments)?;
        let valid = if accepted {
            fields
                .get("action")
                .and_then(Value::as_str)
                .is_some_and(|s| matches!(s, "delegate" | "finish" | "call_plugin"))
        } else {
            fields
                .get("validation_code")
                .and_then(Value::as_str)
                .is_some_and(|s| {
                    !s.is_empty()
                        && s.len() <= 64
                        && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
                })
                && fields
                    .get("validation_path")
                    .and_then(Value::as_str)
                    .is_some_and(|s| {
                        s.starts_with('$')
                            && s.len() <= 128
                            && s.bytes()
                                .all(|b| b.is_ascii_alphanumeric() || b"$.[]_-".contains(&b))
                    })
        };
        if !valid
            || fields.len() != 2 && accepted
            || fields.len() != 3 && !accepted
            || fields.get("attempt_no").and_then(Value::as_u64).is_none()
        {
            return Err(JobError::InvalidArguments);
        }
        let mut payload = json!({"run_id":run.run_id,"turn_id":run.run_id,
            "agent_run_id":run.agent_run_id,"role":run.role});
        for (key, value) in fields {
            payload[key] = value.clone();
        }
        let mut tx = self.pool().begin_with("BEGIN IMMEDIATE").await?;
        append(
            &mut tx,
            &run.job_id,
            if accepted {
                "agent_decision_accepted"
            } else {
                "agent_decision_rejected"
            },
            None,
            &payload,
        )
        .await?;
        tx.commit().await?;
        self.job_changes.publish_committed(&run.job_id);
        Ok(())
    }
}
