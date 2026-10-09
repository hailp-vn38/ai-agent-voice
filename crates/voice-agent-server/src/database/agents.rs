//! Agent persistence and atomic audited mutations.
use super::{
    Database,
    audit::{AuditOutcome, audit, audit_conflict},
    writes::WriteError,
};
use serde::Serialize;
use sqlx::FromRow;

#[derive(Serialize, FromRow)]
pub(crate) struct Agent {
    pub(crate) id: i64,
    pub(crate) key: String,
    pub(crate) name: String,
    pub(crate) description: Option<String>,
    pub(crate) enabled: i64,
    pub(crate) revision: i64,
    pub(crate) created_at: i64,
    pub(crate) updated_at: i64,
}

pub(crate) struct NewAgent<'a> {
    pub key: &'a str,
    pub name: &'a str,
    pub description: Option<&'a str>,
}
pub(crate) struct AgentChanges<'a> {
    pub name: &'a str,
    pub description: Option<&'a str>,
    pub enabled: i64,
}
pub(crate) async fn get_agent_by(database: &Database, key: &str) -> Result<Agent, sqlx::Error> {
    sqlx::query_as("SELECT id,key,name,description,enabled,revision,created_at,updated_at FROM agents WHERE key=?").bind(key).fetch_one(&database.pool).await
}
impl Database {
    pub(crate) async fn create_agent(
        &self,
        input: NewAgent<'_>,
        request_id: &str,
    ) -> Result<(), WriteError> {
        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(|_| WriteError::Unavailable)?;
        let time = crate::database::unix_seconds().unwrap_or_default();
        let resource_id = sqlx::query(
            "INSERT INTO agents (key,name,description,created_at,updated_at) VALUES (?,?,?,?,?)",
        )
        .bind(input.key)
        .bind(input.name)
        .bind(input.description)
        .bind(time)
        .bind(time)
        .execute(&mut *tx)
        .await
        .map_err(WriteError::Mutation)?
        .last_insert_rowid();
        audit(
            &mut *tx,
            request_id,
            "agent",
            Some(resource_id),
            "create",
            None,
            Some(1),
            AuditOutcome::Success,
            1,
        )
        .await
        .map_err(|_| WriteError::Unavailable)?;
        tx.commit().await.map_err(|_| WriteError::Unavailable)
    }
    pub(crate) async fn list_agents(
        &self,
        enabled: Option<bool>,
        sort: Option<&str>,
        page: u32,
        size: u32,
    ) -> Result<Vec<Agent>, sqlx::Error> {
        let order = match sort.unwrap_or("key") {
            "name" => "name ASC",
            "-name" => "name DESC",
            "-key" => "key DESC",
            _ => "key ASC",
        };
        let sql = format!(
            "SELECT id,key,name,description,enabled,revision,created_at,updated_at FROM agents WHERE (? IS NULL OR enabled=?) ORDER BY {order} LIMIT ? OFFSET ?"
        );
        let enabled = enabled.map(i64::from);
        sqlx::query_as(&sql)
            .bind(enabled)
            .bind(enabled)
            .bind(i64::from(size))
            .bind(i64::from((page - 1) * size))
            .fetch_all(&self.pool)
            .await
    }
    pub(crate) async fn agent_conflict(&self, request_id: &str, id: i64, expected: i64) {
        audit_conflict(&self.pool, request_id.into(), "agent", id, expected).await;
    }
    pub(crate) async fn update_agent(
        &self,
        resource_id: i64,
        expected: i64,
        changes: AgentChanges<'_>,
        request_id: &str,
    ) -> Result<(), WriteError> {
        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(|_| WriteError::Unavailable)?;
        let result = sqlx::query("UPDATE agents SET name=?,description=?,enabled=?,revision=revision+1,updated_at=? WHERE id=? AND revision=?")
            .bind(changes.name).bind(changes.description).bind(changes.enabled).bind(crate::database::unix_seconds().unwrap_or_default()).bind(resource_id).bind(expected).execute(&mut *tx).await;
        // Preserve the endpoint's existing conflict mapping for failed conditional updates.
        if !matches!(result, Ok(ref result) if result.rows_affected() == 1) {
            let _ = tx.rollback().await;
            self.agent_conflict(request_id, resource_id, expected).await;
            return Err(WriteError::Conflict("revision_conflict"));
        }
        audit(
            &mut *tx,
            request_id,
            "agent",
            Some(resource_id),
            "update",
            Some(expected),
            Some(expected + 1),
            AuditOutcome::Success,
            1,
        )
        .await
        .map_err(|_| WriteError::Unavailable)?;
        tx.commit().await.map_err(|_| WriteError::Unavailable)
    }
}
