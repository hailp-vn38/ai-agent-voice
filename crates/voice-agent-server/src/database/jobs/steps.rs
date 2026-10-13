use super::*;
use serde_json::Value;

#[derive(FromRow)]
struct StepRow {
    step_key: String,
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
pub(super) async fn read(connection: &mut SqliteConnection, id: &str) -> Result<Vec<JobStepView>, JobError> {
    let rows: Vec<StepRow> =
        sqlx::query_as("SELECT * FROM job_steps WHERE job_id=? ORDER BY step_key")
            .bind(id)
            .fetch_all(&mut *connection)
            .await?;
    rows.into_iter()
        .map(|row| {
            Ok(JobStepView {
                step_key: row.step_key,
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
