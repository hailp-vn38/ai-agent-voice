//! Device persistence, assignment validation and atomic enrollment cancellation.
use super::{
    Database,
    audit::{AuditOutcome, audit, audit_conflict},
    writes::WriteError,
};
use serde::Serialize;
use sqlx::{FromRow, Sqlite};
#[derive(Serialize, FromRow)]
pub(crate) struct Device {
    pub(crate) id: i64,
    pub(crate) device_id: String,
    pub(crate) agent_key: String,
    pub(crate) template_key: Option<String>,
    pub(crate) name: Option<String>,
    pub(crate) description: Option<String>,
    pub(crate) enabled: i64,
    pub(crate) metadata_json: Option<String>,
    pub(crate) revision: i64,
    pub(crate) created_at: i64,
    pub(crate) updated_at: i64,
}
pub(crate) async fn get_device_by(
    database: &Database,
    device_id: &str,
) -> Result<Device, sqlx::Error> {
    sqlx::query_as("SELECT d.id,d.device_id,a.key AS agent_key,t.key AS template_key,d.name,d.description,d.enabled,d.metadata_json,d.revision,d.created_at,d.updated_at FROM devices d JOIN agents a ON a.id=d.agent_id LEFT JOIN agent_templates t ON t.id=d.template_id WHERE d.device_id=?").bind(device_id).fetch_one(&database.pool).await
}
/// Returns the selected Template only when it is enabled and has an enabled assignment to the
/// resulting Device Agent.  This is duplicated at admission as a fail-closed integrity check.
pub(super) async fn template_override_id(
    tx: &mut sqlx::Transaction<'_, Sqlite>,
    agent_id: i64,
    key: Option<&str>,
) -> Result<Option<i64>, sqlx::Error> {
    let Some(key) = key else {
        return Ok(None);
    };
    sqlx::query_scalar("SELECT t.id FROM agent_template_assignments ata JOIN agent_templates t ON t.id=ata.template_id WHERE ata.agent_id=? AND t.key=? AND ata.enabled=1 AND t.enabled=1")
        .bind(agent_id).bind(key).fetch_optional(&mut **tx).await?.ok_or(sqlx::Error::RowNotFound).map(Some)
}

