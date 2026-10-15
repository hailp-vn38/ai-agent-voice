//! Persisted activation for statically compiled Core groups; never an installation.
use super::Database;
use crate::tools::core::CorePluginSetting;
use thiserror::Error;

#[derive(Debug, Error)]
pub(crate) enum CorePluginPolicyError {
    #[error("not_found")]
    NotFound,
    #[error("revision_conflict")]
    RevisionConflict,
    #[error("database_unavailable")]
    Unavailable,
    #[error("database_error: {0}")]
    Sql(#[from] sqlx::Error),
}

impl Database {
    pub(crate) async fn core_plugin_settings(
        &self,
    ) -> Result<Vec<CorePluginSetting>, CorePluginPolicyError> {
        Ok(sqlx::query_as(
            "SELECT plugin_id,enabled,revision FROM core_plugin_settings ORDER BY plugin_id",
        )
        .fetch_all(self.pool())
        .await?)
    }

    /// Serialize publication with Job submission, and serialize concurrent
    /// PATCH writes with the same SQLite write lock used by Step start.
    /// Every successful CAS advances the revision, including same-value writes.
    pub(crate) async fn set_core_plugin_enabled(
        &self,
        id: &str,
        expected_revision: i64,
        enabled: bool,
    ) -> Result<CorePluginSetting, CorePluginPolicyError> {
        if !crate::tools::core::known_group(id) {
            return Err(CorePluginPolicyError::NotFound);
        }
        let _publication = self.tool_security.publication.write().await;
        let mut tx = self.pool().begin_with("BEGIN IMMEDIATE").await?;
        let exists: Option<i64> =
            sqlx::query_scalar("SELECT revision FROM core_plugin_settings WHERE plugin_id=?")
                .bind(id)
                .fetch_optional(&mut *tx)
                .await?;
        let Some(revision) = exists else {
            return Err(CorePluginPolicyError::Unavailable);
        };
        if revision != expected_revision {
            return Err(CorePluginPolicyError::RevisionConflict);
        }
        let changed = sqlx::query(
            "UPDATE core_plugin_settings SET enabled=?,revision=revision+1,updated_at=CAST(strftime('%s','now') AS INTEGER) WHERE plugin_id=? AND revision=?",
        )
        .bind(i64::from(enabled))
        .bind(id)
        .bind(expected_revision)
        .execute(&mut *tx)
        .await?;
        if changed.rows_affected() != 1 {
            return Err(CorePluginPolicyError::RevisionConflict);
        }
        let row: CorePluginSetting = sqlx::query_as(
            "SELECT plugin_id,enabled,revision FROM core_plugin_settings WHERE plugin_id=?",
        )
        .bind(id)
        .fetch_one(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(row)
    }
}
