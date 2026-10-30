use super::*;
use crate::{
    config::DatabaseConfig,
    database::{
        credentials::CredentialCipher,
        secrets::{SecretResolver, SecretValue},
        tool_security::SpeakerDeps,
    },
};
use sqlx::Row;

async fn open_temp_db(label: &str) -> (Database, std::path::PathBuf) {
    let path = std::env::temp_dir().join(format!(
        "voice-agent-backup-test-{label}-{}.db",
        uuid::Uuid::new_v4()
    ));
    let cfg = DatabaseConfig {
        url: format!("sqlite://{}", path.display()),
        max_connections: 4,
        busy_timeout_ms: 5_000,
        migrate_on_start: true,
        devices: Default::default(),
        history: Default::default(),
    };
    let db = Database::connect(&cfg).await.expect("connect test db");
    (db, path)
}

fn cleanup_db_file(path: &std::path::Path) {
    let _ = std::fs::remove_file(path);
    let _ = std::fs::remove_file(path.with_extension("db-wal"));
    let _ = std::fs::remove_file(path.with_extension("db-shm"));
}

#[tokio::test]
async fn json_and_sqlite_roundtrip_preserve_revisions_web_assistant_and_constraints() {
    let (src_db, src_path) = open_temp_db("src").await;
    let (dst_db, dst_path) = open_temp_db("dst").await;
    let cipher = CredentialCipher::new(1, &[0x11; 32]).expect("cipher");

    let prov_cred = cipher
        .seal(
            &SecretValue::new("sk-provider-secret-1234".to_owned()),
            "provider:llm-main",
        )
        .expect("seal provider");
    let mcp_cred = cipher
        .seal(
            &SecretValue::new("Bearer-mcp-token-5678".to_owned()),
            "mcp:search-mcp",
        )
        .expect("seal mcp");
    let gw_cred = cipher
        .seal(
            &SecretValue::new("oauth-refresh-token-9999".to_owned()),
            "gateway_account:gemini-acc",
        )
        .expect("seal gateway");
    let notif_cred = cipher
        .seal(
            &SecretValue::new("tg-bot-token-abcd".to_owned()),
            "notification_channel:telegram",
        )
        .expect("seal notification");

    // Seed source DB across all 11 groups
    sqlx::query(
        "INSERT INTO providers (key, name, type, adapter, config_json, credential_json, enabled, revision, created_at, updated_at)
         VALUES ('llm-main', 'Main LLM', 'llm', 'openai_compatible', '{\"model\":\"gpt-4o\"}', ?, 1, 3, 100, 100)",
    )
    .bind(&prov_cred)
    .execute(src_db.pool())
    .await
    .unwrap();

    sqlx::query(
        "INSERT INTO agent_templates (key, name, description, language, prompt, enabled, revision, created_at, updated_at)
         VALUES ('tpl-default', 'Default Template', 'desc', 'vi', 'Xin chào', 1, 2, 100, 100)",
    )
    .execute(src_db.pool())
    .await
    .unwrap();

    sqlx::query(
        "INSERT INTO template_provider_bindings (template_id, provider_type, provider_source, provider_id, created_at, updated_at)
         SELECT t.id, 'llm', 'database', p.id, 100, 100
         FROM agent_templates t, providers p WHERE t.key = 'tpl-default' AND p.key = 'llm-main'",
    )
    .execute(src_db.pool())
    .await
    .unwrap();

    sqlx::query(
        "INSERT INTO agents (key, name, description, enabled, reminders_enabled, revision, reminder_policy_revision, created_at, updated_at)
         VALUES ('agent-home', 'Home Agent', 'desc', 1, 1, 4, 2, 100, 100)",
    )
    .execute(src_db.pool())
    .await
    .unwrap();

    sqlx::query(
        "INSERT INTO agent_template_assignments (agent_id, template_id, is_default, enabled, created_at)
         SELECT a.id, t.id, 1, 1, 100 FROM agents a, agent_templates t
         WHERE a.key = 'agent-home' AND t.key = 'tpl-default'",
    )
    .execute(src_db.pool())
    .await
    .unwrap();

    sqlx::query(
        "INSERT INTO mcp_servers (key, name, transport, url, headers_json, auth_type, connect_timeout_ms, request_timeout_ms, enabled, credential_json, revision, created_at, updated_at)
         VALUES ('search-mcp', 'Search MCP', 'streamable_http', 'https://example.com/mcp', '{}', 'bearer', 5000, 30000, 1, ?, 2, 100, 100)",
    )
    .bind(&mcp_cred)
    .execute(src_db.pool())
    .await
    .unwrap();

    // Seed both physical device and singleton web_assistant on source
    sqlx::query(
        "INSERT INTO devices (device_id, kind, agent_id, template_id, name, enabled, revision, created_at, updated_at)
         SELECT 'esp32-living', 'physical', a.id, t.id, 'Living Room', 1, 1, 100, 100
         FROM agents a, agent_templates t WHERE a.key = 'agent-home' AND t.key = 'tpl-default'",
    )
    .execute(src_db.pool())
    .await
    .unwrap();

    sqlx::query(
        "INSERT INTO devices (device_id, kind, agent_id, template_id, name, enabled, revision, created_at, updated_at)
         SELECT 'web_assistant_source_uuid', 'web_assistant', a.id, t.id, 'Web Assistant', 1, 1, 100, 100
         FROM agents a, agent_templates t WHERE a.key = 'agent-home' AND t.key = 'tpl-default'",
    )
    .execute(src_db.pool())
    .await
    .unwrap();

    sqlx::query(
        "INSERT INTO speakers (key, name, description, enabled, revision, created_at, updated_at)
         VALUES ('spk-owner', 'Owner', 'Primary user', 1, 1, 100, 100)",
    )
    .execute(src_db.pool())
    .await
    .unwrap();

    sqlx::query(
        "INSERT INTO speaker_voiceprints (speaker_id, embedding_space, provider_key, provider_revision, dims, vector, revision, sample_count, browser_validation_status, enrolled_at, updated_at, calibration_revision)
         SELECT id, 'campp-512', 'builtin', 1, 4, X'01020304', 1, 3, 'passed', 100, 100, 'cal-v1'
         FROM speakers WHERE key = 'spk-owner'",
    )
    .execute(src_db.pool())
    .await
    .unwrap();

    sqlx::query(
        "INSERT INTO llm_gateway_accounts (account_key, vendor, label, account_email, credential_json, enabled, priority, status, created_at, updated_at)
         VALUES ('gemini-acc', 'gemini', 'Gemini Pool', 'user@example.com', ?, 1, 100, 'active', 100, 100)",
    )
    .bind(&gw_cred)
    .execute(src_db.pool())
    .await
    .unwrap();

    sqlx::query(
        "INSERT OR REPLACE INTO notification_channel_configs (channel_kind, enabled, target_json, subscribed_topics_json, encrypted_credential, updated_at_ms)
         VALUES ('telegram', 1, '{\"chat_id\":\"123\"}', '[\"reminder_due\"]', ?, 1000)",
    )
    .bind(&notif_cred)
    .execute(src_db.pool())
    .await
    .unwrap();

    // Seed reminder and history on web_assistant device in source DB
    sqlx::query(
        "INSERT INTO device_reminders (id, device_row_id, message, due_at_ms, timezone, status, row_revision, schedule_revision, source, attempts_used, created_at_ms, updated_at_ms)
         SELECT 'rem-1', id, 'Uống nước', 200000, 'UTC', 'pending', 2, 7, 'web_assistant', 0, 1000, 1000
         FROM devices WHERE kind = 'web_assistant'",
    )
    .execute(src_db.pool())
    .await
    .unwrap();

    sqlx::query(
        "INSERT INTO history_messages (session_id, sequence, device_id, agent_id, template_id, turn_id, role, text, created_at)
         SELECT 'sess-web-1', 1, d.id, a.id, t.id, 'turn-1', 'user', 'Xin chào web', 100
         FROM devices d, agents a, agent_templates t
         WHERE d.kind = 'web_assistant' AND a.key = 'agent-home' AND t.key = 'tpl-default'",
    )
    .execute(src_db.pool())
    .await
    .unwrap();

    sqlx::query(
        "INSERT INTO notifications_inbox (id, dedup_key, topic, severity, title, body, metadata_json, is_read, created_at_ms)
         VALUES ('notif-src-1', 'dedup-shared-key', 'task_completed', 'success', 'Done', 'Task finished', '{}', 0, 1000)",
    )
    .execute(src_db.pool())
    .await
    .unwrap();

    // Seed destination DB with:
    // - pre-existing provider 'llm-main' at revision 10 (must become 11 after import!)
    // - pre-existing web_assistant with a DIFFERENT random device_id ('web_assistant_dest_uuid')
    // - pre-existing obsolete speaker 'spk-old' referenced by a committed speaker_quick_captures row
    // - pre-existing notifications_inbox row with DIFFERENT id ('notif-dst-99') but SAME dedup_key ('dedup-shared-key')
    sqlx::query(
        "INSERT INTO providers (key, name, type, adapter, config_json, enabled, revision, created_at, updated_at)
         VALUES ('llm-main', 'Old LLM', 'llm', 'openai_compatible', '{}', 1, 10, 50, 50)",
    )
    .execute(dst_db.pool())
    .await
    .unwrap();

    sqlx::query(
        "INSERT INTO agents (key, name, enabled, revision, reminder_policy_revision, created_at, updated_at)
         VALUES ('agent-home', 'Old Home', 1, 5, 3, 50, 50)",
    )
    .execute(dst_db.pool())
    .await
    .unwrap();

    sqlx::query(
        "INSERT INTO devices (device_id, kind, agent_id, name, enabled, revision, created_at, updated_at)
         SELECT 'web_assistant_dest_uuid', 'web_assistant', id, 'Dest Web Assistant', 1, 4, 50, 50
         FROM agents WHERE key = 'agent-home'",
    )
    .execute(dst_db.pool())
    .await
    .unwrap();

    sqlx::query(
        "INSERT INTO speakers (key, name, enabled, revision, created_at, updated_at)
         VALUES ('spk-old', 'Obsolete Speaker', 1, 1, 50, 50)",
    )
    .execute(dst_db.pool())
    .await
    .unwrap();

    sqlx::query(
        "INSERT INTO speaker_quick_captures (id, provider_revision, runtime_id, embedding_space, dims, vector, status, speaker_id, created_at, expires_at, committed_at)
         SELECT 'cap-committed-1', 1, 'rt-1', 'campp-512', 4, NULL, 'committed', id, 50, 100, 60
         FROM speakers WHERE key = 'spk-old'",
    )
    .execute(dst_db.pool())
    .await
    .unwrap();

    sqlx::query(
        "INSERT INTO notifications_inbox (id, dedup_key, topic, severity, title, body, metadata_json, is_read, created_at_ms)
         VALUES ('notif-dst-99', 'dedup-shared-key', 'task_completed', 'info', 'Old Title', 'Old Body', '{}', 0, 500)",
    )
    .execute(dst_db.pool())
    .await
    .unwrap();

    // Register a ToolSecurity token on dst_db to verify automatic invalidation on import
    let sys_mcp_token = dst_db.tool_security.register_system_mcp(1);
    let spk_token = dst_db.tool_security.register_speaker(SpeakerDeps {
        agent: 1,
        template: 1,
        speakers: vec![1],
        candidate_set_digest: None,
    });

    // 1. Export JSON bundle from src_db
    let exported_json = src_db
        .export_backup(ExportBackupRequest {
            format: BackupFormat::Json,
            scope: BackupScope::Full,
            companion_settings: BackupCompanionSettings {
                background_settings: Some(serde_json::json!({"enabled": true})),
                knowledge_mcp_settings: Some(serde_json::json!({"enabled": true})),
            },
        })
        .await
        .expect("export json");

    // 2. Inspect against dst_db in Replace mode with same cipher
    let inspection = dst_db
        .inspect_backup(InspectBackupRequest {
            payload: &exported_json.bytes,
            options: BackupImportOptions {
                mode: ImportMode::Replace,
                scope: BackupScope::Full,
                clear_unreadable_credentials: true,
            },
            secret_resolver: &cipher,
        })
        .await
        .expect("inspect json");
    assert_eq!(inspection.detected_format, BackupFormat::Json);
    assert_eq!(inspection.credentials.total_encrypted, 4);
    assert_eq!(inspection.credentials.decryptable, 4);
    assert_eq!(inspection.credentials.unreadable, 0);

    // 3. Import JSON bundle into dst_db in Replace mode
    let outcome = dst_db
        .import_backup(ImportBackupRequest {
            request_id: "req-backup-import-1",
            payload: &exported_json.bytes,
            options: BackupImportOptions {
                mode: ImportMode::Replace,
                scope: BackupScope::Full,
                clear_unreadable_credentials: true,
            },
            secret_resolver: &cipher,
        })
        .await
        .expect("import json");

    assert!(outcome.report.applied_background_settings);
    assert!(outcome.report.applied_knowledge_mcp_settings);
    assert!(sys_mcp_token.is_cancelled(), "system_mcp token must be invalidated by Database::import_backup");
    assert!(spk_token.is_cancelled(), "speaker token must be invalidated by Database::import_backup");

    // Verify monotonic revision bump: provider 'llm-main' was at rev 10, must now be 11
    let prov_rev: i64 =
        sqlx::query_scalar("SELECT revision FROM providers WHERE key = 'llm-main'")
            .fetch_one(dst_db.pool())
            .await
            .unwrap();
    assert_eq!(prov_rev, 11);

    // Verify singleton web_assistant preserved destination device_id ('web_assistant_dest_uuid')
    // and linked both the imported reminder and history message
    let web_dev_id: String =
        sqlx::query_scalar("SELECT device_id FROM devices WHERE kind = 'web_assistant'")
            .fetch_one(dst_db.pool())
            .await
            .unwrap();
    assert_eq!(web_dev_id, "web_assistant_dest_uuid");

    let rem_sync_rev: i64 = sqlx::query_scalar(
        "SELECT s.schedule_revision FROM device_reminder_sync s
         JOIN devices d ON d.id = s.device_row_id WHERE d.kind = 'web_assistant'",
    )
    .fetch_one(dst_db.pool())
    .await
    .unwrap();
    assert!(rem_sync_rev >= 7, "device_reminder_sync.schedule_revision must be >= imported reminder's schedule_revision (7)");

    let hist_text: String = sqlx::query_scalar(
        "SELECT h.text FROM history_messages h
         JOIN devices d ON d.id = h.device_id WHERE d.kind = 'web_assistant' AND h.session_id = 'sess-web-1'",
    )
    .fetch_one(dst_db.pool())
    .await
    .unwrap();
    assert_eq!(hist_text, "Xin chào web");

    // Verify obsolete speaker 'spk-old' and its committed quick capture were cleanly deleted
    let spk_old_exists: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM speakers WHERE key = 'spk-old'")
            .fetch_one(dst_db.pool())
            .await
            .unwrap();
    assert_eq!(spk_old_exists, 0);

    // Verify notifications_inbox matched by dedup_key without UNIQUE constraint error
    let notif_title: String = sqlx::query_scalar(
        "SELECT title FROM notifications_inbox WHERE dedup_key = 'dedup-shared-key'",
    )
    .fetch_one(dst_db.pool())
    .await
    .unwrap();
    assert_eq!(notif_title, "Done");

    // Verify atomic audit event was recorded inside the transaction
    let audit_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM admin_audit_events WHERE request_id = 'req-backup-import-1' AND resource_type = 'database_backup' AND action = 'import'",
    )
    .fetch_one(dst_db.pool())
    .await
    .unwrap();
    assert_eq!(audit_count, 1);

    // 4. Test SQLite (.db) snapshot export (ConfigOnly) & import with a DIFFERENT key
    let exported_sqlite = src_db
        .export_backup(ExportBackupRequest {
            format: BackupFormat::Sqlite,
            scope: BackupScope::ConfigOnly,
            companion_settings: BackupCompanionSettings::default(),
        })
        .await
        .expect("export sqlite config_only");
    assert!(exported_sqlite.bytes.starts_with(SQLITE_MAGIC_HEADER));

    let other_cipher = CredentialCipher::new(1, &[0x99; 32]).expect("other cipher");
    let sqlite_inspection = dst_db
        .inspect_backup(InspectBackupRequest {
            payload: &exported_sqlite.bytes,
            options: BackupImportOptions {
                mode: ImportMode::Replace,
                scope: BackupScope::Full,
                clear_unreadable_credentials: true,
            },
            secret_resolver: &other_cipher,
        })
        .await
        .expect("inspect sqlite config_only snapshot with full requested scope");
    assert_eq!(sqlite_inspection.bundle_scope, BackupScope::ConfigOnly);
    assert_eq!(sqlite_inspection.effective_scope, BackupScope::ConfigOnly);

    let err = dst_db
        .import_backup(ImportBackupRequest {
            request_id: "req-unreadable-reject",
            payload: &exported_sqlite.bytes,
            options: BackupImportOptions {
                mode: ImportMode::Merge,
                scope: BackupScope::ConfigOnly,
                clear_unreadable_credentials: false,
            },
            secret_resolver: &other_cipher,
        })
        .await
        .expect_err("must reject unreadable credentials when clear_unreadable_credentials = false");
    assert!(matches!(err, BackupError::UnreadableCredentials));

    let outcome_other_key = dst_db
        .import_backup(ImportBackupRequest {
            request_id: "req-unreadable-scrub",
            payload: &exported_sqlite.bytes,
            options: BackupImportOptions {
                mode: ImportMode::Replace,
                scope: BackupScope::Full,
                clear_unreadable_credentials: true,
            },
            secret_resolver: &other_cipher,
        })
        .await
        .expect("import sqlite with credential scrub");
    assert_eq!(outcome_other_key.report.detected_format, BackupFormat::Sqlite);
    assert_eq!(outcome_other_key.effective_scope, BackupScope::ConfigOnly);
    assert_eq!(outcome_other_key.report.credentials.unreadable, 4);

    // Because effective_scope resolved to ConfigOnly, live history_messages must remain intact even in Replace mode
    let hist_remaining: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM history_messages")
        .fetch_one(dst_db.pool())
        .await
        .unwrap();
    assert_eq!(hist_remaining, 1);

    // Verify nullable credentials were cleared to NULL and gateway account was set to "{}" + "reauth_required"
    let mcp_cred_after: Option<String> =
        sqlx::query_scalar("SELECT credential_json FROM mcp_servers WHERE key = 'search-mcp'")
            .fetch_one(dst_db.pool())
            .await
            .unwrap();
    assert_eq!(mcp_cred_after, None);

    let gw_row = sqlx::query(
        "SELECT credential_json, status FROM llm_gateway_accounts WHERE account_key = 'gemini-acc'",
    )
    .fetch_one(dst_db.pool())
    .await
    .unwrap();
    let gw_cred_after: String = gw_row.get("credential_json");
    let gw_status_after: String = gw_row.get("status");
    assert_eq!(gw_cred_after, "{}");
    assert_eq!(gw_status_after, "reauth_required");

    cleanup_db_file(&src_path);
    cleanup_db_file(&dst_path);
}
