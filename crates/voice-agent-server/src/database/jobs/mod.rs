use crate::{
    database::Database,
    services::background::{
        JobArtifactView, JobError, JobRequest, JobRequester, JobSnapshot, JobStepView, JobView,
    },
};
use sqlx::{FromRow, SqliteConnection};
mod recovery;
// P3 scheduler will consume these durable dispatch transitions.
#[allow(dead_code)]
mod execution;
mod orchestration;
mod steps;
mod submission;
pub(crate) use submission::SubmissionIdentity;

#[derive(FromRow)]
pub(crate) struct JobRow {
    pub id: String,
    pub requester_scope: String,
    pub device_incarnation: Option<i64>,
    pub agent_incarnation: Option<i64>,
    pub request_json: String,
    pub request_fingerprint: String,
    pub snapshot_json: String,
    pub plugin_choices_json: String,
    pub settings_revision: i64,
    pub status: String,
    pub deadline_at: i64,
    pub error_code: Option<String>,
    pub created_at: i64,
    pub scheduler_seen_at: Option<i64>,
    pub plan_revision: i64,
    pub retry_of_job_id: Option<String>,
    pub result_json: Option<String>,
    pub budget_used_json: String,
}
impl JobRow {
    pub(crate) fn requester(&self) -> Result<JobRequester, JobError> {
        match (self.device_incarnation, self.agent_incarnation) {
            (None, None) if self.requester_scope == "admin:deployment" => Ok(JobRequester::Admin),
            (Some(device_incarnation), Some(agent_incarnation)) => Ok(JobRequester::Voice {
                device_incarnation,
                agent_incarnation,
            }),
            _ => Err(JobError::AuthorityRevoked),
        }
    }
    pub(crate) fn view(
        self,
        steps: Vec<JobStepView>,
        artifacts: Vec<JobArtifactView>,
    ) -> Result<JobView, JobError> {
        Ok(JobView {
            id: self.id,
            request: serde_json::from_str::<JobRequest>(&self.request_json)
                .map_err(|_| JobError::DatabaseUnavailable)?,
            status: self.status,
            requester_scope: self.requester_scope,
            snapshot: serde_json::from_str::<JobSnapshot>(&self.snapshot_json)
                .map_err(|_| JobError::DatabaseUnavailable)?,
            plugin_choices: serde_json::from_str(&self.plugin_choices_json)
                .map_err(|_| JobError::DatabaseUnavailable)?,
            settings_revision: self.settings_revision,
            deadline_at: self.deadline_at,
            created_at: chrono::DateTime::from_timestamp(self.created_at, 0)
                .ok_or(JobError::ClockUnavailable)?
                .to_rfc3339(),
            error_code: self.error_code,
            scheduler_seen_at: self.scheduler_seen_at,
            plan_revision: self.plan_revision,
            retry_of_job_id: self.retry_of_job_id,
            result: self
                .result_json
                .map(|raw| serde_json::from_str(&raw).map_err(|_| JobError::DatabaseUnavailable))
                .transpose()?,
            budget_used: serde_json::from_str(&self.budget_used_json)
                .map_err(|_| JobError::DatabaseUnavailable)?,
            steps,
            artifacts,
        })
    }
}
pub(crate) async fn authorize(
    connection: &mut SqliteConnection,
    requester: &JobRequester,
) -> Result<(), JobError> {
    if let JobRequester::Voice {
        device_incarnation,
        agent_incarnation,
    } = requester
    {
        let allowed:i64=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM devices d JOIN agents a ON a.id=d.agent_id WHERE d.id=? AND a.id=? AND d.enabled=1 AND a.enabled=1)")
            .bind(device_incarnation).bind(agent_incarnation).fetch_one(connection).await?;
        if allowed != 1 {
            return Err(JobError::AuthorityRevoked);
        }
    }
    Ok(())
}
impl Database {
    pub(crate) async fn job_row(&self, id: &str) -> Result<JobRow, JobError> {
        sqlx::query_as("SELECT * FROM jobs WHERE id=?")
            .bind(id)
            .fetch_optional(self.pool())
            .await?
            .ok_or(JobError::NotFound)
    }
    pub(crate) async fn job_view(&self, id: &str) -> Result<JobView, JobError> {
        let row = self.job_row(id).await?;
        let steps = steps::read(self, id).await?;
        let artifacts = sqlx::query_as(
            "SELECT id,media_type,size_bytes,sha256 FROM job_artifacts WHERE job_id=? ORDER BY id",
        )
        .bind(id)
        .fetch_all(self.pool())
        .await?;
        row.view(steps, artifacts)
    }
    pub(crate) async fn queued_jobs(&self) -> Result<Vec<JobRow>, JobError> {
        Ok(sqlx::query_as(
            "SELECT * FROM jobs WHERE status='queued' ORDER BY created_at,id LIMIT 256",
        )
        .fetch_all(self.pool())
        .await?)
    }
    pub(crate) async fn list_jobs(
        &self,
        page: u32,
        size: u32,
        status: Option<&str>,
        q: Option<&str>,
    ) -> Result<(Vec<JobView>, i64), JobError> {
        let pattern = q.map(|q| {
            format!(
                "%{}%",
                q.replace('\\', "\\\\")
                    .replace('%', "\\%")
                    .replace('_', "\\_")
            )
        });
        let mut tx = self.pool().begin().await?;
        let total:i64=sqlx::query_scalar("SELECT COUNT(*) FROM jobs WHERE (? IS NULL OR status=?) AND (? IS NULL OR json_extract(request_json,'$.goal') LIKE ? ESCAPE '\\')")
            .bind(status).bind(status).bind(&pattern).bind(&pattern).fetch_one(&mut *tx).await?;
        let rows:Vec<JobRow>=sqlx::query_as("SELECT * FROM jobs WHERE (? IS NULL OR status=?) AND (? IS NULL OR json_extract(request_json,'$.goal') LIKE ? ESCAPE '\\') ORDER BY created_at DESC,id DESC LIMIT ? OFFSET ?")
            .bind(status).bind(status).bind(&pattern).bind(&pattern).bind(size).bind(i64::from(page-1)*i64::from(size)).fetch_all(&mut *tx).await?;
        tx.commit().await?;
        Ok((
            rows.into_iter()
                .map(|row| row.view(Vec::new(), Vec::new()))
                .collect::<Result<_, _>>()?,
            total,
        ))
    }
}
