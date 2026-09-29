use crate::config::{DatabaseConfig, DatabaseDevicesConfig};
use sqlx::{
    SqlitePool,
    migrate::{MigrateError, Migrator},
    sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous},
};
use std::{
    str::FromStr,
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use thiserror::Error;

pub mod external_mcp_policy;
pub mod provider_config;
pub mod secrets;

static MIGRATOR: Migrator = sqlx::migrate!("./migrations");

#[derive(Clone)]
pub struct Database {
    pool: SqlitePool,
}

#[derive(Debug, Error)]
pub enum DatabaseError {
    #[error("database_busy")]
    Busy,
    #[error("database_pool_timeout")]
    PoolTimeout,
    #[error("database_unavailable")]
    Unavailable,
    #[error("database_schema_incompatible")]
    SchemaIncompatible,
    #[error("database_schema_pending")]
    SchemaPending,
    #[error("database_migration_failed")]
    Migration,
    #[error("provider_config_invalid")]
    ProviderConfig,
}

/// Immutable database facts captured before a WebSocket is upgraded.
///
/// Later admission/profile tickets extend this snapshot; keeping it outside the realtime actor
/// ensures the actor never owns a pool or needs to query SQLite.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeviceAdmission {
    pub device_db_id: i64,
    pub agent_db_id: i64,
    pub uses_server_defaults: bool,
}

#[derive(Debug, Error)]
pub enum DeviceAdmissionError {
    #[error("device_not_admitted")]
    Denied,
    #[error("device_admission_unavailable")]
    Unavailable,
}

impl Database {
    pub async fn connect(config: &DatabaseConfig) -> Result<Self, DatabaseError> {
        let options = SqliteConnectOptions::from_str(&config.url)
            .map_err(map_sqlx_error)?
            .create_if_missing(true)
            .foreign_keys(true)
            .journal_mode(SqliteJournalMode::Wal)
            .synchronous(SqliteSynchronous::Normal)
            .busy_timeout(Duration::from_millis(config.busy_timeout_ms));
        let pool = SqlitePoolOptions::new()
            .max_connections(config.max_connections)
            .connect_with(options)
            .await
            .map_err(map_sqlx_error)?;

        ensure_schema_not_newer_than_binary(&pool).await?;
        if config.migrate_on_start {
            MIGRATOR.run(&pool).await.map_err(map_migration_error)?;
        } else {
            ensure_schema_is_current(&pool).await?;
        }
        Ok(Self { pool })
    }

    pub async fn connect_if_enabled(
        config: &DatabaseConfig,
    ) -> Result<Option<Self>, DatabaseError> {
        if config.enabled {
            Self::connect(config).await.map(Some)
        } else {
            Ok(None)
        }
    }

    pub fn pool(&self) -> &SqlitePool {
        &self.pool
    }

    /// Resolves the Device and its enabled Agent in one pre-upgrade database operation.
    /// Unknown identities can be provisioned only through the explicit dev/migration switch.
    pub async fn admit_device(
        &self,
        device_id: &str,
        devices: &DatabaseDevicesConfig,
    ) -> Result<DeviceAdmission, DeviceAdmissionError> {
        match self.find_admission(device_id).await? {
            Some(admission) => Ok(admission),
            None if devices.auto_register => self.auto_register_and_admit(device_id, devices).await,
            None => Err(DeviceAdmissionError::Denied),
        }
    }

