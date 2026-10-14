use super::*;
use serde_json::Value;

#[derive(FromRow)]
struct StepRow {
    step_key: String,
    agent_run_id: Option<String>,
    agent_turn_id: Option<String>,
    status: String,
    plugin_id: Option<String>,
    operation: Option<String>,
    error_code: Option<String>,
    attempt_state: Option<String>,
    instruction: String,
    required: bool,
    dependencies_json: String,
    input_refs_json: String,
    arguments_json: String,
    output_schema_json: String,
    result_json: Option<String>,
}
pub(super) async fn read(
    connection: &mut SqliteConnection,
    id: &str,
) -> Result<Vec<JobStepView>, JobError> {
    let rows: Vec<StepRow> =
        sqlx::query_as("SELECT * FROM job_steps WHERE job_id=? ORDER BY step_key")
            .bind(id)
            .fetch_all(&mut *connection)
            .await?;
    rows.into_iter()
        .map(|row| {
            Ok(JobStepView {
                step_key: row.step_key,
                agent_run_id: row.agent_run_id,
                agent_turn_id: row.agent_turn_id,
                status: row.status,
                plugin_id: row.plugin_id,
                operation: row.operation,
                error_code: row.error_code,
                attempt_state: row.attempt_state,
                instruction: row.instruction,
                required: row.required,
                dependencies: serde_json::from_str(&row.dependencies_json)
                    .map_err(|_| JobError::DatabaseUnavailable)?,
                input_refs: serde_json::from_str(&row.input_refs_json)
                    .map_err(|_| JobError::DatabaseUnavailable)?,
                arguments: serde_json::from_str(&row.arguments_json)
                    .map_err(|_| JobError::DatabaseUnavailable)?,
                output_schema: serde_json::from_str(&row.output_schema_json)
                    .map_err(|_| JobError::DatabaseUnavailable)?,
                result: row
                    .result_json
                    .map(|raw| {
                        serde_json::from_str::<Value>(&raw)
                            .map_err(|_| JobError::DatabaseUnavailable)
                    })
                    .transpose()?,
            })
        })
        .collect()
}

impl Database {
    /// Core Operations may inspect only metadata associated with this Job.
    /// SQLx remains in the domain repository module, not under tools/.
    pub(crate) async fn job_artifact_metadata(
        &self,
        job_id: &str,
        artifact_id: &str,
    ) -> Result<Option<(String, String)>, JobError> {
        sqlx::query_as("SELECT media_type,sha256 FROM job_artifacts WHERE job_id=? AND id=?")
            .bind(job_id)
            .bind(artifact_id)
            .fetch_optional(self.pool())
            .await
            .map_err(Into::into)
    }

    /// Return only immutable artifacts created by this exact persisted Step.
    /// IDs are safe to share in a Child's matched Tool Result.
    pub(crate) async fn job_step_artifact_refs(
        &self,
        job_id: &str,
        step_key: &str,
    ) -> Result<Vec<String>, JobError> {
        sqlx::query_scalar(
            "SELECT a.id FROM job_artifacts a JOIN job_steps s ON s.job_id=a.job_id AND s.step_key=a.step_key \
             WHERE a.job_id=? AND a.step_key=? AND s.status='succeeded' ORDER BY a.id LIMIT 64",
        )
        .bind(job_id)
        .bind(step_key)
        .fetch_all(self.pool())
        .await
        .map_err(Into::into)
    }

    /// A Child may cite its own persisted evidence, never another fork member's.
    pub(crate) async fn v3_child_artifact_refs(
        &self,
        job_id: &str,
        agent_run_id: &str,
    ) -> Result<Vec<String>, JobError> {
        sqlx::query_scalar(
            "SELECT a.id FROM job_artifacts a JOIN job_steps s ON s.job_id=a.job_id AND s.step_key=a.step_key
             JOIN job_agent_runs owner ON owner.job_id=s.job_id AND owner.agent_run_id=s.agent_run_id
             JOIN job_agent_runs current ON current.job_id=owner.job_id AND current.session_id=owner.session_id
             WHERE a.job_id=? AND current.agent_run_id=? AND s.status='succeeded' ORDER BY a.id LIMIT 64"
        ).bind(job_id).bind(agent_run_id).fetch_all(self.pool()).await.map_err(Into::into)
    }

    pub(crate) async fn job_child_artifact_refs(
        &self,
        job_id: &str,
        agent_run_id: &str,
    ) -> Result<Vec<String>, JobError> {
        sqlx::query_scalar(
            "SELECT a.id FROM job_artifacts a JOIN job_steps s ON s.job_id=a.job_id AND s.step_key=a.step_key \
             WHERE a.job_id=? AND s.agent_run_id=? AND s.status='succeeded' ORDER BY a.id LIMIT 64",
        )
        .bind(job_id)
        .bind(agent_run_id)
        .fetch_all(self.pool())
        .await
        .map_err(Into::into)
    }

    pub(crate) async fn job_step_outcome(
        &self,
        job_id: &str,
        step_key: &str,
    ) -> Result<(String, Option<String>, Option<String>), JobError> {
        sqlx::query_as(
            "SELECT status,result_json,error_code FROM job_steps WHERE job_id=? AND step_key=?",
        )
        .bind(job_id)
        .bind(step_key)
        .fetch_one(self.pool())
        .await
        .map_err(Into::into)
    }
}
