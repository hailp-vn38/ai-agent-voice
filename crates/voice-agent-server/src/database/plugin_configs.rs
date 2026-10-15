//! Immutable plugin value-settings revisions. No activation flags live here.
use super::Database;
use serde_json::Value;
use sqlx::FromRow;

#[derive(Clone, Debug, FromRow)]
pub(crate) struct PluginConfigVersion {
    pub revision: i64,
    pub schema_digest: String,
    pub package_digest: Option<String>,
    pub public_json: String,
    pub secrets_json: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ConfigWriteError {
    Conflict,
    Storage,
}

impl Database {
    pub(crate) async fn plugin_config_version(
        &self,
        key: &str,
    ) -> Result<Option<PluginConfigVersion>, sqlx::Error> {
        sqlx::query_as::<_, PluginConfigVersion>(
            "SELECT v.revision,v.schema_digest,v.package_digest,v.public_json,v.secrets_json
             FROM plugin_configs c JOIN plugin_config_versions v
              ON v.plugin_key=c.plugin_key AND v.revision=c.current_revision
             WHERE c.plugin_key=?",
        )
        .bind(key)
        .fetch_optional(self.pool())
        .await
    }

    pub(crate) async fn pinned_plugin_config(
        &self,
        job_id: &str,
        key: &str,
    ) -> Result<Option<PluginConfigVersion>, sqlx::Error> {
        sqlx::query_as::<_, PluginConfigVersion>(
            "SELECT v.revision,v.schema_digest,v.package_digest,v.public_json,v.secrets_json
             FROM job_plugin_config_pins p JOIN plugin_config_versions v
               ON v.plugin_key=p.plugin_key AND v.revision=p.revision
             WHERE p.job_id=? AND p.plugin_key=?",
        )
        .bind(job_id)
        .bind(key)
        .fetch_optional(self.pool())
        .await
    }

    /// CAS against a distinct value-settings revision. The immutable record and
    /// current pointer commit together; no concurrent writer can overwrite a pin.
    pub(crate) async fn write_plugin_config(
        &self,
        key: &str,
        expected: i64,
        schema_digest: &str,
        package_digest: Option<&str>,
        public: &Value,
        sealed: &Value,
    ) -> Result<i64, ConfigWriteError> {
        if expected < 0 {
            return Err(ConfigWriteError::Conflict);
        }
        let mut tx = self
            .pool()
            .begin_with("BEGIN IMMEDIATE")
            .await
            .map_err(|_| ConfigWriteError::Storage)?;
        let existing: Option<i64> =
            sqlx::query_scalar("SELECT current_revision FROM plugin_configs WHERE plugin_key=?")
                .bind(key)
                .fetch_optional(&mut *tx)
                .await
                .map_err(|_| ConfigWriteError::Storage)?;
        if existing.unwrap_or(0) != expected {
            return Err(ConfigWriteError::Conflict);
        }
        let next = expected.checked_add(1).ok_or(ConfigWriteError::Conflict)?;
        let now = super::unix_seconds().ok_or(ConfigWriteError::Storage)?;
        if existing.is_none() {
            sqlx::query(
                "INSERT INTO plugin_configs(plugin_key,current_revision,updated_at) VALUES(?,?,?)",
            )
            .bind(key)
            .bind(next)
            .bind(now)
            .execute(&mut *tx)
            .await
            .map_err(|_| ConfigWriteError::Storage)?;
        } else {
            sqlx::query("UPDATE plugin_configs SET current_revision=?,updated_at=? WHERE plugin_key=? AND current_revision=?")
                .bind(next).bind(now).bind(key).bind(expected).execute(&mut *tx).await
                .map_err(|_|ConfigWriteError::Storage)?;
        }
        sqlx::query(
            "INSERT INTO plugin_config_versions(plugin_key,revision,schema_digest,package_digest,public_json,secrets_json,created_at)
             VALUES(?,?,?,?,?,?,?)"
        ).bind(key).bind(next).bind(schema_digest).bind(package_digest)
         .bind(serde_json::to_string(public).map_err(|_|ConfigWriteError::Storage)?)
         .bind(serde_json::to_string(sealed).map_err(|_|ConfigWriteError::Storage)?)
         .bind(now).execute(&mut *tx).await.map_err(|_|ConfigWriteError::Storage)?;
        tx.commit().await.map_err(|_| ConfigWriteError::Storage)?;
        Ok(next)
    }
}
