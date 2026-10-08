use crate::config::DatabaseConfig;
use sqlx::{
    Connection, SqliteConnection, SqlitePool,
    migrate::{MigrateError, Migrator},
    sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous},
};
use std::{str::FromStr, time::Duration};
use thiserror::Error;

pub mod admission;
pub mod device_enrollments;
pub mod external_mcp;
pub mod external_mcp_policy;
pub mod history;
pub mod load_plan;
pub mod provider_config;
pub mod secrets;
pub mod speaker_candidate_set;
pub mod tool_allowlist;
pub mod tool_security;

pub use history::{
    HistoryArchive, HistoryDrop, HistoryRole, HistoryWrite, HistoryWriter, HistoryWriterCounters,
    HistoryWriterMetrics, RetentionCleaner, TranscriptCapture,
};

pub use admission::{
    AdmittedAgent, AdmittedAssignment, AdmittedProviderBinding, DeviceAdmissionError,
    DeviceAdmissionGraph,
};
pub use device_enrollments::{
    DeviceRegistration, EnrollmentClaim, EnrollmentClaimError, EnrollmentCleaner,
    EnrollmentCreateError, EnrollmentPending, EnrollmentPurge, EnrollmentRequest, EnrollmentStatus,
};
pub use external_mcp::{AdmittedMcpServer, McpAdmissionError};
pub use load_plan::{ProviderLoadPlan, ProviderLoadRequirement};

static MIGRATOR: Migrator = sqlx::migrate!("./migrations");

#[derive(Clone, Debug)]
pub struct Database {
    pool: SqlitePool,
    pub tool_security: std::sync::Arc<tool_security::ToolSecurity>,
}

/// Desired provider copied out of SQLite before runtime construction.  It contains no resolved
/// credential and is deliberately independent from the read-only runtime catalog.
#[derive(Clone, PartialEq, Eq)]
pub struct DesiredProvider {
    pub id: i64,
    pub key: String,
    pub kind: String,
    pub adapter: String,
    pub config_json: String,
    pub secret_ref: Option<String>,
    pub revision: i64,
}

impl std::fmt::Debug for DesiredProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DesiredProvider")
            .field("id", &self.id)
            .field("revision", &self.revision)
            .field("kind", &self.kind)
            .field("adapter", &self.adapter)
            .finish_non_exhaustive()
    }
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

pub(crate) fn unix_seconds() -> Option<i64> {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .map(|elapsed| elapsed.as_secs() as i64)
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
        if let Some(parent) = options.get_filename().parent()
            && !parent.as_os_str().is_empty()
        {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|_| DatabaseError::Unavailable)?;
        }
        let migration_options = options.clone().foreign_keys(false);
        let pool = SqlitePoolOptions::new()
            .max_connections(config.max_connections)
            .connect_with(options)
            .await
            .map_err(map_sqlx_error)?;

        ensure_schema_not_newer_than_binary(&pool).await?;
        if config.migrate_on_start {
            // SQLite cannot toggle foreign_keys inside SQLx's migration transaction. A scoped
            // startup-only connection permits atomic table rebuilds; the application pool keeps
            // foreign_keys enabled, and rebuild migrations check all references before commit.
            let mut connection = SqliteConnection::connect_with(&migration_options)
                .await
                .map_err(map_sqlx_error)?;
            MIGRATOR
                .run(&mut connection)
                .await
                .map_err(map_migration_error)?;
            connection.close().await.map_err(map_sqlx_error)?;
        } else {
            ensure_schema_is_current(&pool).await?;
        }
        Ok(Self {
            pool,
            tool_security: Default::default(),
        })
    }

    pub fn pool(&self) -> &SqlitePool {
        &self.pool
    }

    /// Whether the database still answers at all.
    ///
    /// This is the only database question readiness asks, and it is deliberately the cheapest one:
    /// one statement over one acquired connection, with no application row read, no schema walk,
    /// no Device lookup and no write.  `SELECT 1` is what proves the pool can still hand out a
    /// connection and the file is still there — a process whose schema is newer than its binary
    /// never reaches this point, because startup rejected it before the listener was bound.
    pub async fn is_reachable(&self) -> Result<(), DatabaseError> {
        sqlx::query("SELECT 1")
            .execute(&self.pool)
            .await
            .map(|_| ())
            .map_err(map_sqlx_error)
    }

    /// Reads desired state only.  Caller chooses whether an outcome is required, optional, or
    /// unbound; this database seam never constructs a provider or resolves a secret.
    pub async fn enabled_provider_rows(&self) -> Result<Vec<DesiredProvider>, DatabaseError> {
        let rows = sqlx::query_as::<_, (i64, String, String, String, String, Option<String>, i64)>(
            "SELECT id,key,type,adapter,config_json,secret_ref,revision FROM providers WHERE enabled=1 ORDER BY id",
        ).fetch_all(&self.pool).await.map_err(map_sqlx_error)?;
        Ok(rows
            .into_iter()
            .map(
                |(id, key, kind, adapter, config_json, secret_ref, revision)| DesiredProvider {
                    id,
                    key,
                    kind,
                    adapter,
                    config_json,
                    secret_ref,
                    revision,
                },
            )
            .collect())
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
