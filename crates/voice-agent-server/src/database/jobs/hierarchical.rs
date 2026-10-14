//! Durable V2 Agent Run ownership. All SQL stays behind the Database seam.
use super::*;
use crate::services::background::{AgentRunView, model::DelegatedTask, plugins::FrozenChoice};
use serde_json::{Value, json};

#[derive(sqlx::FromRow)]
struct RunRow {
    agent_run_id: String,
    parent_agent_run_id: Option<String>,
    fork_group_id: Option<String>,
    member_ordinal: Option<i64>,
    role: String,
    goal_excerpt: String,
    allowed_operations_json: String,
    status: String,
    result_json: Option<String>,
    error_code: Option<String>,
    started_at: Option<i64>,
    finished_at: Option<i64>,
}
impl RunRow {
    fn view(self) -> Result<AgentRunView, JobError> {
        let format = |seconds: Option<i64>| -> Result<Option<String>, JobError> {
            seconds
                .map(|value| {
                    chrono::DateTime::from_timestamp(value, 0)
                        .map(|timestamp| timestamp.to_rfc3339())
                        .ok_or(JobError::ClockUnavailable)
                })
                .transpose()
        };
        Ok(AgentRunView {
            agent_run_id: self.agent_run_id,
            parent_agent_run_id: self.parent_agent_run_id,
            fork_group_id: self.fork_group_id,
            member_ordinal: self.member_ordinal,
            role: self.role,
            goal_excerpt: self.goal_excerpt,
            allowed_operations: serde_json::from_str(&self.allowed_operations_json)
                .map_err(|_| JobError::DatabaseUnavailable)?,
            status: self.status,
            result: self
                .result_json
                .map(|raw| serde_json::from_str(&raw).map_err(|_| JobError::DatabaseUnavailable))
                .transpose()?,
            error_code: self.error_code,
            started_at: format(self.started_at)?,
            finished_at: format(self.finished_at)?,
        })
    }
}
pub(super) async fn read_runs(
    tx: &mut SqliteConnection,
    job_id: &str,
) -> Result<Vec<AgentRunView>, JobError> {
    let rows: Vec<RunRow> = sqlx::query_as(
        "SELECT * FROM job_agent_runs WHERE job_id=? ORDER BY created_at, CASE WHEN member_ordinal IS NULL THEN -1 ELSE member_ordinal END, agent_run_id"
    ).bind(job_id).fetch_all(&mut *tx).await?;
    rows.into_iter().map(RunRow::view).collect()
}
impl Database {
    pub(crate) async fn last_agent_decision(
        &self,
        job_id: &str,
        agent_id: &str,
    ) -> Result<String, JobError> {
        sqlx::query_scalar::<_,String>(
            "SELECT json_extract(payload_json,'$.run_id') FROM job_events WHERE job_id=? AND event_type='llm_run_started' AND json_extract(payload_json,'$.agent_run_id')=? ORDER BY sequence DESC LIMIT 1"
        ).bind(job_id).bind(agent_id).fetch_optional(self.pool()).await?
            .ok_or(JobError::InvalidPlan)
    }

    pub(crate) async fn create_root_run(&self, id: &str) -> Result<AgentRunView, JobError> {
        let now = crate::database::unix_seconds().ok_or(JobError::ClockUnavailable)?;
        let mut tx = self.pool().begin_with("BEGIN IMMEDIATE").await?;
        let job: JobRow = sqlx::query_as(
            "SELECT * FROM jobs WHERE id=? AND status='planning' AND execution_mode='hierarchical_v2' AND deadline_at>?"
        ).bind(id).bind(now).fetch_optional(&mut *tx).await?.ok_or(JobError::Terminal)?;
        let run_id = uuid::Uuid::new_v4().to_string();
        let goal =
            serde_json::from_str::<crate::services::background::JobRequest>(&job.request_json)
                .map_err(|_| JobError::DatabaseUnavailable)?
                .goal;
        let goal_excerpt: String = goal.chars().take(256).collect();
        sqlx::query("INSERT INTO job_agent_runs(agent_run_id,job_id,role,goal_excerpt,allowed_operations_json,status,created_at,started_at) VALUES(?,?,'orchestrator',?,'[]','running',?,?)")
            .bind(&run_id).bind(id).bind(goal_excerpt).bind(now).bind(now)
            .execute(&mut *tx).await?;
        let updated = sqlx::query(
            "UPDATE jobs SET status='running',updated_at=? WHERE id=? AND status='planning'",
        )
        .bind(now)
        .bind(id)
        .execute(&mut *tx)
        .await?;
        if updated.rows_affected() != 1 {
            return Err(JobError::Terminal);
        }
        events::append(
            &mut tx,
            id,
            "agent_run_created",
            None,
            &json!({"agent_run_id":run_id,"role":"orchestrator"}),
        )
        .await?;
        tx.commit().await?;
        self.job_changes.publish_committed(id);
        let mut connection = self.pool().acquire().await?;
        let row: RunRow = sqlx::query_as("SELECT * FROM job_agent_runs WHERE agent_run_id=?")
            .bind(&run_id)
            .fetch_one(&mut *connection)
            .await?;
        row.view()
    }

