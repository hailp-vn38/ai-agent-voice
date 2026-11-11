use super::harness::*;

#[tokio::test]
async fn startup_upgrades_known_legacy_built_in_skill_and_preserves_admin_content() {
    let temp_root = std::env::temp_dir().join(format!(
        "background-skills-upgrade-{}",
        uuid::Uuid::new_v4()
    ));
    let skills_dir = temp_root.join("skills");
    std::fs::create_dir_all(skills_dir.join("wiki-ingest")).unwrap();
    std::fs::write(
        skills_dir.join("wiki-ingest/SKILL.md"),
        include_str!("../fixtures/skills/wiki-ingest-legacy.md"),
    )
    .unwrap();

    let preserved = "---\nname: \"wiki-query\"\ndescription: \"Admin's local workflow\"\n---\n\n# Local workflow\n\nKeep this content.\n";
    std::fs::create_dir_all(skills_dir.join("wiki-query")).unwrap();
    std::fs::write(skills_dir.join("wiki-query/SKILL.md"), preserved).unwrap();
    std::fs::create_dir_all(skills_dir.join("custom-local")).unwrap();
    let custom =
        "---\nname: \"custom-local\"\ndescription: \"Custom workflow\"\n---\n\nKeep this too.\n";
    std::fs::write(skills_dir.join("custom-local/SKILL.md"), custom).unwrap();

    let config: AppConfig = toml::from_str(&format!(
        r#"
[server]
bind = "127.0.0.1:0"
public_ws_url = "ws://127.0.0.1:0/voice/v1/"
[provider_defaults]
vad = "test"
asr = "test"
llm = "test"
tts = "test"
[database]
url = "sqlite://{}"
[api]
enabled = true
admin_token = "background-test-token"
[orchestrator]
enabled = true
store_dir = "{}"
tasks_store_dir = "{}"
reports_dir = "{}"
work_dir = "{}"
prompts_dir = "{}"
skills_dir = "{}"
[wiki]
vault_dir = "{}"
"#,
        temp_root.join("vela.db").display(),
        temp_root.join("orch-store").display(),
        temp_root.join("tasks-store").display(),
        temp_root.join("reports").display(),
        temp_root.join("work").display(),
        temp_root.join("prompts").display(),
        skills_dir.display(),
        temp_root.join("wiki-vault").display(),
    ))
    .unwrap();

    let router = bootstrap_with_providers(config.clone(), Arc::new(ProviderSet::unavailable()))
        .await
        .unwrap();

    let upgraded = std::fs::read_to_string(skills_dir.join("wiki-ingest/SKILL.md")).unwrap();
    let expected = include_str!("../../builtin-skills/wiki-ingest/SKILL.md");
    assert_eq!(upgraded, expected);
    assert_eq!(
        std::fs::read_to_string(skills_dir.join("wiki-query/SKILL.md")).unwrap(),
        preserved
    );
    assert_eq!(
        std::fs::read_to_string(skills_dir.join("custom-local/SKILL.md")).unwrap(),
        custom
    );
    assert!(skills_dir.join("wiki-chat/SKILL.md").is_file());

    drop(router);
    std::fs::write(
        skills_dir.join("wiki-ingest/SKILL.md"),
        include_str!("../fixtures/skills/wiki-ingest-legacy.md"),
    )
    .unwrap();
    std::fs::write(
        skills_dir.join(".builtin-job-skill-versions.json"),
        json!({
            "wiki-ingest": "266b4f2a6588ce386bbcdbcde0fc2625934c2871a676014ede1b2363a86be833",
            "wiki-query": "049790e057e9a676e5be6b0ec3d20363740252fc05bc9602459a473aae63a32b"
        })
        .to_string(),
    )
    .unwrap();

    let mut second_config = config.clone();
    second_config.orchestrator.store_dir = temp_root.join("orch-store-second");
    second_config.orchestrator.tasks_store_dir = temp_root.join("tasks-store-second");
    second_config.orchestrator.work_dir = temp_root.join("work-second");
    second_config.orchestrator.reports_dir = temp_root.join("reports-second");
    let router = bootstrap_with_providers(second_config, Arc::new(ProviderSet::unavailable()))
        .await
        .unwrap();
    assert_eq!(
        std::fs::read_to_string(skills_dir.join("wiki-ingest/SKILL.md")).unwrap(),
        expected
    );
    assert_eq!(
        std::fs::read_to_string(skills_dir.join("wiki-query/SKILL.md")).unwrap(),
        preserved
    );
    drop(router);

    let stable_builtin = std::fs::read(skills_dir.join("wiki-ingest/SKILL.md")).unwrap();
    let stable_manifest =
        std::fs::read(skills_dir.join(".builtin-job-skill-versions.json")).unwrap();
    let mut third_config = config;
    third_config.orchestrator.store_dir = temp_root.join("orch-store-third");
    third_config.orchestrator.tasks_store_dir = temp_root.join("tasks-store-third");
    third_config.orchestrator.work_dir = temp_root.join("work-third");
    third_config.orchestrator.reports_dir = temp_root.join("reports-third");
    let router = bootstrap_with_providers(third_config, Arc::new(ProviderSet::unavailable()))
        .await
        .unwrap();
    assert_eq!(
        std::fs::read(skills_dir.join("wiki-ingest/SKILL.md")).unwrap(),
        stable_builtin
    );
    assert_eq!(
        std::fs::read(skills_dir.join(".builtin-job-skill-versions.json")).unwrap(),
        stable_manifest
    );
    drop(router);
}

