use std::{fs, str::FromStr, sync::Arc};

use sha2::{Digest, Sha384};
use sqlx::{
    SqlitePool,
    sqlite::{SqliteConnectOptions, SqlitePoolOptions},
};
use tokio::{net::TcpListener, task::JoinHandle};
use voice_agent_server::{
    app::bootstrap_with_providers,
    config::{AppConfig, DatabaseConfig},
    database::{Database, DatabaseError},
    providers::ProviderSet,
};

fn temp_database_url(label: &str) -> String {
    let path = std::env::temp_dir().join(format!(
        "voice-agent-{label}-{}-{}.db",
        std::process::id(),
        uuid::Uuid::new_v4()
    ));
    format!("sqlite://{}", path.display())
}

fn database_config(url: String) -> DatabaseConfig {
    DatabaseConfig {
        enabled: true,
        url,
        max_connections: 2,
        busy_timeout_ms: 5_000,
        migrate_on_start: true,
        devices: Default::default(),
    }
}

fn config_for_database(database: DatabaseConfig) -> AppConfig {
    let path = std::env::temp_dir().join(format!(
        "voice-agent-bootstrap-{}-{}.toml",
        std::process::id(),
        uuid::Uuid::new_v4()
    ));
    fs::write(
        &path,
        format!(
            r#"
[server]
bind = "127.0.0.1:0"
public_ws_url = "ws://127.0.0.1:0/voice/v1/"

[provider_defaults]
vad = "test"
asr = "test"
llm = "test"
tts = "test"

[database]
enabled = {}
url = "{}"
max_connections = {}
busy_timeout_ms = {}
migrate_on_start = {}
"#,
            database.enabled,
            database.url,
            database.max_connections,
            database.busy_timeout_ms,
            database.migrate_on_start,
        ),
    )
    .unwrap();
    let config = AppConfig::parse_and_resolve(&path).unwrap();
    fs::remove_file(path).unwrap();
    config
}

async fn serve(router: axum::Router) -> (String, JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let task = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    (format!("http://{address}"), task)
}

#[tokio::test]
async fn fresh_database_migrates_then_boots_health_and_ready_routes() {
    let database = database_config(temp_database_url("fresh"));
    let router = bootstrap_with_providers(
        config_for_database(database.clone()),
        Arc::new(ProviderSet::unavailable()),
    )
    .await
    .unwrap();

    let (base, task) = serve(router).await;
    assert_eq!(
        reqwest::get(format!("{base}/health"))
            .await
            .unwrap()
            .status(),
        200
    );
    assert_eq!(
        reqwest::get(format!("{base}/ready"))
            .await
            .unwrap()
            .status(),
        200
    );
    task.abort();

    let database = Database::connect(&database).await.unwrap();
    let foreign_keys: i64 = sqlx::query_scalar("PRAGMA foreign_keys")
        .fetch_one(database.pool())
        .await
        .unwrap();
    let journal_mode: String = sqlx::query_scalar("PRAGMA journal_mode")
        .fetch_one(database.pool())
        .await
        .unwrap();
    assert_eq!(foreign_keys, 1);
    assert_eq!(journal_mode, "wal");
    let table_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'agents'",
    )
    .fetch_one(database.pool())
    .await
    .unwrap();
    assert_eq!(table_count, 1);
    let agent_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM agents")
        .fetch_one(database.pool())
        .await
        .unwrap();
    assert_eq!(
        agent_count, 0,
        "migrations never provision application rows"
    );
}

#[tokio::test]
async fn current_database_is_idempotent_when_migrations_run_again() {
    let config = database_config(temp_database_url("idempotent"));
    Database::connect(&config).await.unwrap();
    Database::connect(&config).await.unwrap();
}

#[tokio::test]
async fn supported_old_schema_is_forward_migrated_to_the_current_indexes() {
    let config = database_config(temp_database_url("forward"));
    let options = SqliteConnectOptions::from_str(&config.url)
        .unwrap()
        .create_if_missing(true);
    let pool: SqlitePool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(options)
        .await
        .unwrap();
    let migration = include_str!("../migrations/0001_initial_database.sql");
    sqlx::raw_sql(migration).execute(&pool).await.unwrap();
    sqlx::query(
        "CREATE TABLE _sqlx_migrations (version BIGINT PRIMARY KEY, description TEXT NOT NULL, \
         installed_on TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP, success BOOLEAN NOT NULL, \
         checksum BLOB NOT NULL, execution_time BIGINT NOT NULL)",
    )
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO _sqlx_migrations (version, description, success, checksum, execution_time) \
         VALUES (1, 'initial database', 1, ?, 0)",
    )
    .bind(Sha384::digest(migration.as_bytes()).to_vec())
    .execute(&pool)
    .await
    .unwrap();
    drop(pool);

    let database = Database::connect(&config).await.unwrap();
    let index_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM sqlite_master WHERE type = 'index' AND name = 'idx_devices_agent_id'",
    )
    .fetch_one(database.pool())
    .await
    .unwrap();
    assert_eq!(index_count, 1);
}

#[tokio::test]
async fn pending_schema_with_migration_on_start_disabled_fails_before_boot() {
    let mut config = database_config(temp_database_url("pending"));
    config.migrate_on_start = false;
    let error = bootstrap_with_providers(
        config_for_database(config),
        Arc::new(ProviderSet::unavailable()),
    )
    .await
    .unwrap_err();
    assert!(matches!(
        error,
        voice_agent_server::app::BootstrapError::Database(DatabaseError::SchemaPending)
    ));
}

#[tokio::test]
async fn failed_migration_history_fails_before_boot() {
    let config = database_config(temp_database_url("failed-history"));
    let database = Database::connect(&config).await.unwrap();
    sqlx::query("UPDATE _sqlx_migrations SET success = 0 WHERE version = 2")
        .execute(database.pool())
        .await
        .unwrap();
    drop(database);

    let error = bootstrap_with_providers(
        config_for_database(config),
        Arc::new(ProviderSet::unavailable()),
    )
    .await
    .unwrap_err();
    assert!(matches!(
        error,
        voice_agent_server::app::BootstrapError::Database(DatabaseError::Migration)
    ));
}

#[tokio::test]
async fn schema_newer_than_binary_is_rejected_before_listener_bind() {
    let config = database_config(temp_database_url("newer"));
    let database = Database::connect(&config).await.unwrap();
    sqlx::query(
        "INSERT INTO _sqlx_migrations (version, description, success, checksum, execution_time) \
         VALUES (999, 'future', 1, X'00', 0)",
    )
    .execute(database.pool())
    .await
    .unwrap();
    drop(database);

    let error = bootstrap_with_providers(
        config_for_database(config),
        Arc::new(ProviderSet::unavailable()),
    )
    .await
    .unwrap_err();
    assert!(matches!(
        error,
        voice_agent_server::app::BootstrapError::Database(DatabaseError::SchemaIncompatible)
    ));
}

#[tokio::test]
async fn disabled_database_preserves_legacy_boot() {
    let mut database = database_config(temp_database_url("disabled"));
    database.enabled = false;
    let router = bootstrap_with_providers(
        config_for_database(database),
        Arc::new(ProviderSet::unavailable()),
    )
    .await
    .unwrap();
    let (base, task) = serve(router).await;
    assert_eq!(
        reqwest::get(format!("{base}/health"))
            .await
            .unwrap()
            .status(),
        200
    );
    task.abort();
}
