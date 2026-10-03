use voice_agent_server::{
    config::{DatabaseConfig, DatabaseDevicesConfig},
    database::{Database, DeviceAdmissionError},
};

async fn database() -> Database {
    Database::connect(&DatabaseConfig {
        url: "sqlite::memory:".into(),
        max_connections: 1,
        ..Default::default()
    })
    .await
    .unwrap()
}

async fn provision(db: &Database) {
    sqlx::raw_sql(r#"
INSERT INTO agents (key,name,created_at,updated_at) VALUES ('agent','Agent',1,1);
INSERT INTO devices (device_id,agent_id,created_at,updated_at) VALUES ('device',1,1,1);
INSERT INTO agent_templates (key,name,language,prompt,created_at,updated_at) VALUES ('primary','Primary','vi','prompt',1,1);
INSERT INTO agent_template_assignments (agent_id,template_id,is_default,created_at) VALUES (1,1,1,1);
INSERT INTO providers (key,name,type,adapter,config_json,secret_ref,created_at,updated_at)
VALUES ('llm','LLM','llm','openai','{"base_url":"https://example.test/v1","model":"version-one"}','PRIVATE_SECRET_REFERENCE',1,1);
INSERT INTO template_provider_bindings (template_id,provider_type,provider_id,created_at,updated_at) VALUES (1,'llm',1,1,1);
"#).execute(db.pool()).await.unwrap();
}

#[tokio::test]
async fn admission_owns_exact_provider_version_after_patch_and_key_recreation() {
    let db = database().await;
    provision(&db).await;
    let old = db
        .admit_device("device", &DatabaseDevicesConfig::default())
        .await
        .unwrap();
    let old = old.assignments[0].bindings[0]
        .snapshot
        .as_ref()
        .unwrap()
        .clone();
    assert_eq!(old.revision, 1);
    assert_eq!(old.adapter, "openai");
    assert_eq!(old.secret_ref.as_deref(), Some("PRIVATE_SECRET_REFERENCE"));
    sqlx::query("UPDATE providers SET revision=2, config_json=? WHERE key='llm'")
        .bind(r#"{"base_url":"https://example.test/v1","model":"version-two"}"#)
        .execute(db.pool())
        .await
        .unwrap();
    let new = db
        .admit_device("device", &DatabaseDevicesConfig::default())
        .await
        .unwrap();
    let new = new.assignments[0].bindings[0].snapshot.as_ref().unwrap();
    assert_eq!(new.revision, 2);
    assert!(new.config_json.contains("version-two"));
    assert!(old.config_json.contains("version-one"));
    assert!(!format!("{old:?}").contains("PRIVATE_SECRET_REFERENCE"));
    assert!(!format!("{old:?}").contains("version-one"));

    sqlx::raw_sql(r#"
DELETE FROM template_provider_bindings;
DELETE FROM providers;
INSERT INTO providers (key,name,type,adapter,config_json,created_at,updated_at)
VALUES ('llm','LLM','llm','openai','{"base_url":"https://example.test/v1","model":"replacement"}',1,1);
INSERT INTO template_provider_bindings (template_id,provider_type,provider_id,created_at,updated_at)
SELECT 1,'llm',id,1,1 FROM providers WHERE key='llm';
"#).execute(db.pool()).await.unwrap();
    let replacement = db
        .admit_device("device", &DatabaseDevicesConfig::default())
        .await
        .unwrap();
    let replacement = replacement.assignments[0].bindings[0]
        .snapshot
        .as_ref()
        .unwrap();
    assert_ne!(replacement.id, old.id);
    assert_eq!(replacement.revision, 1);
}

#[tokio::test]
async fn admission_refuses_an_unbounded_assignment_graph() {
    let db = database().await;
    provision(&db).await;
    for index in 0..64 {
        let id = sqlx::query("INSERT INTO agent_templates (key,name,language,prompt,created_at,updated_at) VALUES (?,'Extra','vi','prompt',1,1)")
            .bind(format!("extra-{index}"))
            .execute(db.pool()).await.unwrap().last_insert_rowid();
        sqlx::query("INSERT INTO agent_template_assignments (agent_id,template_id,created_at) VALUES (1,?,1)")
            .bind(id).execute(db.pool()).await.unwrap();
    }
    assert!(matches!(
        db.admit_device("device", &DatabaseDevicesConfig::default())
            .await,
        Err(DeviceAdmissionError::Unavailable)
    ));
}

#[tokio::test]
async fn admission_bounds_total_snapshot_bytes_before_returning_configuration() {
    let db = database().await;
    provision(&db).await;
    sqlx::query("UPDATE agent_templates SET prompt=?")
        .bind("é".repeat(1024 * 1024))
        .execute(db.pool())
        .await
        .unwrap();
    assert!(matches!(
        db.admit_device("device", &DatabaseDevicesConfig::default())
            .await,
        Err(DeviceAdmissionError::Unavailable)
    ));
}

#[tokio::test]
async fn admission_marks_invalid_persisted_provider_config_unusable_without_copying_it_to_a_runtime_snapshot()
 {
    let db = database().await;
    provision(&db).await;
    sqlx::query("UPDATE providers SET config_json=?")
        .bind(r#"{"model":"invalid","token":"must-not-leave-the-db-snapshot-validator"}"#)
        .execute(db.pool())
        .await
        .unwrap();
    let graph = db
        .admit_device("device", &DatabaseDevicesConfig::default())
        .await
        .unwrap();
    assert!(graph.assignments[0].bindings[0].snapshot.is_none());
}