#[tokio::test]
async fn background_settings_integrates_llm_providers_and_configures_subagents() {
    let temp_root =
        std::env::temp_dir().join(format!("background-agents-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&temp_root).unwrap();
    let config: AppConfig = toml::from_str(&format!(
        r#"
[server]
bind = "127.0.0.1:0"
public_ws_url = "ws://127.0.0.1:0/voice/v1/"
[provider_defaults]
vad = "test"
asr = "test"
llm = "test"
tts = "test"
[database]
url = "sqlite://{}"
[api]
enabled = true
admin_token = "background-test-token"
[orchestrator]
store_dir = "{}"
prompts_dir = "{}"
"#,
        temp_root.join("vela.db").display(),
        temp_root.join("orch-store").display(),
        temp_root.join("prompts").display(),
    ))
    .unwrap();
    let router = bootstrap_with_providers(config, Arc::new(ProviderSet::unavailable()))
        .await
        .unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}/api/admin", listener.local_addr().unwrap());
    let server = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    let client = Client::new();

    // 1. Create a database LLM provider with full model & generation bounds
    let created_resp = client
        .post(format!("{base}/providers"))
        .bearer_auth("background-test-token")
        .json(&json!({
            "name": "DeepSeek R1 Worker",
            "type": "llm",
            "adapter": "openai",
            "config_json": {
                "base_url": "https://api.deepseek.example/v1",
                "model": "deepseek-reasoner",
                "timeout_ms": 240_000,
                "max_tokens": 16_384,
                "context_limit_tokens": 128_000,
                "reasoning_effort": "high",
                "temperature_percent": 60
            }
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(created_resp.status(), StatusCode::CREATED);
    let created_provider: Value = created_resp.json().await.unwrap();
    let db_provider_key = created_provider["key"].as_str().unwrap().to_owned();

    // 1b. Create a database TTS provider (`type = "tts"`) and verify it is selectable for `tts_synthesize`
    let created_tts_resp = client
        .post(format!("{base}/providers"))
        .bearer_auth("background-test-token")
        .json(&json!({
            "name": "VieNeu Server TTS",
            "type": "tts",
            "adapter": "vieneu",
            "config_json": {}
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(created_tts_resp.status(), StatusCode::CREATED);
    let created_tts_provider: Value = created_tts_resp.json().await.unwrap();
    let db_tts_key = created_tts_provider["key"].as_str().unwrap().to_owned();

    let created_muse = client
        .post(format!("{base}/providers"))
        .bearer_auth("background-test-token")
        .json(&json!({
            "name": "Muse Cloud",
            "type": "llm",
            "adapter": "llm_gateway",
            "config_json": {"gateway_binding": "pool:muse", "model": "muse"}
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(created_muse.status(), StatusCode::CREATED);
    let muse_key = created_muse.json::<Value>().await.unwrap()["key"]
        .as_str()
        .unwrap()
        .to_owned();

    // 2. Fetch GET /api/admin/background/settings and verify provider catalog + 11 agents
    let initial: Value = client
        .get(format!("{base}/background/settings"))
        .bearer_auth("background-test-token")
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let embedded_ingestor_prompt =
        std::fs::read_to_string(temp_root.join("prompts/wiki_ingestor.md")).unwrap();
    assert!(embedded_ingestor_prompt.contains("# Wiki Ingest"));
    assert!(embedded_ingestor_prompt.contains("# Wiki Taxonomy"));
    let embedded_curator_prompt =
        std::fs::read_to_string(temp_root.join("prompts/wiki_curator.md")).unwrap();
    assert!(embedded_curator_prompt.contains("# Wiki Lint"));
    let llm_providers = initial["llm_providers"].as_array().unwrap();
    assert!(
        llm_providers
            .iter()
            .any(|p| p["ref"] == "orchestrator:default")
    );
    let db_entry = llm_providers
        .iter()
        .find(|p| p["ref"] == db_provider_key)
        .expect("newly created DB LLM provider must appear in background settings");
    assert_eq!(db_entry["model"], "deepseek-reasoner");
    assert_eq!(db_entry["max_tokens"], 16_384);
    assert_eq!(db_entry["context_limit_tokens"], 128_000);
    assert_eq!(db_entry["reasoning_effort"], "high");
    assert_eq!(db_entry["temperature_percent"], 60);

    let db_tts_entry = llm_providers
        .iter()
        .find(|p| p["ref"] == db_tts_key)
        .expect("newly created DB TTS provider must appear in background settings with tag `tts`");
    assert_eq!(db_tts_entry["adapter"], "vieneu");
    assert_eq!(db_tts_entry["tags"], json!(["tts"]));

    assert!(
        llm_providers.iter().find(|p| p["ref"] == muse_key).unwrap()["tags"]
            .as_array()
            .unwrap()
            .contains(&json!("agent_task"))
    );
    assert!(
        !db_entry["tags"]
            .as_array()
            .unwrap()
            .contains(&json!("agent_task"))
    );

    let agents = initial["agents"].as_array().unwrap();
    assert_eq!(agents.len(), 11);
    let cloud_worker = agents.iter().find(|a| a["id"] == "cloud_worker").unwrap();
    assert_eq!(cloud_worker["enabled"], false);
    assert_eq!(cloud_worker["default_enabled"], false);
    assert_eq!(
        cloud_worker["available_tools"],
        json!(["muse_task", "artifact_read"])
    );
    assert_eq!(
        cloud_worker["tool_capability_tags"]["muse_task"],
        "agent_task"
    );
    let daily_root = agents.iter().find(|a| a["id"] == "daily_root").unwrap();
    assert!(
        !daily_root["allowed_subagents"]
            .as_array()
            .unwrap()
            .contains(&json!("cloud_worker"))
    );
    let initial_collector = agents
        .iter()
        .find(|a| a["id"] == "source_collector")
        .unwrap();
    assert_eq!(initial_collector["flow_id"], "daily");
    assert_eq!(initial_collector["parent_id"], "daily_root");
    let initial_wiki_root = agents.iter().find(|a| a["id"] == "wiki_root").unwrap();
    assert_eq!(
        initial_wiki_root["allowed_subagents"],
        json!(["wiki_ingestor", "wiki_curator"])
    );

    // 3. Update global worker provider + specific subagent overrides via PUT /api/admin/background/settings
    let update_resp = client
        .put(format!("{base}/background/settings"))
        .bearer_auth("background-test-token")
        .header("If-Match", "\"1\"")
        .json(&json!({
            "enabled": true,
            "orchestrator_llm_provider": "orchestrator:default",
            "worker_llm_provider": db_provider_key,
            "agents": {
                "wiki_researcher": {
                    "enabled": true,
                    "provider_ref": db_provider_key,
                    "steps": 8,
                    "allowed_tools": ["skill", "wiki_tree", "wiki_query", "wiki_get"],
                    "allowed_skills": ["wiki-query"],
                    "description": "Custom fast researcher",
                    "system_prompt": "Custom Vietnamese researcher prompt."
                },
                "media_creator": {
                    "enabled": false
                },
                "cloud_worker": {
                    "enabled": true,
                    "tool_providers": {"muse_task": muse_key}
                },
                "media_processor": {
                    "enabled": true,
                    "tool_providers": {
                        "tts_synthesize": db_tts_key
                    }
                }
            }
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(update_resp.status(), StatusCode::OK);
    let updated: Value = update_resp.json().await.unwrap();
    assert_eq!(updated["revision"], 2);
    assert_eq!(updated["worker_llm_provider"], db_provider_key);

    let updated_agents = updated["agents"].as_array().unwrap();
    let cloud_worker = updated_agents
        .iter()
        .find(|a| a["id"] == "cloud_worker")
        .unwrap();
    assert_eq!(cloud_worker["enabled"], true);
    assert_eq!(cloud_worker["tool_providers"]["muse_task"], muse_key);
    assert_eq!(
        cloud_worker["effective_tool_providers"]["muse_task"],
        muse_key
    );
    let media_processor = updated_agents
        .iter()
        .find(|a| a["id"] == "media_processor")
        .unwrap();
    assert_eq!(
        media_processor["tool_capability_tags"]["tts_synthesize"],
        "tts"
    );
    assert_eq!(
        media_processor["tool_providers"]["tts_synthesize"],
        db_tts_key
    );
    assert_eq!(
        media_processor["effective_tool_providers"]["tts_synthesize"],
        db_tts_key
    );
    let researcher = updated_agents
        .iter()
        .find(|a| a["id"] == "wiki_researcher")
        .unwrap();
    assert_eq!(researcher["steps"], 8);
    assert_eq!(researcher["effective_provider_ref"], db_provider_key);
    assert_eq!(researcher["effective_model"], "deepseek-reasoner");
    assert_eq!(researcher["description"], "Custom fast researcher");
    assert_eq!(
        researcher["system_prompt"],
        "Custom Vietnamese researcher prompt."
    );
    assert_eq!(
        researcher["allowed_tools"],
        json!(["skill", "wiki_tree", "wiki_query", "wiki_get"])
    );

    let daily_root = updated_agents
        .iter()
        .find(|a| a["id"] == "daily_root")
        .unwrap();
    let daily_subs = daily_root["allowed_subagents"].as_array().unwrap();
    assert!(
        !daily_subs.iter().any(|s| s == "media_creator"),
        "disabled subagent media_creator must be removed from daily_root.allowed_subagents"
    );

    assert!(daily_subs.contains(&json!("cloud_worker")));
    let rejected_cloud_provider = client
        .put(format!("{base}/background/settings"))
        .bearer_auth("background-test-token")
        .header("If-Match", "\"2\"")
        .json(&json!({"enabled": true, "agents": {"cloud_worker": {
            "enabled": true, "tool_providers": {"muse_task": db_provider_key}
        }}}))
        .send()
        .await
        .unwrap();
    assert_eq!(rejected_cloud_provider.status(), StatusCode::BAD_REQUEST);

    // 4. Reject invalid provider_ref or disabling a primary root agent
    let bad_provider = client
        .put(format!("{base}/background/settings"))
        .bearer_auth("background-test-token")
        .header("If-Match", "\"2\"")
        .json(&json!({
            "enabled": true,
            "worker_llm_provider": "nonexistent:provider"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(bad_provider.status(), StatusCode::BAD_REQUEST);

    let disable_root = client
        .put(format!("{base}/background/settings"))
        .bearer_auth("background-test-token")
        .header("If-Match", "\"2\"")
        .json(&json!({
            "enabled": true,
            "agents": {
                "daily_root": { "enabled": false }
            }
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(disable_root.status(), StatusCode::BAD_REQUEST);

    server.abort();
}

#[tokio::test]
async fn background_skills_crud_etag_preconditions_and_agent_visibility() {
    let temp_root =
        std::env::temp_dir().join(format!("background-skills-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&temp_root).unwrap();
    let config: AppConfig = toml::from_str(&format!(
        r#"
[server]
bind = "127.0.0.1:0"
public_ws_url = "ws://127.0.0.1:0/voice/v1/"
[provider_defaults]
vad = "test"
asr = "test"
llm = "test"
tts = "test"
[database]
url = "sqlite://{}"
[api]
enabled = true
admin_token = "background-test-token"
[orchestrator]
enabled = true
store_dir = "{}"
prompts_dir = "{}"
skills_dir = "{}"
"#,
        temp_root.join("vela.db").display(),
        temp_root.join("orch-store").display(),
        temp_root.join("prompts").display(),
        temp_root.join("skills").display(),
    ))
    .unwrap();
    let router = bootstrap_with_providers(config, Arc::new(ProviderSet::unavailable()))
        .await
        .unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}/api/admin", listener.local_addr().unwrap());
    let server = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    let client = Client::new();

    // 1. GET /background/settings includes 8 built-in skills and dynamic subagent available_skills
    let initial: Value = client
        .get(format!("{base}/background/settings"))
        .bearer_auth("background-test-token")
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let skills = initial["skills"]
        .as_array()
        .expect("skills array in settings");
    assert_eq!(skills.len(), 11);
    for expected_id in [
        "daily-synthesis-staging",
        "daily-task-reminder",
        "trading-chart",
        "web-fetch-policy",
        "wiki-chat",
        "wiki-ingest",
        "wiki-lint",
        "wiki-page-format",
        "wiki-query",
        "wiki-taxonomy",
    ] {
        let entry = skills
            .iter()
            .find(|s| s["id"] == expected_id)
            .unwrap_or_else(|| panic!("missing built-in skill {expected_id}"));
        assert_eq!(entry["is_builtin"], true);
        assert_eq!(entry["is_modified"], false);
        assert!(!entry["etag"].as_str().unwrap().is_empty());
    }

    let agents = initial["agents"].as_array().unwrap();
    let daily_root = agents.iter().find(|a| a["id"] == "daily_root").unwrap();
    assert_eq!(daily_root["available_skills"], json!([]));
    let researcher = agents
        .iter()
        .find(|a| a["id"] == "wiki_researcher")
        .unwrap();
    let researcher_avail = researcher["available_skills"].as_array().unwrap();
    assert_eq!(researcher_avail.len(), 11);
    assert_eq!(researcher_avail[0], "wiki-query");
    assert_eq!(researcher_avail[1], "web-fetch-policy");
    assert_eq!(researcher_avail[2], "wiki-taxonomy");

    assert_eq!(
        agents.iter().find(|a| a["id"] == "wiki_ingestor").unwrap()["required_job_skills"],
        json!(["wiki-ingest", "wiki-taxonomy", "wiki-page-format"])
    );
    assert_eq!(
        agents.iter().find(|a| a["id"] == "wiki_curator").unwrap()["required_job_skills"],
        json!(["wiki-lint", "wiki-query"])
    );
    assert_eq!(researcher["required_job_skills"], json!([]));
    assert_eq!(daily_root["required_job_skills"], json!([]));
    for agent in agents {
        for required in agent["required_job_skills"].as_array().unwrap() {
            let required_id = required.as_str().unwrap();
            assert!(
                skills.iter().any(|skill| skill["id"] == required_id),
                "required Job Skill {required_id} must exist as a Built-in Job Skill"
            );
        }
    }
    let ingestor_prompt =
        std::fs::read_to_string(temp_root.join("prompts/wiki_ingestor.md")).unwrap();
    assert!(ingestor_prompt.contains("<required_job_skills>"));
    assert!(ingestor_prompt.contains("already loaded"));
    assert!(ingestor_prompt.contains("# Wiki Ingest"));
    assert!(ingestor_prompt.contains("# Wiki Taxonomy"));
    assert!(ingestor_prompt.contains("# Wiki Page Format"));
    let curator_prompt =
        std::fs::read_to_string(temp_root.join("prompts/wiki_curator.md")).unwrap();
    assert!(curator_prompt.contains("# Wiki Lint"));
    assert!(curator_prompt.contains("# Universal Wiki Query & Synthesis Skill"));
    let researcher_prompt =
        std::fs::read_to_string(temp_root.join("prompts/wiki_researcher.md")).unwrap();
    assert!(!researcher_prompt.contains("<required_job_skills>"));

    // 2. GET /background/skills/wiki-ingest returns detail + default + ETag header
    let detail_resp = client
        .get(format!("{base}/background/skills/wiki-ingest"))
        .bearer_auth("background-test-token")
        .send()
        .await
        .unwrap();
    assert_eq!(detail_resp.status(), StatusCode::OK);
    let header_etag = detail_resp
        .headers()
        .get("etag")
        .unwrap()
        .to_str()
        .unwrap()
        .to_owned();
    let detail: Value = detail_resp.json().await.unwrap();
    let etag1 = detail["etag"].as_str().unwrap().to_owned();
    assert_eq!(header_etag, format!("\"{etag1}\""));
    assert_eq!(detail["is_builtin"], true);
    assert_eq!(detail["is_modified"], false);
    assert!(detail["default"].is_object());
    let default_name = detail["default"]["name"].as_str().unwrap().to_owned();
    let default_desc = detail["default"]["description"]
        .as_str()
        .unwrap()
        .to_owned();
    let default_body = detail["default"]["body"].as_str().unwrap().to_owned();

    // 3. Edit built-in skill wiki-ingest with If-Match
    let edited_resp = client
        .put(format!("{base}/background/skills/wiki-ingest"))
        .bearer_auth("background-test-token")
        .header("If-Match", format!("\"{etag1}\""))
        .json(&json!({
            "name": "wiki-ingest",
            "description": "Customized ingest description",
            "body": "# Custom Ingest Workflow\n\n1. Verify frontmatter."
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(edited_resp.status(), StatusCode::OK);
    let edited: Value = edited_resp.json().await.unwrap();
    assert_eq!(edited["is_builtin"], true);
    assert_eq!(edited["is_modified"], true);
    assert_eq!(edited["description"], "Customized ingest description");
    let etag2 = edited["etag"].as_str().unwrap().to_owned();
    assert_ne!(etag1, etag2);
    let edited_ingestor_prompt =
        std::fs::read_to_string(temp_root.join("prompts/wiki_ingestor.md")).unwrap();
    assert!(edited_ingestor_prompt.contains("# Custom Ingest Workflow"));

    // Stale ETag on PUT -> 409 Conflict
    let stale_put = client
        .put(format!("{base}/background/skills/wiki-ingest"))
        .bearer_auth("background-test-token")
        .header("If-Match", format!("\"{etag1}\""))
        .json(&json!({
            "name": "wiki-ingest",
            "description": "Stale write",
            "body": "# Stale"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(stale_put.status(), StatusCode::CONFLICT);
    assert_eq!(
        stale_put.json::<Value>().await.unwrap()["error"]["code"],
        "revision_conflict"
    );

    // 4. Deleting a built-in skill is rejected with 400 builtin_skill_cannot_be_deleted
    let del_builtin = client
        .delete(format!("{base}/background/skills/wiki-ingest"))
        .bearer_auth("background-test-token")
        .header("If-Match", format!("\"{etag2}\""))
        .send()
        .await
        .unwrap();
    assert_eq!(del_builtin.status(), StatusCode::BAD_REQUEST);
    assert_eq!(
        del_builtin.json::<Value>().await.unwrap()["error"]["code"],
        "builtin_skill_cannot_be_deleted"
    );

    // 5. Restore default on built-in skill via PUT using `default` fields -> is_modified becomes false
    let restored_resp = client
        .put(format!("{base}/background/skills/wiki-ingest"))
        .bearer_auth("background-test-token")
        .header("If-Match", format!("\"{etag2}\""))
        .json(&json!({
            "name": default_name,
            "description": default_desc,
            "body": default_body
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(restored_resp.status(), StatusCode::OK);
    let restored: Value = restored_resp.json().await.unwrap();
    assert_eq!(restored["is_modified"], false);

    // 6. Create a Custom Job Skill with If-None-Match: *
    let create_resp = client
        .put(format!("{base}/background/skills/custom-seo-audit"))
        .bearer_auth("background-test-token")
        .header("If-None-Match", "*")
        .json(&json!({
            "name": "custom-seo-audit",
            "description": "Audit technical SEO: headings, links, and metadata",
            "body": "# SEO Audit\n\nCheck H1 and canonical tags."
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(create_resp.status(), StatusCode::CREATED);
    let created_custom: Value = create_resp.json().await.unwrap();
    assert_eq!(created_custom["id"], "custom-seo-audit");
    assert_eq!(created_custom["is_builtin"], false);
    assert_eq!(created_custom["is_modified"], false);
    assert!(created_custom["default"].is_null());
    let custom_etag = created_custom["etag"].as_str().unwrap().to_owned();

    // Duplicate create with If-None-Match: * -> 409 already_exists
    let dup_create = client
        .put(format!("{base}/background/skills/custom-seo-audit"))
        .bearer_auth("background-test-token")
        .header("If-None-Match", "*")
        .json(&json!({
            "name": "custom-seo-audit-2",
            "description": "Duplicate id",
            "body": "# Dup"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(dup_create.status(), StatusCode::CONFLICT);
    assert_eq!(
        dup_create.json::<Value>().await.unwrap()["error"]["code"],
        "already_exists"
    );

    // Invalid quote in frontmatter name -> 400 invalid_skill_name
    let bad_name = client
        .put(format!("{base}/background/skills/bad-quote-skill"))
        .bearer_auth("background-test-token")
        .header("If-None-Match", "*")
        .json(&json!({
            "name": "bad \"quote\"",
            "description": "ok",
            "body": "# Body"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(bad_name.status(), StatusCode::BAD_REQUEST);
    assert_eq!(
        bad_name.json::<Value>().await.unwrap()["error"]["code"],
        "invalid_skill_name"
    );

    // 7. Assign custom-seo-audit to wiki_researcher via PUT /background/settings
    let assign_resp = client
        .put(format!("{base}/background/settings"))
        .bearer_auth("background-test-token")
        .header("If-Match", "\"1\"")
        .json(&json!({
            "enabled": true,
            "agents": {
                "wiki_researcher": {
                    "enabled": true,
                    "allowed_skills": ["wiki-query", "custom-seo-audit"]
                },
                "wiki_ingestor": {
                    "enabled": true,
                    "allowed_skills": [],
                    "system_prompt": "Admin's Wiki ingestor prompt.\n<required_job_skills>\nOld server snapshot.\n</required_job_skills>\nKeep this ending."
                }
            }
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(assign_resp.status(), StatusCode::OK);
    let assigned_settings: Value = assign_resp.json().await.unwrap();
    let researcher_after = assigned_settings["agents"]
        .as_array()
        .unwrap()
        .iter()
        .find(|a| a["id"] == "wiki_researcher")
        .unwrap();
    assert_eq!(
        researcher_after["allowed_skills"],
        json!(["wiki-query", "custom-seo-audit"])
    );
    let ingestor_after = assigned_settings["agents"]
        .as_array()
        .unwrap()
        .iter()
        .find(|a| a["id"] == "wiki_ingestor")
        .unwrap();
    assert_eq!(
        ingestor_after["allowed_skills"],
        json!(["wiki-ingest", "wiki-taxonomy", "wiki-page-format"])
    );
    assert_eq!(
        ingestor_after["system_prompt"],
        "Admin's Wiki ingestor prompt.\n<required_job_skills>\nOld server snapshot.\n</required_job_skills>\nKeep this ending."
    );
    let ingestor_custom_prompt =
        std::fs::read_to_string(temp_root.join("prompts/wiki_ingestor.md")).unwrap();
    assert!(ingestor_custom_prompt.starts_with("Admin's Wiki ingestor prompt."));
    assert!(ingestor_custom_prompt.contains("Keep this ending."));
    assert!(ingestor_custom_prompt.contains("# Wiki Ingest"));
    assert!(!ingestor_custom_prompt.contains("Old server snapshot."));
    assert_eq!(
        ingestor_custom_prompt
            .matches("<required_job_skills>")
            .count(),
        1
    );

    // An invalid disk copy must fall back to the embedded Required Job Skill when prompts refresh.
    std::fs::write(
        temp_root.join("skills/wiki-ingest/SKILL.md"),
        "malformed required skill on disk",
    )
    .unwrap();
    let fallback_sync = client
        .put(format!("{base}/background/settings"))
        .bearer_auth("background-test-token")
        .header("If-Match", "\"2\"")
        .json(&json!({
            "enabled": true,
            "agents": { "wiki_ingestor": { "enabled": true, "allowed_skills": [] } }
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(fallback_sync.status(), StatusCode::OK);
    let fallback_prompt =
        std::fs::read_to_string(temp_root.join("prompts/wiki_ingestor.md")).unwrap();
    assert!(fallback_prompt.contains("Ingest integrates one raw source"));
    assert!(!fallback_prompt.contains("malformed required skill on disk"));

    let custom_detail: Value = client
        .get(format!("{base}/background/skills/custom-seo-audit"))
        .bearer_auth("background-test-token")
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(custom_detail["used_by_agents"], json!(["wiki_researcher"]));

    // 8. Delete custom-seo-audit with If-Match -> 204 No Content, and read-time filtering removes it from wiki_researcher
    let wrong_del = client
        .delete(format!("{base}/background/skills/custom-seo-audit"))
        .bearer_auth("background-test-token")
        .header("If-Match", "\"wrong-etag\"")
        .send()
        .await
        .unwrap();
    assert_eq!(wrong_del.status(), StatusCode::CONFLICT);

    let ok_del = client
        .delete(format!("{base}/background/skills/custom-seo-audit"))
        .bearer_auth("background-test-token")
        .header("If-Match", format!("\"{custom_etag}\""))
        .send()
        .await
        .unwrap();
    assert_eq!(ok_del.status(), StatusCode::NO_CONTENT);

    let get_deleted = client
        .get(format!("{base}/background/skills/custom-seo-audit"))
        .bearer_auth("background-test-token")
        .send()
        .await
        .unwrap();
    assert_eq!(get_deleted.status(), StatusCode::NOT_FOUND);

    let after_del_settings: Value = client
        .get(format!("{base}/background/settings"))
        .bearer_auth("background-test-token")
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let researcher_pruned = after_del_settings["agents"]
        .as_array()
        .unwrap()
        .iter()
        .find(|a| a["id"] == "wiki_researcher")
        .unwrap();
    assert_eq!(
        researcher_pruned["allowed_skills"],
        json!(["wiki-query"]),
        "deleted custom skill must be filtered out of effective allowed_skills"
    );

    server.abort();
}

#[tokio::test]
async fn malformed_skill_files_do_not_block_startup_settings_or_provider_saves() {
    let temp_root = std::env::temp_dir().join(format!(
        "background-skills-recovery-{}",
        uuid::Uuid::new_v4()
    ));
    let skills_dir = temp_root.join("skills");
    let broken_custom = skills_dir.join("custom-broken");
    let broken_builtin = skills_dir.join("wiki-chat");
    std::fs::create_dir_all(&broken_custom).unwrap();
    std::fs::create_dir_all(&broken_builtin).unwrap();
    let custom_bad_bytes = b"---\nname: broken\n";
    let builtin_bad_bytes = b"not valid built-in frontmatter\n";
    std::fs::write(broken_custom.join("SKILL.md"), custom_bad_bytes).unwrap();
    std::fs::write(broken_builtin.join("SKILL.md"), builtin_bad_bytes).unwrap();
    std::fs::create_dir_all(skills_dir.join("empty-skill-dir")).unwrap();
    std::fs::create_dir_all(skills_dir.join("Bad ID")).unwrap();

    let store_dir = temp_root.join("orch-store");
    std::fs::create_dir_all(&store_dir).unwrap();
    std::fs::write(
        store_dir.join("agent_settings.json"),
        r#"{
          "revision": 1,
          "orchestrator_llm_provider": "orchestrator:default",
          "worker_llm_provider": "orchestrator:default",
          "agents": {
            "wiki_researcher": {
              "enabled": true,
              "allowed_skills": ["custom-broken", "wiki-query"]
            }
          }
        }"#,
    )
    .unwrap();
    let config: AppConfig = toml::from_str(&format!(
        r#"
[server]
bind = "127.0.0.1:0"
public_ws_url = "ws://127.0.0.1:0/voice/v1/"
[provider_defaults]
vad = "test"
asr = "test"
llm = "test"
tts = "test"
[database]
url = "sqlite://{}"
[api]
enabled = true
admin_token = "background-test-token"
[orchestrator]
enabled = true
store_dir = "{}"
prompts_dir = "{}"
skills_dir = "{}"
"#,
        temp_root.join("vela.db").display(),
        store_dir.display(),
        temp_root.join("prompts").display(),
        skills_dir.display(),
    ))
    .unwrap();
    let router = bootstrap_with_providers(config, Arc::new(ProviderSet::unavailable()))
        .await
        .expect("malformed custom and built-in skills must not prevent startup");

    let default_builtin = std::fs::read(skills_dir.join("wiki-chat/SKILL.md")).unwrap();
    assert_eq!(
        default_builtin,
        include_bytes!("../../builtin-skills/wiki-chat/SKILL.md")
    );
    assert_eq!(
        std::fs::read(broken_custom.join("SKILL.md")).unwrap(),
        custom_bad_bytes
    );
    let backups = std::fs::read_dir(&broken_builtin)
        .unwrap()
        .flatten()
        .filter(|entry| {
            entry
                .file_name()
                .to_string_lossy()
                .starts_with(".SKILL.md.corrupt-")
        })
        .collect::<Vec<_>>();
    assert_eq!(backups.len(), 1);
    assert_eq!(std::fs::read(backups[0].path()).unwrap(), builtin_bad_bytes);

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}/api/admin", listener.local_addr().unwrap());
    let server = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    let client = Client::new();

    let settings: Value = client
        .get(format!("{base}/background/settings"))
        .bearer_auth("background-test-token")
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let skills = settings["skills"].as_array().unwrap();
    assert!(skills.iter().any(|skill| skill["id"] == "wiki-chat"));
    assert!(!skills.iter().any(|skill| skill["id"] == "custom-broken"));
    let researcher = settings["agents"]
        .as_array()
        .unwrap()
        .iter()
        .find(|agent| agent["id"] == "wiki_researcher")
        .unwrap();
    assert_eq!(researcher["allowed_skills"], json!(["wiki-query"]));

    let broken_detail = client
        .get(format!("{base}/background/skills/custom-broken"))
        .bearer_auth("background-test-token")
        .send()
        .await
        .unwrap();
    assert_eq!(broken_detail.status(), StatusCode::NOT_FOUND);
    let builtin_detail: Value = client
        .get(format!("{base}/background/skills/wiki-chat"))
        .bearer_auth("background-test-token")
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(
        builtin_detail["related_files"]
            .as_array()
            .unwrap()
            .iter()
            .all(|file| !file.as_str().unwrap().contains("corrupt"))
    );

    let settings_save = client
        .put(format!("{base}/background/settings"))
        .bearer_auth("background-test-token")
        .header("If-Match", "\"1\"")
        .json(&json!({
            "enabled": true,
            "orchestrator_llm_provider": "orchestrator:default",
            "worker_llm_provider": "orchestrator:default",
            "agents": {
                "wiki_researcher": {"enabled": true, "allowed_skills": ["wiki-query"]}
            }
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(
        settings_save.status(),
        StatusCode::OK,
        "settings save response: {}",
        settings_save.text().await.unwrap()
    );

    let provider_save = client
        .post(format!("{base}/providers"))
        .bearer_auth("background-test-token")
        .json(&json!({
            "name": "Malformed Skill Resilience Provider",
            "type": "llm",
            "adapter": "openai",
            "config_json": {
                "base_url": "https://api.example.invalid/v1",
                "model": "test-model",
                "timeout_ms": 30_000,
                "max_tokens": 2048,
                "context_limit_tokens": 32_000
            }
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(provider_save.status(), StatusCode::CREATED);

    server.abort();
    let _ = std::fs::remove_dir_all(temp_root);
}

#[tokio::test]
async fn no_configured_skill_directory_has_empty_admin_catalog() {
    let temp_root =
        std::env::temp_dir().join(format!("background-skills-no-dir-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&temp_root).unwrap();
    let mut config: AppConfig = toml::from_str(&format!(
        r#"
[server]
bind = "127.0.0.1:0"
public_ws_url = "ws://127.0.0.1:0/voice/v1/"
[provider_defaults]
vad = "test"
asr = "test"
llm = "test"
tts = "test"
[database]
url = "sqlite://{}"
[api]
enabled = true
admin_token = "background-test-token"
[orchestrator]
store_dir = "{}"
prompts_dir = "{}"
"#,
        temp_root.join("vela.db").display(),
        temp_root.join("orch-store").display(),
        temp_root.join("prompts").display(),
    ))
    .unwrap();
    config.orchestrator.skills_dir = None;
    let router = bootstrap_with_providers(config, Arc::new(ProviderSet::unavailable()))
        .await
        .unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}/api/admin", listener.local_addr().unwrap());
    let server = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    let client = Client::new();

    let settings: Value = client
        .get(format!("{base}/background/settings"))
        .bearer_auth("background-test-token")
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(settings["skills"], json!([]));
    let detail = client
        .get(format!("{base}/background/skills/wiki-chat"))
        .bearer_auth("background-test-token")
        .send()
        .await
        .unwrap();
    assert_eq!(detail.status(), StatusCode::NOT_FOUND);

    server.abort();
    let _ = std::fs::remove_dir_all(temp_root);
}

#[tokio::test]
async fn background_skill_tool_requirements_warn_without_granting_tools() {
    let temp_root =
        std::env::temp_dir().join(format!("background-skill-tools-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&temp_root).unwrap();
    let config: AppConfig = toml::from_str(&format!(
        r#"
[server]
bind = "127.0.0.1:0"
public_ws_url = "ws://127.0.0.1:0/voice/v1/"
[provider_defaults]
vad = "test"
asr = "test"
llm = "test"
tts = "test"
[database]
url = "sqlite://{}"
[api]
enabled = true
admin_token = "background-test-token"
[orchestrator]
store_dir = "{}"
prompts_dir = "{}"
skills_dir = "{}"
"#,
        temp_root.join("vela.db").display(),
        temp_root.join("orch-store").display(),
        temp_root.join("prompts").display(),
        temp_root.join("skills").display(),
    ))
    .unwrap();
    let router = bootstrap_with_providers(config, Arc::new(ProviderSet::unavailable()))
        .await
        .unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}/api/admin", listener.local_addr().unwrap());
    let server = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    let client = Client::new();

    let created = client
        .put(format!("{base}/background/skills/custom-tool-aware"))
        .bearer_auth("background-test-token")
        .header("If-None-Match", "*")
        .json(&json!({
            "name": "custom-tool-aware",
            "description": "Tool-aware workflow",
            "body": "# Workflow\n\nUse the requested tool.",
            "requires_tools": ["add_to_sources"]
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(created.status(), StatusCode::CREATED);
    let created_skill: Value = created.json().await.unwrap();
    assert_eq!(created_skill["requires_tools"], json!(["add_to_sources"]));
    let first_etag = created_skill["etag"].as_str().unwrap().to_owned();

    let listed: Value = client
        .get(format!("{base}/background/settings"))
        .bearer_auth("background-test-token")
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let listed_skill = listed["skills"]
        .as_array()
        .unwrap()
        .iter()
        .find(|skill| skill["id"] == "custom-tool-aware")
        .unwrap();
    assert_eq!(listed_skill["requires_tools"], json!(["add_to_sources"]));

    let invalid = client
        .put(format!("{base}/background/skills/invalid-tool-requirement"))
        .bearer_auth("background-test-token")
        .header("If-None-Match", "*")
        .json(&json!({
            "name": "invalid-tool-requirement",
            "description": "Invalid tool name",
            "body": "# Workflow\n\nUse an invalid tool.",
            "requires_tools": ["not a tool"]
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(invalid.status(), StatusCode::BAD_REQUEST);
    assert_eq!(
        invalid.json::<Value>().await.unwrap()["error"]["code"],
        "invalid_skill_requires_tools"
    );

    let updated = client
        .put(format!("{base}/background/skills/custom-tool-aware"))
        .bearer_auth("background-test-token")
        .header("If-Match", format!("\"{first_etag}\""))
        .json(&json!({
            "name": "custom-tool-aware",
            "description": "Tool-aware workflow",
            "body": "# Workflow\n\nUse the requested tool.",
            "requires_tools": ["wiki_query"]
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(updated.status(), StatusCode::OK);
    let updated_skill: Value = updated.json().await.unwrap();
    assert_eq!(updated_skill["requires_tools"], json!(["wiki_query"]));
    assert_ne!(updated_skill["etag"], first_etag);

    let custom_agent = client
        .post(format!("{base}/background/agents"))
        .bearer_auth("background-test-token")
        .header("If-Match", "\"1\"")
        .json(&json!({
            "id": "custom_skill_tool_agent",
            "flow_id": "daily",
            "enabled": true,
            "steps": 10,
            "allowed_tools": ["fetch_html"],
            "allowed_skills": ["custom-tool-aware"],
            "description": "Custom test agent",
            "system_prompt": "Use the assigned skill."
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(custom_agent.status(), StatusCode::CREATED);
    let custom_settings: Value = custom_agent.json().await.unwrap();
    let custom_projection = custom_settings["agents"]
        .as_array()
        .unwrap()
        .iter()
        .find(|agent| agent["id"] == "custom_skill_tool_agent")
        .unwrap();
    assert_eq!(
        custom_projection["allowed_tools"],
        json!(["skill", "fetch_html"])
    );
    assert_eq!(
        custom_projection["tool_requirement_gaps"],
        json!([
            {"skill_id": "custom-tool-aware", "missing_tools": ["wiki_query"]}
        ])
    );

    // A Built-in Sub-Agent can save an incomplete grant; the gap is advisory and disappears
    // after the missing tool is explicitly added to its effective grants.
    let incomplete = client
        .put(format!("{base}/background/settings"))
        .bearer_auth("background-test-token")
        .header("If-Match", "\"2\"")
        .json(&json!({
            "enabled": true,
            "agents": {
                "source_collector": {
                    "enabled": true,
                    "allowed_tools": ["skill", "fetch_html"],
                    "allowed_skills": ["custom-tool-aware"]
                }
            }
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(incomplete.status(), StatusCode::OK);
    let incomplete_settings: Value = incomplete.json().await.unwrap();
    let source_collector = incomplete_settings["agents"]
        .as_array()
        .unwrap()
        .iter()
        .find(|agent| agent["id"] == "source_collector")
        .unwrap();
    assert_eq!(
        source_collector["allowed_tools"],
        json!(["skill", "fetch_html"])
    );
    assert_eq!(
        source_collector["tool_requirement_gaps"],
        json!([
            {"skill_id": "custom-tool-aware", "missing_tools": ["wiki_query"]}
        ])
    );

    let complete = client
        .put(format!("{base}/background/settings"))
        .bearer_auth("background-test-token")
        .header("If-Match", "\"3\"")
        .json(&json!({
            "enabled": true,
            "agents": {
                "source_collector": {
                    "enabled": true,
                    "allowed_tools": ["skill", "fetch_html", "wiki_query"],
                    "allowed_skills": ["custom-tool-aware"]
                }
            }
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(complete.status(), StatusCode::OK);
    let complete_settings: Value = complete.json().await.unwrap();
    let source_collector = complete_settings["agents"]
        .as_array()
        .unwrap()
        .iter()
        .find(|agent| agent["id"] == "source_collector")
        .unwrap();
    assert_eq!(source_collector["tool_requirement_gaps"], json!([]));

    let no_requirements = client
        .put(format!("{base}/background/skills/custom-no-tools"))
        .bearer_auth("background-test-token")
        .header("If-None-Match", "*")
        .json(&json!({
            "name": "custom-no-tools",
            "description": "No tool metadata",
            "body": "# Workflow\n\nNo tools declared."
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(no_requirements.status(), StatusCode::CREATED);
    let no_requirements: Value = no_requirements.json().await.unwrap();
    assert_eq!(no_requirements["requires_tools"], json!([]));

    let no_requirements_assigned = client
        .put(format!("{base}/background/settings"))
        .bearer_auth("background-test-token")
        .header("If-Match", "\"4\"")
        .json(&json!({
            "enabled": true,
            "agents": {
                "wiki_researcher": {
                    "enabled": true,
                    "allowed_skills": ["custom-no-tools"]
                }
            }
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(no_requirements_assigned.status(), StatusCode::OK);
    let no_requirements_assigned: Value = no_requirements_assigned.json().await.unwrap();
    let researcher = no_requirements_assigned["agents"]
        .as_array()
        .unwrap()
        .iter()
        .find(|agent| agent["id"] == "wiki_researcher")
        .unwrap();
    assert_eq!(researcher["tool_requirement_gaps"], json!([]));

    server.abort();
}

#[tokio::test]
async fn background_settings_save_survives_deleted_provider_refs() {
    let temp_root =
        std::env::temp_dir().join(format!("background-stale-refs-{}", uuid::Uuid::new_v4()));
    let store_dir = temp_root.join("orch-store");
    std::fs::create_dir_all(&store_dir).unwrap();
    std::fs::write(
        store_dir.join("agent_settings.json"),
        json!({
            "revision": 5,
            "orchestrator_llm_provider": "deleted-orchestrator",
            "worker_llm_provider": "deleted-worker",
            "wiki_chat_llm_provider": "deleted-wiki-chat",
            "agents": {
                "wiki_researcher": {"enabled": true, "provider_ref": "deleted-researcher"},
                "media_processor": {
                    "enabled": true,
                    "tool_providers": {"tts_synthesize": "deleted-tts"}
                }
            }
        })
        .to_string(),
    )
    .unwrap();
    let config: AppConfig = toml::from_str(&format!(
        r#"
[server]
bind = "127.0.0.1:0"
public_ws_url = "ws://127.0.0.1:0/voice/v1/"
[provider_defaults]
vad = "test"
asr = "test"
llm = "test"
tts = "test"
[database]
url = "sqlite://{}"
[api]
enabled = true
admin_token = "background-test-token"
[orchestrator]
store_dir = "{}"
prompts_dir = "{}"
"#,
        temp_root.join("vela.db").display(),
        store_dir.display(),
        temp_root.join("prompts").display(),
    ))
    .unwrap();
    let router = bootstrap_with_providers(config, Arc::new(ProviderSet::unavailable()))
        .await
        .unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}/api/admin", listener.local_addr().unwrap());
    let server = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    let client = Client::new();
    let agent = |settings: &Value, id: &str| {
        settings["agents"]
            .as_array()
            .unwrap()
            .iter()
            .find(|a| a["id"] == id)
            .unwrap()
            .clone()
    };

    let initial: Value = client
        .get(format!("{base}/background/settings"))
        .bearer_auth("background-test-token")
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(initial["orchestrator_llm_provider"], "orchestrator:default");
    assert_eq!(initial["worker_llm_provider"], "orchestrator:default");
    assert_eq!(initial["wiki_chat_llm_provider"], Value::Null);
    assert_eq!(
        agent(&initial, "wiki_researcher")["provider_ref"],
        Value::Null
    );
    assert_eq!(
        agent(&initial, "media_processor")["tool_providers"],
        json!({})
    );
    let revision = initial["revision"].as_i64().unwrap();

    // A newly chosen ref is still rejected, even while stale refs are carried over.
    let rejected = client
        .put(format!("{base}/background/settings"))
        .bearer_auth("background-test-token")
        .header("If-Match", format!("\"{revision}\""))
        .json(&json!({
            "enabled": true,
            "agents": {"source_collector": {"enabled": true, "provider_ref": "never-existed"}}
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(rejected.status(), StatusCode::BAD_REQUEST);

    let saved = client
        .put(format!("{base}/background/settings"))
        .bearer_auth("background-test-token")
        .header("If-Match", format!("\"{revision}\""))
        .json(&json!({
            "enabled": true,
            "agents": {"source_collector": {"enabled": true, "steps": 7}}
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(saved.status(), StatusCode::OK);
    let saved: Value = saved.json().await.unwrap();
    assert_eq!(agent(&saved, "source_collector")["steps"], 7);

    // An unrelated save must not erase the refs: a disabled provider can come back.
    let persisted: Value = serde_json::from_str(
        &std::fs::read_to_string(store_dir.join("agent_settings.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(persisted["worker_llm_provider"], "deleted-worker");
    assert_eq!(persisted["wiki_chat_llm_provider"], "deleted-wiki-chat");
    assert_eq!(
        persisted["agents"]["wiki_researcher"]["provider_ref"],
        "deleted-researcher"
    );

    let created = client
        .post(format!("{base}/background/agents"))
        .bearer_auth("background-test-token")
        .header("If-Match", format!("\"{}\"", saved["revision"]))
        .json(&json!({
            "id": "custom_after_delete",
            "flow_id": "daily",
            "description": "Created while stale refs remain",
            "system_prompt": "Do the work."
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(created.status(), StatusCode::CREATED);

    server.abort();
}

#[tokio::test]
async fn background_skill_summary_reports_default_updates_separately_from_edits() {
    let temp_root =
        std::env::temp_dir().join(format!("background-skill-update-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&temp_root).unwrap();
    let skills_dir = temp_root.join("skills");
    let config: AppConfig = toml::from_str(&format!(
        r#"
[server]
bind = "127.0.0.1:0"
public_ws_url = "ws://127.0.0.1:0/voice/v1/"
[provider_defaults]
vad = "test"
asr = "test"
llm = "test"
tts = "test"
[database]
url = "sqlite://{}"
[api]
enabled = true
admin_token = "background-test-token"
[orchestrator]
store_dir = "{}"
prompts_dir = "{}"
skills_dir = "{}"
"#,
        temp_root.join("vela.db").display(),
        temp_root.join("orch-store").display(),
        temp_root.join("prompts").display(),
        skills_dir.display(),
    ))
    .unwrap();
    let router = bootstrap_with_providers(config, Arc::new(ProviderSet::unavailable()))
        .await
        .unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}/api/admin", listener.local_addr().unwrap());
    let server = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    let client = Client::new();

    let list: Value = client
        .get(format!("{base}/background/settings"))
        .bearer_auth("background-test-token")
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let wiki_query = list["skills"]
        .as_array()
        .unwrap()
        .iter()
        .find(|skill| skill["id"] == "wiki-query")
        .unwrap();
    assert_eq!(wiki_query["is_modified"], false);
    assert_eq!(wiki_query["default_has_update"], false);

    let wiki_query_detail: Value = client
        .get(format!("{base}/background/skills/wiki-query"))
        .bearer_auth("background-test-token")
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let original_etag = wiki_query_detail["etag"].as_str().unwrap().to_owned();
    let edit: Value = client
        .put(format!("{base}/background/skills/wiki-query"))
        .bearer_auth("background-test-token")
        .header("If-Match", format!("\"{original_etag}\""))
        .json(&json!({
            "name": "wiki-query",
            "description": "Admin edit with current base",
            "body": "# Admin edit\n\nKeep my workflow."
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(edit["is_modified"], true);
    assert_eq!(edit["default_has_update"], false);

    let manifest_path = skills_dir.join(".builtin-job-skill-versions.json");
    let mut manifest: Value =
        serde_json::from_slice(&std::fs::read(&manifest_path).unwrap()).unwrap();
    manifest["wiki-query"] = json!("older-default-hash");
    manifest["wiki-ingest"] = json!("older-default-hash");
    std::fs::write(&manifest_path, serde_json::to_vec(&manifest).unwrap()).unwrap();

    let updated_edit: Value = client
        .get(format!("{base}/background/skills/wiki-query"))
        .bearer_auth("background-test-token")
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(updated_edit["is_modified"], true);
    assert_eq!(updated_edit["default_has_update"], true);

    let unchanged_default: Value = client
        .get(format!("{base}/background/skills/wiki-ingest"))
        .bearer_auth("background-test-token")
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(unchanged_default["is_modified"], false);
    assert_eq!(unchanged_default["default_has_update"], false);

    let latest_etag = updated_edit["etag"].as_str().unwrap();
    let defaults = &updated_edit["default"];
    let restored: Value = client
        .put(format!("{base}/background/skills/wiki-query"))
        .bearer_auth("background-test-token")
        .header("If-Match", format!("\"{latest_etag}\""))
        .json(&json!({
            "name": defaults["name"],
            "description": defaults["description"],
            "body": defaults["body"]
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(restored["is_modified"], false);
    assert_eq!(restored["default_has_update"], false);

    let custom: Value = client
        .put(format!("{base}/background/skills/custom-update-check"))
        .bearer_auth("background-test-token")
        .header("If-None-Match", "*")
        .json(&json!({
            "name": "Custom update check",
            "description": "Custom skills have no built-in update state",
            "body": "# Custom\n\nNo default."
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(custom["is_builtin"], false);
    assert_eq!(custom["is_modified"], false);
    assert_eq!(custom["default_has_update"], false);

    server.abort();
}