pub(crate) struct DeviceInput<'a> {
    pub device_id: &'a str,
    pub agent_key: &'a str,
    pub template_key: Option<&'a str>,
    pub name: Option<&'a str>,
    pub description: Option<&'a str>,
    pub metadata: Option<&'a str>,
}
pub(crate) struct DeviceChanges<'a> {
    pub agent_key: &'a str,
    pub template_key: Option<&'a str>,
    pub name: Option<&'a str>,
    pub description: Option<&'a str>,
    pub metadata: Option<&'a str>,
    pub enabled: i64,
}
impl Database {
    pub(crate) async fn create_device(
        &self,
        input: DeviceInput<'_>,
        request_id: &str,
    ) -> Result<(), WriteError> {
        let pool = &self.pool;
        let mut tx = match pool.begin().await {
            Ok(v) => v,
            Err(_) => {
                return Err(WriteError::Unavailable);
            }
        };
        let agent: Result<(i64, i64), _> =
            sqlx::query_as("SELECT id, enabled FROM agents WHERE key=?")
                .bind(input.agent_key)
                .fetch_one(&mut *tx)
                .await;
        let (agent_id, _) = match agent {
            Ok(value) if value.1 == 1 => value,
            _ => return Err(WriteError::Invalid("invalid_agent")),
        };
        let template_id = match template_override_id(&mut tx, agent_id, input.template_key).await {
            Ok(value) => value,
            Err(sqlx::Error::RowNotFound) => {
                return Err(WriteError::Invalid("invalid_template_override"));
            }
            Err(error_value) => return Err(WriteError::Sql(error_value)),
        };
        let time = crate::database::unix_seconds().unwrap_or_default();
        let result=sqlx::query("INSERT INTO devices (device_id,agent_id,template_id,name,description,metadata_json,created_at,updated_at) VALUES (?,?,?,?,?,?,?,?)").bind(input.device_id).bind(agent_id).bind(template_id).bind(input.name).bind(input.description).bind(input.metadata).bind(time).bind(time).execute(&mut *tx).await;
        let device_id = match result {
            Ok(v) => v.last_insert_rowid(),
            Err(error_value) => return Err(WriteError::Mutation(error_value)),
        };
        if self
            .cancel_pending_enrollment(&mut tx, input.device_id, time)
            .await
            .is_err()
        {
            return Err(WriteError::Unavailable);
        }
        if audit(
            &mut *tx,
            request_id,
            "device",
            Some(device_id),
            "create",
            None,
            Some(1),
            AuditOutcome::Success,
            1,
        )
        .await
        .is_err()
            || tx.commit().await.is_err()
        {
            return Err(WriteError::Unavailable);
        };
        Ok(())
    }
    pub(crate) async fn update_device(
        &self,
        resource_id: i64,
        expected: i64,
        input: DeviceChanges<'_>,
        request_id: &str,
    ) -> Result<(), WriteError> {
        let pool = &self.pool;
        let mut tx = match pool.begin().await {
            Ok(v) => v,
            Err(_) => {
                return Err(WriteError::Unavailable);
            }
        };
        let agent: Result<(i64, i64), _> =
            sqlx::query_as("SELECT id, enabled FROM agents WHERE key=?")
                .bind(input.agent_key)
                .fetch_one(&mut *tx)
                .await;
        let (agent_id, _) = match agent {
            Ok(value) if value.1 == 1 => value,
            _ => return Err(WriteError::Invalid("invalid_agent")),
        };
        let template_id = match template_override_id(&mut tx, agent_id, input.template_key).await {
            Ok(value) => value,
            Err(sqlx::Error::RowNotFound) => {
                return Err(WriteError::Invalid("invalid_template_override"));
            }
            Err(error_value) => return Err(WriteError::Sql(error_value)),
        };
        let update = sqlx::query("UPDATE devices SET agent_id=?,template_id=?,name=?,description=?,metadata_json=?,enabled=?,revision=revision+1,updated_at=? WHERE id=? AND revision=?").bind(agent_id).bind(template_id).bind(input.name).bind(input.description).bind(input.metadata).bind(input.enabled).bind(crate::database::unix_seconds().unwrap_or_default()).bind(resource_id).bind(expected).execute(&mut *tx).await;
        let update = match update {
            Ok(result) if result.rows_affected() == 1 => Ok(()),
            Ok(_) => Err(()),
            Err(_) => Err(()),
        };
        if update.is_err() {
            let _ = tx.rollback().await;
            audit_conflict(pool, request_id.to_owned(), "device", resource_id, expected).await;
            return Err(WriteError::Conflict("revision_conflict"));
        }
        if audit(
            &mut *tx,
            request_id,
            "device",
            Some(resource_id),
            "update",
            Some(expected),
            Some(expected + 1),
            AuditOutcome::Success,
            1,
        )
        .await
        .is_err()
            || tx.commit().await.is_err()
        {
            return Err(WriteError::Unavailable);
        };
        Ok(())
    }
    pub(crate) async fn list_devices(
        &self,
        enabled: Option<bool>,
        sort: Option<&str>,
        page: u32,
        page_size: u32,
    ) -> Result<Vec<Device>, sqlx::Error> {
        let enabled = enabled.map(i64::from);
        let order = match sort.unwrap_or("device_id") {
            "name" => "d.name ASC",
            "-name" => "d.name DESC",
            "-device_id" => "d.device_id DESC",
            _ => "d.device_id ASC",
        };
        let sql = format!(
            "SELECT d.id,d.device_id,a.key AS agent_key,t.key AS template_key,d.name,d.description,d.enabled,d.metadata_json,d.revision,d.created_at,d.updated_at FROM devices d JOIN agents a ON a.id=d.agent_id LEFT JOIN agent_templates t ON t.id=d.template_id WHERE (? IS NULL OR d.enabled=?) ORDER BY {order} LIMIT ? OFFSET ?"
        );
        sqlx::query_as::<_, Device>(&sql)
            .bind(enabled)
            .bind(enabled)
            .bind(i64::from(page_size))
            .bind(i64::from((page - 1) * page_size))
            .fetch_all(&self.pool)
            .await
    }
    pub(crate) async fn device_conflict(&self, request_id: &str, id: i64, expected: i64) {
        audit_conflict(&self.pool, request_id.into(), "device", id, expected).await;
    }
}
