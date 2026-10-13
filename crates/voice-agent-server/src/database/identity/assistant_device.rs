//! The System-owned Device; assignment, revision and audit change atomically.
use crate::database::{
    Database,
    audit::{AuditOutcome, audit, audit_conflict},
    devices::{Device, get_device_by, template_override_id},
    writes::WriteError,
};

impl Database {
    pub(crate) async fn assistant_device(&self) -> Result<Option<Device>, sqlx::Error> {
        let identity: Option<String> =
            sqlx::query_scalar("SELECT device_id FROM devices WHERE kind='web_assistant'")
                .fetch_optional(&self.pool)
                .await?;
        match identity {
            Some(id) => get_device_by(self, &id).await.map(Some),
            None => Ok(None),
        }
    }

    pub(crate) async fn save_assistant_device(
        &self,
        agent: &str,
        template: Option<&str>,
        enabled: bool,
        expected: Option<i64>,
        request_id: &str,
    ) -> Result<Device, WriteError> {
        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(|_| WriteError::Unavailable)?;
        // Reserve SQLite's write lock before reading the singleton/CAS, including first creation.
        sqlx::query("UPDATE devices SET revision=revision WHERE kind='web_assistant'")
            .execute(&mut *tx)
            .await
            .map_err(WriteError::Sql)?;
        let old: Option<(i64, String, i64)> =
            sqlx::query_as("SELECT id,device_id,revision FROM devices WHERE kind='web_assistant'")
                .fetch_optional(&mut *tx)
                .await
                .map_err(WriteError::Sql)?;
        if old.as_ref().map(|v| v.2) != expected {
            tx.rollback().await.map_err(WriteError::Sql)?;
            if let (Some((id, _, _)), Some(expected)) = (&old, expected) {
                audit_conflict(&self.pool, request_id.to_owned(), "device", *id, expected).await;
            }
            return Err(WriteError::Conflict("revision_conflict"));
        }
        // Disabling an unchanged binding must remain possible after its graph is disabled.
        let unchanged: Option<(i64, Option<i64>)> = if !enabled && old.is_some() {
            sqlx::query_as("SELECT d.agent_id,d.template_id FROM devices d JOIN agents a ON a.id=d.agent_id LEFT JOIN agent_templates t ON t.id=d.template_id WHERE d.kind='web_assistant' AND a.key=? AND t.key IS ?")
                .bind(agent).bind(template).fetch_optional(&mut *tx).await.map_err(WriteError::Sql)?
        } else {
            None
        };
        let (agent_id, template_id) = if let Some(binding) = unchanged {
            binding
        } else {
            let agent_id: Option<i64> =
                sqlx::query_scalar("SELECT id FROM agents WHERE key=? AND enabled=1")
                    .bind(agent)
                    .fetch_optional(&mut *tx)
                    .await
                    .map_err(WriteError::Sql)?;
            let agent_id = agent_id.ok_or(WriteError::Invalid("invalid_agent"))?;
            let template_id = template_override_id(&mut tx, agent_id, template)
                .await
                .map_err(|error| match error {
                    sqlx::Error::RowNotFound => WriteError::Invalid("invalid_template_override"),
                    e => WriteError::Sql(e),
                })?;
            (agent_id, template_id)
        };
        let now = crate::database::unix_seconds().unwrap_or_default();
        let (id, identity, revision) = if let Some((id, identity, revision)) = old {
            sqlx::query("UPDATE devices SET agent_id=?,template_id=?,enabled=?,revision=revision+1,updated_at=? WHERE id=? AND revision=?").bind(agent_id).bind(template_id).bind(enabled).bind(now).bind(id).bind(revision).execute(&mut *tx).await.map_err(WriteError::Sql)?;
            (id, identity, revision + 1)
        } else {
            let identity = format!("web_assistant_{}", uuid::Uuid::new_v4().simple());
            let result = sqlx::query("INSERT INTO devices(device_id,agent_id,template_id,name,enabled,kind,created_at,updated_at) VALUES (?,?,?,'Studio Assistant',?,'web_assistant',?,?)").bind(&identity).bind(agent_id).bind(template_id).bind(enabled).bind(now).bind(now).execute(&mut *tx).await.map_err(WriteError::Mutation)?;
            (result.last_insert_rowid(), identity, 1)
        };
        audit(
            &mut *tx,
            request_id,
            "device",
            Some(id),
            if expected.is_some() {
                "update"
            } else {
                "create"
            },
            expected,
            Some(revision),
            AuditOutcome::Success,
            1,
        )
        .await
        .map_err(WriteError::Sql)?;
        tx.commit().await.map_err(WriteError::Sql)?;
        get_device_by(self, &identity)
            .await
            .map_err(WriteError::Sql)
    }
}