    pub(crate) async fn create_children_for_decision(
        &self,
        job_id: &str,
        root: &str,
        turn_id: &str,
        tasks: &[DelegatedTask],
    ) -> Result<Vec<AgentRunView>, JobError> {
        if tasks.is_empty() || tasks.len() > 2 || turn_id.len() > 64 {
            return Err(JobError::InvalidPlan);
        }
        let now = crate::database::unix_seconds().ok_or(JobError::ClockUnavailable)?;
        let mut tx = self.pool().begin_with("BEGIN IMMEDIATE").await?;
        let job:JobRow=sqlx::query_as("SELECT * FROM jobs WHERE id=? AND status='running' AND execution_mode='hierarchical_v2' AND deadline_at>?")
            .bind(job_id).bind(now).fetch_optional(&mut *tx).await?.ok_or(JobError::Terminal)?;
        let root_valid:i64=sqlx::query_scalar("SELECT COUNT(*) FROM job_agent_runs WHERE job_id=? AND agent_run_id=? AND parent_agent_run_id IS NULL AND status='running'")
            .bind(job_id).bind(root).fetch_one(&mut *tx).await?;
        if root_valid != 1 {
            return Err(JobError::InvalidPlan);
        }
        let existing:Vec<RunRow>=sqlx::query_as("SELECT * FROM job_agent_runs WHERE job_id=? AND fork_group_id=? ORDER BY member_ordinal")
            .bind(job_id).bind(turn_id).fetch_all(&mut *tx).await?;
        if !existing.is_empty() {
            if existing.len() != tasks.len() {
                return Err(JobError::InvalidPlan);
            }
            return existing.into_iter().map(RunRow::view).collect();
        }
        let unjoined:i64=sqlx::query_scalar("SELECT COUNT(*) FROM job_agent_runs a WHERE a.job_id=? AND a.fork_group_id IS NOT NULL AND NOT EXISTS(SELECT 1 FROM job_events e WHERE e.job_id=a.job_id AND e.event_type='join_completed' AND json_extract(e.payload_json,'$.fork_group_id')=a.fork_group_id)")
            .bind(job_id).fetch_one(&mut *tx).await?;
        if unjoined > 0 {
            return Err(JobError::InvalidPlan);
        }
        let snapshot: crate::services::background::JobSnapshot =
            serde_json::from_str(&job.snapshot_json).map_err(|_| JobError::DatabaseUnavailable)?;
        let choices: Vec<Value> = serde_json::from_str(&job.plugin_choices_json)
            .map_err(|_| JobError::DatabaseUnavailable)?;
        let mut seen = std::collections::HashSet::new();
        for task in tasks {
            if task.goal.trim().is_empty()
                || task.goal.chars().count() > 256
                || task.task_key.is_empty()
                || task.task_key.len() > 64
                || !task
                    .task_key
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || c == b'_' || c == b'-')
                || !seen.insert(&task.task_key)
                || !matches!(task.role.as_str(), "research" | "document" | "general")
                || task.allowed_operations.len() > 16
                || task.allowed_operations.iter().any(|op| {
                    !choices.iter().any(|c| {
                        format!(
                            "{}.{}",
                            c["plugin_id"].as_str().unwrap_or(""),
                            c["operation"].as_str().unwrap_or("")
                        ) == *op
                    })
                })
            {
                return Err(JobError::InvalidPlan);
            }
        }
        let count:i64=sqlx::query_scalar("SELECT COUNT(*) FROM job_agent_runs WHERE job_id=? AND parent_agent_run_id IS NOT NULL")
            .bind(job_id).fetch_one(&mut *tx).await?;
        if count + tasks.len() as i64 > i64::from(snapshot.limits.max_child_runs) {
            return Err(JobError::CapacityUnavailable);
        }
        let mut created = Vec::new();
        for (ordinal, task) in tasks.iter().enumerate() {
            let run_id = uuid::Uuid::new_v4().to_string();
            let excerpt: String = task.goal.chars().take(256).collect();
            sqlx::query("INSERT INTO job_agent_runs(agent_run_id,job_id,parent_agent_run_id,fork_group_id,member_ordinal,role,goal_excerpt,allowed_operations_json,status,created_at) VALUES(?,?,?,?,?,?,?,?,'queued',?)")
                .bind(&run_id).bind(job_id).bind(root).bind(turn_id).bind(ordinal as i64)
                .bind(&task.role).bind(excerpt).bind(serde_json::to_string(&task.allowed_operations).map_err(|_|JobError::InvalidPlan)?)
                .bind(now).execute(&mut *tx).await?;
            events::append(
                &mut tx,
                job_id,
                "agent_run_created",
                None,
                &json!({
                    "agent_run_id":run_id,"parent_agent_run_id":root,"fork_group_id":turn_id,
                    "member_ordinal":ordinal,"role":task.role
                }),
            )
            .await?;
            created.push(run_id);
        }
        tx.commit().await?;
        self.job_changes.publish_committed(job_id);
        let mut connection = self.pool().acquire().await?;
        let mut results = Vec::new();
        for id in created {
            let row: RunRow =
                sqlx::query_as("SELECT * FROM job_agent_runs WHERE job_id=? AND agent_run_id=?")
                    .bind(job_id)
                    .bind(id)
                    .fetch_one(&mut *connection)
                    .await?;
            results.push(row.view()?);
        }
        Ok(results)
    }

    pub(crate) async fn start_child_run(&self, job_id: &str, run_id: &str) -> Result<(), JobError> {
        let now = crate::database::unix_seconds().ok_or(JobError::ClockUnavailable)?;
        let updated=sqlx::query("UPDATE job_agent_runs SET status='running',started_at=? WHERE job_id=? AND agent_run_id=? AND status='queued' AND EXISTS(SELECT 1 FROM jobs WHERE id=? AND status='running' AND deadline_at>?)")
            .bind(now).bind(job_id).bind(run_id).bind(job_id).bind(now)
            .execute(self.pool()).await?;
        if updated.rows_affected() != 1 {
            return Err(JobError::Terminal);
        }
        self.job_changes.publish_committed(job_id);
        Ok(())
    }

    pub(crate) async fn finish_child_run(
        &self,
        job_id: &str,
        run_id: &str,
        status: &str,
        result: Option<&Value>,
        error: Option<&str>,
    ) -> Result<(), JobError> {
        if !matches!(status, "succeeded" | "failed" | "cancelled" | "interrupted") {
            return Err(JobError::InvalidArguments);
        }
        let encoded = result
            .map(serde_json::to_string)
            .transpose()
            .map_err(|_| JobError::InvalidArguments)?;
        if encoded.as_ref().is_some_and(|value| value.len() > 4096) {
            return Err(JobError::CapacityUnavailable);
        }
        let now = crate::database::unix_seconds().ok_or(JobError::ClockUnavailable)?;
        let mut tx = self.pool().begin_with("BEGIN IMMEDIATE").await?;
        let changed=sqlx::query("UPDATE job_agent_runs SET status=?,result_json=?,error_code=?,finished_at=? WHERE job_id=? AND agent_run_id=? AND parent_agent_run_id IS NOT NULL AND status IN ('queued','running')")
            .bind(status).bind(encoded).bind(error).bind(now).bind(job_id).bind(run_id)
            .execute(&mut *tx).await?;
        if changed.rows_affected() != 1 {
            return Err(JobError::Terminal);
        }
        events::append(
            &mut tx,
            job_id,
            "agent_run_finished",
            None,
            &json!({
                "agent_run_id":run_id,"status":status,"error_code":error
            }),
        )
        .await?;
        tx.commit().await?;
        self.job_changes.publish_committed(job_id);
        Ok(())
    }

    pub(crate) async fn join_children(
        &self,
        job_id: &str,
        root_id: &str,
        fork_id: &str,
    ) -> Result<Vec<AgentRunView>, JobError> {
        let mut tx = self.pool().begin_with("BEGIN IMMEDIATE").await?;
        let rows:Vec<RunRow>=sqlx::query_as("SELECT * FROM job_agent_runs WHERE job_id=? AND parent_agent_run_id=? AND fork_group_id=? ORDER BY member_ordinal")
            .bind(job_id).bind(root_id).bind(fork_id).fetch_all(&mut *tx).await?;
        if rows.is_empty()
            || rows
                .iter()
                .any(|r| matches!(r.status.as_str(), "queued" | "running"))
        {
            return Err(JobError::InvalidPlan);
        }
        let old:i64=sqlx::query_scalar("SELECT COUNT(*) FROM job_events WHERE job_id=? AND event_type='join_completed' AND json_extract(payload_json,'$.fork_group_id')=?")
            .bind(job_id).bind(fork_id).fetch_one(&mut *tx).await?;
        if old == 0 {
            let success = rows.iter().filter(|r| r.status == "succeeded").count();
            events::append(
                &mut tx,
                job_id,
                "join_completed",
                None,
                &json!({
                    "fork_group_id":fork_id,"root_agent_run_id":root_id,
                    "member_count":rows.len(),"succeeded":success,"failed":rows.len()-success
                }),
            )
            .await?;
        }
        tx.commit().await?;
        if old == 0 {
            self.job_changes.publish_committed(job_id);
        }
        rows.into_iter().map(RunRow::view).collect()
    }

    pub(crate) async fn insert_dynamic_step(
        &self,
        job_id: &str,
        agent_id: &str,
        turn_id: &str,
        choice: &FrozenChoice,
        args: &Value,
        output_schema: &Value,
    ) -> Result<String, JobError> {
        if args.to_string().len() > 16384
            || output_schema.to_string().len() > 8192
            || !args.is_object()
        {
            return Err(JobError::InvalidPlan);
        }
        let now = crate::database::unix_seconds().ok_or(JobError::ClockUnavailable)?;
        let key = format!("t_{}", turn_id.replace('-', ""));
        if key.len() > 64 {
            return Err(JobError::InvalidPlan);
        }
        let mut tx = self.pool().begin_with("BEGIN IMMEDIATE").await?;
        let job:JobRow=sqlx::query_as("SELECT * FROM jobs WHERE id=? AND status='running' AND execution_mode='hierarchical_v2' AND deadline_at>?")
            .bind(job_id).bind(now).fetch_optional(&mut *tx).await?.ok_or(JobError::Terminal)?;
        let choices: Vec<Value> = serde_json::from_str(&job.plugin_choices_json)
            .map_err(|_| JobError::DatabaseUnavailable)?;
        if !choices
            .iter()
            .any(|v| crate::tools::core::matches_frozen(v, choice))
        {
            return Err(JobError::CapabilityUnavailable);
        }
        let run:Option<String>=sqlx::query_scalar("SELECT allowed_operations_json FROM job_agent_runs WHERE job_id=? AND agent_run_id=? AND parent_agent_run_id IS NOT NULL AND status='running'")
            .bind(job_id).bind(agent_id).fetch_optional(&mut *tx).await?;
        let allowed: Vec<String> = serde_json::from_str(&run.ok_or(JobError::InvalidPlan)?)
            .map_err(|_| JobError::DatabaseUnavailable)?;
        if !allowed.contains(&format!("{}.{}", choice.plugin_id, choice.operation)) {
            return Err(JobError::CapabilityUnavailable);
        }
        let turn:i64=sqlx::query_scalar("SELECT COUNT(*) FROM job_events WHERE job_id=? AND event_type='llm_run_started' AND json_extract(payload_json,'$.run_id')=? AND json_extract(payload_json,'$.agent_run_id')=?")
            .bind(job_id).bind(turn_id).bind(agent_id).fetch_one(&mut *tx).await?;
        if turn != 1 {
            return Err(JobError::InvalidPlan);
        }
        sqlx::query("INSERT INTO job_steps(job_id,step_key,plan_revision,plugin_id,plugin_version,package_digest,installation_digest,operation,arguments_json,output_schema_json,required,instruction,status,created_at,updated_at,agent_run_id,agent_turn_id) VALUES(?,?,0,?,?,?,?,?,?,?,1,?,'pending',?,?,?,?)")
            .bind(job_id).bind(&key).bind(&choice.plugin_id).bind(&choice.version)
            .bind(&choice.package_digest).bind(&choice.installation_digest).bind(&choice.operation)
            .bind(serde_json::to_string(args).map_err(|_|JobError::InvalidPlan)?)
            .bind(serde_json::to_string(output_schema).map_err(|_|JobError::InvalidPlan)?)
            .bind(format!("{} / {}",choice.plugin_id,choice.operation))
            .bind(now).bind(now).bind(agent_id).bind(turn_id)
            .execute(&mut *tx).await?;
        tx.commit().await?;
        self.job_changes.publish_committed(job_id);
        Ok(key)
    }
}
