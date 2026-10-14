//! Upgrade a populated V2/DAG database through the real startup migrator.
use sqlx::{Connection, SqliteConnection, migrate::Migrator, sqlite::SqliteConnectOptions};
use std::{borrow::Cow, str::FromStr};
use voice_agent_server::{config::DatabaseConfig, database::Database};

#[tokio::test]
async fn v3_migration_preserves_legacy_jobs_retries_runs_events_artifacts_and_constraints() {
    let url = format!(
        "sqlite://{}",
        std::env::temp_dir()
            .join(format!("v3-migration-{}.db", uuid::Uuid::new_v4()))
            .display()
    );
    let options = SqliteConnectOptions::from_str(&url)
        .unwrap()
        .create_if_missing(true)
        .foreign_keys(false);
    let mut conn = SqliteConnection::connect_with(&options).await.unwrap();
    let all = sqlx::migrate!("./migrations");
    let old = Migrator {
        migrations: Cow::Owned(all.iter().filter(|m| m.version < 114).cloned().collect()),
        ignore_missing: false,
        locking: true,
        no_tx: false,
    };
    old.run(&mut conn).await.unwrap();
    for (id, mode, retry) in [
        ("dag", "dag_v1", None),
        ("v2", "hierarchical_v2", Some("dag")),
    ] {
        sqlx::query("INSERT INTO jobs(id,requester_scope,request_json,request_fingerprint,snapshot_json,plugin_choices_json,settings_revision,budget_limit_json,execution_mode,status,deadline_at,retry_of_job_id,created_at,updated_at) VALUES(?,'admin:deployment','{\"goal\":\"legacy\"}','fingerprint','{}','[]',1,'{}',?,'failed',9999999999,?,1,2)")
            .bind(id).bind(mode).bind(retry).execute(&mut conn).await.unwrap();
        sqlx::query("INSERT INTO job_steps(job_id,step_key,plan_revision,status,created_at,updated_at) VALUES(?,'step',1,'failed',1,2)").bind(id).execute(&mut conn).await.unwrap();
        sqlx::query("INSERT INTO job_artifacts(id,job_id,step_key,relative_path,media_type,size_bytes,sha256,created_at) VALUES(?,?,'step',?,'text/plain',3,'digest',1)").bind(format!("artifact-{id}")).bind(id).bind(format!("{id}/artifact")).execute(&mut conn).await.unwrap();
        sqlx::query("INSERT INTO job_events(job_id,sequence,event_type,created_at,payload_json) VALUES(?,1,'job_finished',2,'{\"status\":\"failed\"}')").bind(id).execute(&mut conn).await.unwrap();
    }
    sqlx::query("INSERT INTO job_agent_runs(agent_run_id,job_id,role,goal_excerpt,allowed_operations_json,status,created_at) VALUES('old-root','v2','orchestrator','Legacy goal','[]','failed',1)").execute(&mut conn).await.unwrap();
    let before: Vec<(String, String, Option<String>, String)> = sqlx::query_as(
        "SELECT id,execution_mode,retry_of_job_id,request_json FROM jobs ORDER BY id",
    )
    .fetch_all(&mut conn)
    .await
    .unwrap();
    conn.close().await.unwrap();
    let db = Database::connect(&DatabaseConfig {
        url,
        max_connections: 2,
        busy_timeout_ms: 5000,
        migrate_on_start: true,
        devices: Default::default(),
        history: Default::default(),
    })
    .await
    .unwrap();
    let after: Vec<(String, String, Option<String>, String)> = sqlx::query_as(
        "SELECT id,execution_mode,retry_of_job_id,request_json FROM jobs ORDER BY id",
    )
    .fetch_all(db.pool())
    .await
    .unwrap();
    assert_eq!(before, after);
    for table in ["job_events", "job_artifacts", "job_steps"] {
        let count: i64 = sqlx::query_scalar(&format!("SELECT count(*) FROM {table}"))
            .fetch_one(db.pool())
            .await
            .unwrap();
        assert_eq!(count, 2);
    }
    let legacy_session: Option<String> =
        sqlx::query_scalar("SELECT session_id FROM job_agent_runs WHERE agent_run_id='old-root'")
            .fetch_one(db.pool())
            .await
            .unwrap();
    assert!(legacy_session.is_none());
    let sessions: i64 = sqlx::query_scalar("SELECT count(*) FROM job_agent_sessions")
        .fetch_one(db.pool())
        .await
        .unwrap();
    assert_eq!(sessions, 0);
    let invalid: i64 = sqlx::query_scalar("SELECT count(*) FROM pragma_foreign_key_check")
        .fetch_one(db.pool())
        .await
        .unwrap();
    assert_eq!(invalid, 0);
    assert!(
        sqlx::query("UPDATE jobs SET execution_mode='hierarchical_v3' WHERE id='v2'")
            .execute(db.pool())
            .await
            .is_err()
    );
    assert!(
        sqlx::query("DELETE FROM jobs WHERE id='dag'")
            .execute(db.pool())
            .await
            .is_err()
    );
    let indexes: Vec<String> =
        sqlx::query_scalar("SELECT name FROM sqlite_master WHERE type='index' AND tbl_name='jobs'")
            .fetch_all(db.pool())
            .await
            .unwrap();
    for name in ["jobs_idempotency", "jobs_queue", "jobs_requester"] {
        assert!(indexes.iter().any(|n| n == name));
    }
}
