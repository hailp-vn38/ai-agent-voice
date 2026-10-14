//! Plugin installation persistence and transaction boundaries.
use super::Database;
use crate::services::background::plugins::{FrozenChoice, PluginError};
use sqlx::FromRow;

#[derive(FromRow)]
pub(crate) struct Installation {
    pub(crate) plugin_id: String,
    pub(crate) version: String,
    pub(crate) package_digest: String,
    pub(crate) installation_digest: String,
    pub(crate) status: String,
    pub(crate) revision: i64,
    pub(crate) operations_json: String,
    pub(crate) last_error_code: Option<String>,
}

impl Database {
    pub(crate) async fn plugin_installation(&self, id: &str) -> Result<Installation, PluginError> {
        sqlx::query_as("SELECT * FROM plugin_installations WHERE plugin_id=?")
            .bind(id)
            .fetch_optional(self.pool())
            .await?
            .ok_or(PluginError::NotFound)
    }
    pub(crate) async fn list_plugin_installations(
        &self,
        page: u32,
        size: u32,
        status: Option<&str>,
        pattern: Option<&str>,
    ) -> Result<(Vec<Installation>, i64), PluginError> {
        let total: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM plugin_installations WHERE (? IS NULL OR status=?) AND (? IS NULL OR plugin_id LIKE ? ESCAPE '\\')")
            .bind(status).bind(status).bind(pattern).bind(pattern).fetch_one(self.pool()).await?;
        let rows: Vec<Installation> = sqlx::query_as("SELECT * FROM plugin_installations WHERE (? IS NULL OR status=?) AND (? IS NULL OR plugin_id LIKE ? ESCAPE '\\') ORDER BY plugin_id LIMIT ? OFFSET ?")
            .bind(status).bind(status).bind(pattern).bind(pattern).bind(size).bind(i64::from(page-1)*i64::from(size)).fetch_all(self.pool()).await?;
        Ok((rows, total))
    }
    /// Keep the pin check, package publication and row update under one write transaction.
    pub(crate) async fn publish_plugin_installation<F, Fut>(
        &self,
        id: &str,
        version: &str,
        digest: &str,
        installation: &str,
        manifest: &str,
        publish: F,
    ) -> Result<Installation, PluginError>
    where
        F: FnOnce() -> Fut,
        Fut: std::future::Future<Output = Result<(), PluginError>>,
    {
        let _publication = self.tool_security.publication.write().await;
        let mut tx = self.pool().begin_with("BEGIN IMMEDIATE").await?;
        let pinned: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM jobs WHERE status IN ('queued','planning','running','cancelling') AND EXISTS (SELECT 1 FROM json_each(jobs.plugin_choices_json) WHERE json_extract(value,'$.plugin_id')=?)")
            .bind(id).fetch_one(&mut *tx).await?;
        if pinned != 0 {
            return Err(PluginError::InUse);
        }
        publish().await?;
        let now = crate::database::unix_seconds().ok_or(PluginError::DatabaseUnavailable)?;
        sqlx::query("INSERT INTO plugin_installations(plugin_id,version,package_digest,installation_digest,status,revision,operations_json,created_at,updated_at) VALUES(?,?,?,?,'disabled',1,?,?,?) ON CONFLICT(plugin_id) DO UPDATE SET version=excluded.version,package_digest=excluded.package_digest,installation_digest=excluded.installation_digest,status='disabled',revision=plugin_installations.revision+1,operations_json=excluded.operations_json,last_error_code=NULL,updated_at=excluded.updated_at")
            .bind(id).bind(version).bind(digest).bind(installation).bind(manifest).bind(now).bind(now).execute(&mut *tx).await?;
        tx.commit().await?;
        self.plugin_installation(id).await
    }
    pub(crate) async fn activate_plugin_installation(
        &self,
        id: &str,
        expected: i64,
    ) -> Result<Installation, PluginError> {
        let _publication = self.tool_security.publication.write().await;
        let result = sqlx::query("UPDATE plugin_installations SET status='ready',revision=revision+1,last_error_code=NULL,updated_at=? WHERE plugin_id=? AND revision=? AND status IN ('disabled','error')")
            .bind(crate::database::unix_seconds()).bind(id).bind(expected).execute(self.pool()).await?;
        if result.rows_affected() != 1 {
            return Err(PluginError::RevisionConflict);
        }
        self.plugin_installation(id).await
    }
    pub(crate) async fn disable_plugin_installation(
        &self,
        id: &str,
        expected: i64,
    ) -> Result<Installation, PluginError> {
        let _publication = self.tool_security.publication.write().await;
        let result = sqlx::query("UPDATE plugin_installations SET status='disabled',revision=revision+1,updated_at=? WHERE plugin_id=? AND revision=?")
            .bind(crate::database::unix_seconds()).bind(id).bind(expected).execute(self.pool()).await?;
        if result.rows_affected() != 1 {
            return Err(PluginError::RevisionConflict);
        }
        self.plugin_installation(id).await
    }
    pub(crate) async fn uninstall_plugin(
        &self,
        id: &str,
        expected: i64,
    ) -> Result<(), PluginError> {
        let _publication = self.tool_security.publication.write().await;
        let mut tx = self.pool().begin_with("BEGIN IMMEDIATE").await?;
        let row: Option<Installation> =
            sqlx::query_as("SELECT * FROM plugin_installations WHERE plugin_id=?")
                .bind(id)
                .fetch_optional(&mut *tx)
                .await?;
        let row = row.ok_or(PluginError::NotFound)?;
        if row.revision != expected {
            return Err(PluginError::RevisionConflict);
        }
        let pinned: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM jobs WHERE status IN ('queued','planning','running','cancelling') AND EXISTS (SELECT 1 FROM json_each(jobs.plugin_choices_json) WHERE json_extract(value,'$.plugin_id')=?)")
            .bind(id).fetch_one(&mut *tx).await?;
        if pinned != 0 {
            return Err(PluginError::InUse);
        }
        sqlx::query("DELETE FROM plugin_installations WHERE plugin_id=?")
            .bind(id)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(())
    }
    pub(crate) async fn fail_plugin_installation(
        &self,
        choice: &FrozenChoice,
        code: &str,
    ) -> Result<(), PluginError> {
        let _publication = self.tool_security.publication.write().await;
        sqlx::query("UPDATE plugin_installations SET status='error',last_error_code=?,revision=revision+1,updated_at=? WHERE plugin_id=? AND version=? AND package_digest=? AND installation_digest=? AND status='ready'")
            .bind(code).bind(crate::database::unix_seconds()).bind(&choice.plugin_id)
            .bind(&choice.version).bind(&choice.package_digest).bind(&choice.installation_digest)
            .execute(self.pool()).await?;
        Ok(())
    }
}

pub(in crate::database) async fn ready_installations(
    connection: &mut sqlx::SqliteConnection,
) -> Result<Vec<Installation>, PluginError> {
    sqlx::query_as(
        "SELECT * FROM plugin_installations WHERE status='ready' ORDER BY plugin_id LIMIT 33",
    )
    .fetch_all(connection)
    .await
    .map_err(PluginError::from)
}
