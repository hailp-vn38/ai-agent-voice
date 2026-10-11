use super::*;

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
