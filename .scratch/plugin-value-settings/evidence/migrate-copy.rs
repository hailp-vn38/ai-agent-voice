use voice_agent_server::{config::DatabaseConfig, database::Database};

#[tokio::main]
async fn main() {
    let path = std::env::var("ACCEPTANCE_DB_COPY").expect("backup path required");
    assert!(std::path::Path::new(&path).is_file());
    let db = Database::connect(&DatabaseConfig {
        url: format!("sqlite://{path}"),
        max_connections: 2,
        busy_timeout_ms: 5000,
        migrate_on_start: true,
        devices: Default::default(),
        history: Default::default(),
    })
    .await
    .expect("startup migrator must upgrade the populated backup");
    let violations: i64 = sqlx::query_scalar("SELECT count(*) FROM pragma_foreign_key_check")
        .fetch_one(db.pool())
        .await
        .unwrap();
    assert_eq!(violations, 0);
    let integrity: String = sqlx::query_scalar("PRAGMA integrity_check")
        .fetch_one(db.pool())
        .await
        .unwrap();
    assert_eq!(integrity, "ok");
    db.pool().close().await;
    println!("Startup migration, foreign keys and integrity: PASS");
}
