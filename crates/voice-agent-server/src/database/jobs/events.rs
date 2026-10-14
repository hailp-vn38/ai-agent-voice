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
    ) -> Result<(), JobError> {
        let mut tx = self.pool().begin_with("BEGIN IMMEDIATE").await?;
        let status: Option<String> = sqlx::query_scalar("SELECT status FROM jobs WHERE id=?")
            .bind(id)
            .fetch_optional(&mut *tx)
            .await?;
        if !matches!(status.as_deref(), Some("planning" | "running")) {
            return Err(JobError::Terminal);
        }
        append(&mut tx,id,"plan_rejected",None,&json!({"attempt":attempt,"code":diagnostic.code,"path":diagnostic.path,"message":diagnostic.message})).await?;
        tx.commit().await?;
        self.job_changes.publish_committed(id);
        Ok(())
    }
}
