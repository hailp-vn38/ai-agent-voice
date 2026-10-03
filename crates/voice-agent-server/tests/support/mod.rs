//! File-backed admission fixtures for protocol tests. No database-free WS bypass.
#![allow(dead_code)]

use std::sync::Arc;
use voice_agent_server::{
    app::router_with_providers, config::AppConfig, database::Database, providers::ProviderSet,
};

pub async fn provision(mut config: AppConfig) -> (AppConfig, Database) {
    config.database.url = format!(
        "sqlite://{}",
        std::env::temp_dir()
            .join(format!("voice-agent-protocol-{}.db", uuid::Uuid::new_v4()))
            .display()
    );
    let database = Database::connect(&config.database).await.unwrap();
    sqlx::query("INSERT INTO agents (key,name,created_at,updated_at) VALUES ('protocol_fixture','Protocol fixture',1,1)")
        .execute(database.pool())
        .await
        .unwrap();
    for device_id in [
        "reference-client-01",
        "browser-client",
        "tracer-device",
        "text-turn-device",
        "phase4-gate",
        "phase5-gate",
        "phase6-reference",
        "exit-abort",
        "exit-gate",
    ] {
        sqlx::query("INSERT INTO devices (device_id,agent_id,created_at,updated_at) SELECT ?,id,1,1 FROM agents WHERE key='protocol_fixture'")
            .bind(device_id)
            .execute(database.pool())
            .await
            .unwrap();
    }
    (config, database)
}

pub async fn router(config: AppConfig, providers: Arc<ProviderSet>) -> axum::Router {
    let (config, database) = provision(config).await;
    drop(database);
    router_with_providers(config, providers).await.unwrap()
}
