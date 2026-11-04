//! `devices.memory_mode`: migration of existing rows, read model and `memory_binding`.
use super::*;
use crate::config::DatabaseConfig;
use sqlx::{Connection, Executor, SqliteConnection, sqlite::SqliteConnectOptions};
use std::str::FromStr;

const MEMORY_MODE_MIGRATION: i64 = 141;

fn temp_db_path(label: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!(
        "vela-memory-mode-{label}-{}.db",
        uuid::Uuid::new_v4()
    ))
}

fn cleanup(path: &std::path::Path) {
    let _ = std::fs::remove_file(path);
    let _ = std::fs::remove_file(path.with_extension("db-wal"));
    let _ = std::fs::remove_file(path.with_extension("db-shm"));
}

async fn open_db(label: &str) -> (Database, std::path::PathBuf) {
    let path = temp_db_path(label);
    let database = Database::connect(&DatabaseConfig {
        url: format!("sqlite://{}", path.display()),
        max_connections: 4,
        busy_timeout_ms: 5_000,
        migrate_on_start: true,
        devices: Default::default(),
        history: Default::default(),
    })
    .await
    .expect("connect test db");
    (database, path)
}

async fn seed_device(database: &Database, device_id: &str) -> i64 {
    sqlx::query(
        "INSERT OR IGNORE INTO agents (key, name, enabled, created_at, updated_at)
         VALUES ('home', 'Home', 1, 1, 1)",
    )
    .execute(database.pool())
    .await
    .unwrap();
    database
        .create_device(
            DeviceInput {
                device_id,
                agent_key: "home",
                template_key: None,
                name: None,
                description: None,
                metadata: None,
                memory_mode: MemoryMode::ReadWrite,
            },
            "req-seed",
        )
        .await
        .map_err(|_| "create device failed")
        .unwrap();
    get_device_by(database, device_id).await.unwrap().id
}

#[tokio::test]
async fn memory_mode_migration_gives_existing_devices_off() {
    let path = temp_db_path("migration");
    let options = SqliteConnectOptions::from_str(&format!("sqlite://{}", path.display()))
        .unwrap()
        .create_if_missing(true);
    let mut connection = SqliteConnection::connect_with(&options).await.unwrap();

    let migrator = &crate::database::MIGRATOR;
    for migration in migrator
        .iter()
        .filter(|m| m.version < MEMORY_MODE_MIGRATION)
    {
        connection
            .execute(sqlx::raw_sql(&migration.sql))
            .await
            .unwrap_or_else(|e| panic!("migration {}: {e}", migration.version));
    }
    connection
        .execute(sqlx::raw_sql(
            "INSERT INTO agents (key, name, enabled, created_at, updated_at)
             VALUES ('home', 'Home', 1, 1, 1);
             INSERT INTO devices (device_id, agent_id, created_at, updated_at)
             SELECT 'existing', id, 1, 1 FROM agents WHERE key = 'home';",
        ))
        .await
        .unwrap();

    let migration = migrator
        .iter()
        .find(|m| m.version == MEMORY_MODE_MIGRATION)
        .expect("memory_mode migration is registered");
    connection
        .execute(sqlx::raw_sql(&migration.sql))
        .await
        .unwrap();

    let mode: String =
        sqlx::query_scalar("SELECT memory_mode FROM devices WHERE device_id = 'existing'")
            .fetch_one(&mut connection)
            .await
            .unwrap();
    assert_eq!(mode, "off");

    let rejected = sqlx::query("UPDATE devices SET memory_mode = 'admin'")
        .execute(&mut connection)
        .await;
    assert!(rejected.is_err(), "the column must reject unknown values");

    connection.close().await.unwrap();
    cleanup(&path);
}

#[tokio::test]
async fn memory_binding_returns_device_id_kind_and_mode() {
    let (database, path) = open_db("binding").await;
    let row_id = seed_device(&database, "kitchen").await;

    let binding = memory_binding(&database, row_id)
        .await
        .unwrap()
        .expect("device exists");
    assert_eq!(
        binding,
        MemoryBinding {
            device_id: "kitchen".to_owned(),
            kind: "physical".to_owned(),
            mode: MemoryMode::ReadWrite,
        }
    );
    assert_eq!(memory_binding(&database, row_id + 1).await.unwrap(), None);

    cleanup(&path);
}

#[tokio::test]
async fn memory_mode_is_kept_by_a_device_update_and_listed() {
    let (database, path) = open_db("read-model").await;
    let row_id = seed_device(&database, "kitchen").await;

    database
        .update_device(
            row_id,
            1,
            DeviceChanges {
                agent_key: "home",
                template_key: None,
                name: None,
                description: None,
                metadata: None,
                enabled: 1,
                memory_mode: MemoryMode::Read,
            },
            "req-update",
        )
        .await
        .map_err(|_| "update device failed")
        .unwrap();

    let listed = database.list_devices(None, None, 1, 10).await.unwrap();
    assert_eq!(listed[0].memory_mode, MemoryMode::Read);
    assert_eq!(listed[0].revision, 2);

    cleanup(&path);
}

#[tokio::test]
async fn assistant_device_keeps_memory_mode_unless_one_is_given() {
    let (database, path) = open_db("assistant").await;
    seed_device(&database, "kitchen").await;

    let created = database
        .save_assistant_device("home", None, true, None, None, "req-1")
        .await
        .map_err(|_| "create assistant failed")
        .unwrap();
    assert_eq!(created.memory_mode, MemoryMode::Off);

    let enabled = database
        .save_assistant_device("home", None, true, Some(MemoryMode::Read), Some(1), "req-2")
        .await
        .map_err(|_| "enable memory failed")
        .unwrap();
    assert_eq!(enabled.memory_mode, MemoryMode::Read);

    let kept = database
        .save_assistant_device("home", None, false, None, Some(2), "req-3")
        .await
        .map_err(|_| "update assistant failed")
        .unwrap();
    assert_eq!(kept.memory_mode, MemoryMode::Read);

    cleanup(&path);
}
