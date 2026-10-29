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

#[test]
fn llm_provider_config_accepts_generation_and_context_bounds() {
    for (adapter, valid_payload) in [
        (
            "openai",
            serde_json::json!({
                "base_url": "https://api.example.com/v1",
                "model": "gpt-4.1-mini",
                "timeout_ms": 300_000,
                "max_tokens": 8_192,
                "context_limit_tokens": 128_000,
                "reasoning_effort": "medium",
                "temperature_percent": 70
            }),
        ),
        (
            "opencode_free",
            serde_json::json!({
                "model": "nemotron-3-ultra-free",
                "timeout_ms": 300_000,
                "max_tokens": 4_096,
                "context_limit_tokens": 64_000,
                "reasoning_effort": "high",
                "temperature_percent": 30
            }),
        ),
    ] {
        let canonical = crate::database::provider_config::validate(adapter, &valid_payload)
            .expect("valid LLM generation bounds should pass");
        let parsed: serde_json::Value = serde_json::from_str(&canonical).unwrap();
        assert_eq!(parsed["timeout_ms"], 300_000);
        assert_eq!(
            parsed["reasoning_effort"],
            valid_payload["reasoning_effort"]
        );
        assert_eq!(
            parsed["context_limit_tokens"],
            valid_payload["context_limit_tokens"]
        );
        assert_eq!(
            parsed["temperature_percent"],
            valid_payload["temperature_percent"]
        );
    }

    for (adapter, invalid_payload) in [
        (
            "openai",
            serde_json::json!({
                "base_url": "https://api.example.com/v1",
                "model": "gpt-4.1-mini",
                "context_limit_tokens": 1_024
            }),
        ),
        (
            "openai",
            serde_json::json!({
                "base_url": "https://api.example.com/v1",
                "model": "gpt-4.1-mini",
                "reasoning_effort": "extreme"
            }),
        ),
        (
            "openai",
            serde_json::json!({
                "base_url": "https://api.example.com/v1",
                "model": "gpt-4.1-mini",
                "temperature_percent": 250
            }),
        ),
        (
            "opencode_free",
            serde_json::json!({
                "model": "nemotron-3-ultra-free",
                "timeout_ms": 700_000
            }),
        ),
    ] {
        assert!(
            crate::database::provider_config::validate(adapter, &invalid_payload).is_err(),
            "expected {adapter} with {invalid_payload} to be rejected"
        );
    }

    let openai_keys: Vec<&str> = crate::providers::llm::openai::descriptor::DESCRIPTOR
        .config_schema
        .fields
        .iter()
        .map(|f| f.key)
        .collect();
    for expected in [
        "base_url",
        "model",
        "timeout_ms",
        "max_tokens",
        "context_limit_tokens",
        "reasoning_effort",
        "temperature_percent",
        "tags",
    ] {
        assert!(
            openai_keys.contains(&expected),
            "missing field {expected} in openai descriptor"
        );
    }
}

#[test]
fn llm_provider_config_validates_capability_tags_and_defaults_to_chat() {
    for (adapter, valid_payload, expected_tags) in [
        (
            "openai",
            serde_json::json!({
                "base_url": "https://api.example.com/v1",
                "model": "gpt-4o",
                "tags": ["chat", "vision"]
            }),
            serde_json::json!(["chat", "vision"]),
        ),
        (
            "openai",
            serde_json::json!({
                "base_url": "https://api.example.com/v1",
                "model": "sora-1",
                "tags": ["video"]
            }),
            serde_json::json!(["video"]),
        ),
        (
            "opencode_free",
            serde_json::json!({
                "model": "nemotron-3-ultra-free",
                "tags": ["chat", "reasoning"]
            }),
            serde_json::json!(["chat", "reasoning"]),
        ),
    ] {
        let canonical = crate::database::provider_config::validate(adapter, &valid_payload)
            .expect("valid LLM capability tags should pass");
        let parsed: serde_json::Value = serde_json::from_str(&canonical).unwrap();
        assert_eq!(parsed["tags"], expected_tags);
    }

    for (adapter, invalid_payload) in [
        (
            "openai",
            serde_json::json!({
                "base_url": "https://api.example.com/v1",
                "model": "gpt-4o",
                "tags": []
            }),
        ),
        (
            "openai",
            serde_json::json!({
                "base_url": "https://api.example.com/v1",
                "model": "gpt-4o",
                "tags": ["unknown_tag"]
            }),
        ),
        (
            "openai",
            serde_json::json!({
                "base_url": "https://api.example.com/v1",
                "model": "gpt-4o",
                "tags": ["chat", "chat"]
            }),
        ),
        (
            "opencode_free",
            serde_json::json!({
                "model": "nemotron-3-ultra-free",
                "tags": ["invalid"]
            }),
        ),
    ] {
        assert!(
            crate::database::provider_config::validate(adapter, &invalid_payload).is_err(),
            "expected {adapter} with {invalid_payload} to be rejected"
        );
    }

    assert!(crate::providers::llm::llm_config_supports_tag(
        r#"{"base_url":"https://api.example.com/v1","model":"gpt-4o"}"#,
        "chat"
    ));
    assert!(!crate::providers::llm::llm_config_supports_tag(
        r#"{"base_url":"https://api.example.com/v1","model":"sora","tags":["video"]}"#,
        "chat"
    ));
    assert!(crate::providers::llm::llm_config_supports_tag(
        r#"{"base_url":"https://api.example.com/v1","model":"sora","tags":["video"]}"#,
        "video"
    ));
}

#[tokio::test]
async fn template_llm_binding_requires_chat_capability_tag() {
    let database = Database::connect(&crate::config::DatabaseConfig {
        url: "sqlite::memory:".into(),
        max_connections: 1,
        ..Default::default()
    })
    .await
    .unwrap();
    sqlx::raw_sql(
        r#"
INSERT INTO agent_templates (id,key,name,language,prompt,enabled,created_at,updated_at)
VALUES (1,'tpl','Default','vi','prompt',1,1,1);
INSERT INTO providers (id,key,name,type,adapter,config_json,enabled,created_at,updated_at)
VALUES
 (1,'llm_chat','Chat Multimodal','llm','openai','{"base_url":"https://example.test/v1","model":"gpt-4o","tags":["chat","vision"]}',1,1,1),
 (2,'llm_video_only','Video Generator','llm','openai','{"base_url":"https://example.test/v1","model":"sora","tags":["video"]}',1,1,1);
"#,
    )
    .execute(database.pool())
    .await
    .unwrap();

    let chat_ref =
        crate::providers::config_catalog::ProviderRef::parse("llm_chat", Some("llm")).unwrap();
    database
        .bind_template_provider("tpl", "llm", &chat_ref, 1, "req-1")
        .await
        .expect("LLM with chat tag should bind to template");

    let video_ref =
        crate::providers::config_catalog::ProviderRef::parse("llm_video_only", Some("llm"))
            .unwrap();
    let err = database
        .bind_template_provider("tpl", "llm", &video_ref, 2, "req-2")
        .await
        .expect_err("video-only LLM without chat tag should be rejected for voice template");
    assert!(
        matches!(
            err,
            crate::database::writes::WriteError::Invalid("provider_capability_mismatch")
        ),
        "unexpected error: {err:?}"
    );
}
