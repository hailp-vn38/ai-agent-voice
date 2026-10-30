use super::*;
use crate::{
    config::{AppConfig, DatabaseConfig},
    database::{Database, backup::MAX_BACKUP_BODY_BYTES},
    providers::LoadedProviders,
};

#[tokio::test]
async fn admin_http_backup_export_inspect_and_import_roundtrip_companion_settings() {
    let dir = std::env::temp_dir().join(format!(
        "voice-agent-admin-backup-test-{}",
        uuid::Uuid::new_v4()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let db_path = dir.join("test.db");
    let db = Database::connect(&DatabaseConfig {
        url: format!("sqlite://{}", db_path.display()),
        max_connections: 2,
        busy_timeout_ms: 5_000,
        migrate_on_start: true,
        devices: Default::default(),
        history: Default::default(),
    })
    .await
    .unwrap();

    sqlx::query(
        "INSERT INTO agents (key, name, enabled, revision, reminder_policy_revision, created_at, updated_at)
         VALUES ('agent-http', 'HTTP Agent', 1, 1, 1, 100, 100)",
    )
    .execute(db.pool())
    .await
    .unwrap();

    let store_dir = dir.join("store");
    let prompts_dir = dir.join("prompts");
    std::fs::create_dir_all(&store_dir).unwrap();
    std::fs::write(
        store_dir.join("agent_settings.json"),
        r#"{"enabled":true,"orchestrator_llm_provider":"config:llm:default","worker_llm_provider":"config:llm:default"}"#,
    )
    .unwrap();
    std::fs::write(
        store_dir.join("knowledge_mcp_settings.json"),
        r#"{"revision":3,"enabled":true,"path":"/mcp/knowledge","allow_write":false,"max_concurrency":8,"call_timeout_ms":15000,"max_result_chars":4096,"tokens":[]}"#,
    )
    .unwrap();

    let mut cfg: AppConfig = toml::from_str(
        r#"
        [server]
        bind = "127.0.0.1:0"
        public_ws_url = "ws://127.0.0.1:0/ws"
        [provider_defaults]
        vad = "test"
        asr = "test"
        llm = "test"
        tts = "test"
        "#,
    )
    .unwrap();
    cfg.orchestrator.store_dir = store_dir.clone();
    cfg.orchestrator.prompts_dir = prompts_dir.clone();
    let state = AppState::new_with_database(cfg, LoadedProviders::default(), Some(db.clone()));

    // 1. Export JSON full backup via handler
    let req = Request::builder()
        .uri("/api/admin/system/backup/export?format=json&scope=full")
        .body(Body::empty())
        .unwrap();
    let resp = export_backup(State(state.clone()), req).await;
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(
        resp.headers()
            .get(header::CACHE_CONTROL)
            .and_then(|v| v.to_str().ok()),
        Some("no-store")
    );
    let disp = resp
        .headers()
        .get(header::CONTENT_DISPOSITION)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    assert!(disp.contains("voice-agent-backup-") && disp.ends_with(".json\""));
    let exported_bytes = to_bytes(resp.into_body(), MAX_BACKUP_BODY_BYTES)
        .await
        .unwrap();

    // Remove companion files on disk to verify import restores them
    let _ = std::fs::remove_file(store_dir.join("agent_settings.json"));
    let _ = std::fs::remove_file(store_dir.join("knowledge_mcp_settings.json"));

    // 2. Inspect via handler
    let inspect_req = Request::builder()
        .method("POST")
        .uri("/api/admin/system/backup/inspect?mode=merge&scope=full&clear_unreadable_credentials=true")
        .body(Body::from(exported_bytes.clone()))
        .unwrap();
    let inspect_resp = inspect_backup(State(state.clone()), inspect_req).await;
    assert_eq!(inspect_resp.status(), StatusCode::OK);
    let inspect_body = to_bytes(inspect_resp.into_body(), MAX_BACKUP_BODY_BYTES)
        .await
        .unwrap();
    let report: crate::database::backup::BackupInspectionReport =
        serde_json::from_slice(&inspect_body).unwrap();
    assert!(report.includes_background_settings);
    assert!(report.includes_knowledge_mcp_settings);

    // 3. Import via handler
    let import_req = Request::builder()
        .method("POST")
        .uri("/api/admin/system/backup/import?mode=merge&scope=full&clear_unreadable_credentials=true")
        .body(Body::from(exported_bytes))
        .unwrap();
    let import_resp = import_backup(State(state.clone()), import_req).await;
    assert_eq!(import_resp.status(), StatusCode::OK);
    let import_body = to_bytes(import_resp.into_body(), MAX_BACKUP_BODY_BYTES)
        .await
        .unwrap();
    let import_report: crate::database::backup::BackupImportReport =
        serde_json::from_slice(&import_body).unwrap();
    assert!(import_report.applied_background_settings);
    assert!(import_report.applied_knowledge_mcp_settings);
    assert!(store_dir.join("agent_settings.json").is_file());
    assert!(store_dir.join("knowledge_mcp_settings.json").is_file());
    assert!(prompts_dir.is_dir());

    let _ = std::fs::remove_dir_all(&dir);
}
