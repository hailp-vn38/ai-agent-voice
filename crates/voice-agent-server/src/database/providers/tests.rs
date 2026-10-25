use super::*;

#[tokio::test]
async fn migration_removes_function_calling_flag_from_stored_providers() {
    let database = Database::connect(&crate::config::DatabaseConfig {
        url: "sqlite::memory:".into(),
        max_connections: 1,
        ..Default::default()
    })
    .await
    .unwrap();
    for (key, adapter, config) in [
        (
            "on",
            "openai",
            serde_json::json!({"base_url":"https://example.test/v1","model":"test","supports_function_calling":true,"supports_responses":true,"supports_structured_outputs":true}),
        ),
        (
            "off",
            "opencode_free",
            serde_json::json!({"model":"space-bunny-free","supports_function_calling":false,"supports_responses":false}),
        ),
        (
            "absent",
            "openai",
            serde_json::json!({"base_url":"https://example.test/v1","model":"test"}),
        ),
    ] {
        sqlx::query("INSERT INTO providers (key,name,type,adapter,config_json,created_at,updated_at) VALUES (?,?,'llm',?,?,1,1)")
            .bind(key).bind(key).bind(adapter).bind(config.to_string()).execute(database.pool()).await.unwrap();
    }
    let migration = include_str!("../../../migrations/0116_remove_function_calling_flag.sql");
    sqlx::raw_sql(migration)
        .execute(database.pool())
        .await
        .unwrap();
    let migration_flags =
        include_str!("../../../migrations/0135_remove_agent_job_provider_flags.sql");
    sqlx::raw_sql(migration_flags)
        .execute(database.pool())
        .await
        .unwrap();
    for (key, expected_revision) in [("on", 3_i64), ("off", 3), ("absent", 1)] {
        let (adapter, raw, revision): (String, String, i64) =
            sqlx::query_as("SELECT adapter,config_json,revision FROM providers WHERE key=?")
                .bind(key)
                .fetch_one(database.pool())
                .await
                .unwrap();
        let value: serde_json::Value = serde_json::from_str(&raw).unwrap();
        assert!(value.get("supports_function_calling").is_none());
        assert!(value.get("supports_responses").is_none());
        assert!(value.get("supports_structured_outputs").is_none());
        assert_eq!(revision, expected_revision);
        crate::database::provider_config::validate_raw(&adapter, &raw).unwrap();
    }
    sqlx::raw_sql(migration)
        .execute(database.pool())
        .await
        .unwrap();
    sqlx::raw_sql(migration_flags)
        .execute(database.pool())
        .await
        .unwrap();
    let revision: i64 = sqlx::query_scalar("SELECT revision FROM providers WHERE key='on'")
        .fetch_one(database.pool())
        .await
        .unwrap();
    assert_eq!(revision, 3);
}

#[tokio::test]
async fn prewarm_selects_enabled_defaults_and_device_overrides_with_their_assignments() {
    let database = Database::connect(&crate::config::DatabaseConfig {
        url: "sqlite::memory:".into(),
        max_connections: 1,
        ..Default::default()
    })
    .await
    .unwrap();
    sqlx::raw_sql(r#"
INSERT INTO agents (id,key,name,enabled,created_at,updated_at) VALUES (1,'on','On',1,1,1),(2,'off','Off',0,1,1);
INSERT INTO agent_templates (id,key,name,language,prompt,enabled,created_at,updated_at) VALUES
 (1,'default','Default','vi','prompt',1,1,1),(2,'override','Override','vi','prompt',1,1,1),
 (3,'disabled','Disabled','vi','prompt',0,1,1),(4,'unused','Unused','vi','prompt',1,1,1),
 (5,'unassigned','Unassigned','vi','prompt',1,1,1),(6,'off_default','Off default','vi','prompt',1,1,1);
INSERT INTO agent_template_assignments (agent_id,template_id,is_default,created_at) VALUES
 (1,1,1,1),(1,2,0,1),(1,3,0,1),(1,4,0,1),(2,6,1,1);
INSERT INTO devices (device_id,agent_id,template_id,enabled,created_at,updated_at) VALUES
 ('default',1,NULL,1,1,1),('override',1,2,1,1,1),('disabled',1,4,0,1,1),
 ('unassigned',1,5,1,1,1),('off_agent',2,6,1,1,1);
"#).execute(database.pool()).await.unwrap();
    let selected: std::collections::HashSet<_> = database
        .selected_prewarm_templates()
        .await
        .unwrap()
        .into_iter()
        .collect();
    assert_eq!(selected, std::collections::HashSet::from([(1,), (2,)]));
    for (device, expected) in [
        ("default", Some(1)),
        ("override", Some(2)),
        ("disabled", None),
        ("unassigned", None),
        ("off_agent", None),
        ("unknown", None),
    ] {
        assert_eq!(
            database.device_prewarm_template(device).await.unwrap(),
            expected,
            "{device}"
        );
    }
    sqlx::query("UPDATE devices SET template_id=3 WHERE device_id='override'")
        .execute(database.pool())
        .await
        .unwrap();
    assert_eq!(
        database.device_prewarm_template("override").await.unwrap(),
        None,
        "a disabled explicit override cannot warm the default instead"
    );
}