    async fn find_admission(
        &self,
        device_id: &str,
    ) -> Result<Option<DeviceAdmission>, DeviceAdmissionError> {
        let row = sqlx::query_as::<_, (i64, i64, i64, i64)>(
            "SELECT d.id, a.id, d.enabled, a.enabled \
             FROM devices d JOIN agents a ON a.id = d.agent_id WHERE d.device_id = ?",
        )
        .bind(device_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(map_admission_error)?;
        let Some((device_db_id, agent_db_id, device_enabled, agent_enabled)) = row else {
            return Ok(None);
        };
        if device_enabled == 0 || agent_enabled == 0 {
            return Err(DeviceAdmissionError::Denied);
        }
        let assignments: i64 = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM agent_template_assignments \
             WHERE agent_id = ? AND enabled = 1)",
        )
        .bind(agent_db_id)
        .fetch_one(&self.pool)
        .await
        .map_err(map_admission_error)?;
        if assignments != 0 {
            // Template profile resolution is intentionally introduced by Ticket 07. Do not
            // accept a partial profile or fall back to deployment defaults here.
            return Err(DeviceAdmissionError::Unavailable);
        }
        Ok(Some(DeviceAdmission {
            device_db_id,
            agent_db_id,
            uses_server_defaults: true,
        }))
    }

    async fn auto_register_and_admit(
        &self,
        device_id: &str,
        devices: &DatabaseDevicesConfig,
    ) -> Result<DeviceAdmission, DeviceAdmissionError> {
        let registered_at = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| DeviceAdmissionError::Unavailable)?
            .as_secs() as i64;
        let metadata = serde_json::json!({
            "source": "auto_register",
            "registered_at": registered_at,
        })
        .to_string();
        sqlx::query(
            "INSERT OR IGNORE INTO devices \
             (device_id, agent_id, enabled, metadata_json, created_at, updated_at) \
             SELECT ?, id, 1, ?, ?, ? FROM agents WHERE key = ? AND enabled = 1",
        )
        .bind(device_id)
        .bind(metadata)
        .bind(registered_at)
        .bind(registered_at)
        .bind(&devices.auto_register_agent_key)
        .execute(&self.pool)
        .await
        .map_err(map_admission_error)?;
        // A racing connection may have inserted first. Re-read and enforce the same policy
        // rather than assuming the winner's binding is safe for this connection.
        self.find_admission(device_id)
            .await?
            .ok_or(DeviceAdmissionError::Denied)
    }
}

fn map_admission_error(error: sqlx::Error) -> DeviceAdmissionError {
    match map_sqlx_error(error) {
        DatabaseError::Busy | DatabaseError::PoolTimeout | DatabaseError::Unavailable => {
            DeviceAdmissionError::Unavailable
        }
        DatabaseError::SchemaIncompatible
        | DatabaseError::SchemaPending
        | DatabaseError::Migration
        | DatabaseError::ProviderConfig => DeviceAdmissionError::Unavailable,
    }
}

async fn applied_versions(pool: &SqlitePool) -> Result<Vec<i64>, DatabaseError> {
    let migration_table_exists: i64 = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = '_sqlx_migrations')",
    )
    .fetch_one(pool)
    .await
    .map_err(map_sqlx_error)?;
    if migration_table_exists == 0 {
        return Ok(Vec::new());
    }
    let failed: i64 =
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM _sqlx_migrations WHERE success = 0)")
            .fetch_one(pool)
            .await
            .map_err(map_sqlx_error)?;
    if failed != 0 {
        return Err(DatabaseError::Migration);
    }
    sqlx::query_scalar("SELECT version FROM _sqlx_migrations WHERE success = 1")
        .fetch_all(pool)
        .await
        .map_err(map_sqlx_error)
}

fn map_migration_error(error: MigrateError) -> DatabaseError {
    match error {
        MigrateError::Execute(error) | MigrateError::ExecuteMigration(error, _) => {
            match map_sqlx_error(error) {
                DatabaseError::Unavailable => DatabaseError::Migration,
                error => error,
            }
        }
        _ => DatabaseError::Migration,
    }
}

fn map_sqlx_error(error: sqlx::Error) -> DatabaseError {
    match error {
        sqlx::Error::PoolTimedOut => DatabaseError::PoolTimeout,
        sqlx::Error::Database(error)
            if matches!(
                error.code().as_deref(),
                Some("5" | "6" | "SQLITE_BUSY" | "SQLITE_LOCKED")
            ) =>
        {
            DatabaseError::Busy
        }
        _ => DatabaseError::Unavailable,
    }
}

async fn ensure_schema_not_newer_than_binary(pool: &SqlitePool) -> Result<(), DatabaseError> {
    let applied = applied_versions(pool).await?;
    let latest_binary = MIGRATOR
        .iter()
        .map(|migration| migration.version)
        .max()
        .unwrap_or(0);
    if applied.into_iter().any(|version| version > latest_binary) {
        return Err(DatabaseError::SchemaIncompatible);
    }
    Ok(())
}

async fn ensure_schema_is_current(pool: &SqlitePool) -> Result<(), DatabaseError> {
    let applied = applied_versions(pool).await?;
    if MIGRATOR
        .iter()
        .any(|migration| !applied.contains(&migration.version))
    {
        return Err(DatabaseError::SchemaPending);
    }
    Ok(())
}
