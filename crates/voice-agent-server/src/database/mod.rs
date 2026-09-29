use crate::config::DatabaseConfig;
use sqlx::{
    SqlitePool,
    migrate::{MigrateError, Migrator},
    sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous},
};
use std::{str::FromStr, time::Duration};
use thiserror::Error;

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
