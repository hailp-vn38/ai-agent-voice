use reqwest::{Client, StatusCode};
use serde_json::{Value, json};
use std::sync::Arc;
use voice_agent_server::{
    app::bootstrap_with_providers, config::AppConfig, providers::ProviderSet,
};

#[tokio::test]
async fn background_http_foundation_is_authenticated_and_fails_closed_without_executor() {
    let temp_root = std::env::temp_dir().join(format!("background-http-{}", uuid::Uuid::new_v4()));
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
        temp_root.join("voice-agent.db").display(),
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
    assert_eq!(
        client
            .get(format!("{base}/background/settings"))
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::UNAUTHORIZED
    );
    let response = client
        .get(format!("{base}/background/settings"))
        .bearer_auth("background-test-token")
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let settings: Value = response.json().await.unwrap();
    assert_eq!(settings["enabled"], false);
    assert_eq!(settings["revision"], 1);
    let updated = client
        .put(format!("{base}/background/settings"))
        .bearer_auth("background-test-token")
        .header("If-Match", "\"1\"")
        .json(&json!({"enabled": true}))
        .send()
        .await
        .unwrap();
    assert_eq!(updated.status(), StatusCode::OK);
    let updated_json: Value = updated.json().await.unwrap();
    assert_eq!(updated_json["enabled"], true);
    assert_eq!(updated_json["revision"], 2);
    let fetched = client
        .get(format!("{base}/background/settings"))
        .bearer_auth("background-test-token")
        .send()
        .await
        .unwrap()
        .json::<Value>()
        .await
        .unwrap();
    assert_eq!(fetched["enabled"], true);
    assert_eq!(fetched["revision"], 2);
    let conflict = client
        .put(format!("{base}/background/settings"))
        .bearer_auth("background-test-token")
        .header("If-Match", "\"1\"")
        .json(&json!({"enabled": false}))
        .send()
        .await
        .unwrap();
    assert_eq!(conflict.status(), StatusCode::CONFLICT);
    assert_eq!(
        conflict.json::<Value>().await.unwrap()["error"]["code"],
        "revision_conflict"
    );
    let response = client
        .post(format!("{base}/jobs"))
        .bearer_auth("background-test-token")
        .json(&json!({"goal":"Collect public AI posts"}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(
        response.json::<Value>().await.unwrap()["error"]["code"],
        "runtime_unavailable"
    );

    // /api/admin/artifacts requires authentication and returns empty catalog when unbacked
    assert_eq!(
        client
            .get(format!("{base}/artifacts"))
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::UNAUTHORIZED
    );
    let artifacts_resp = client
        .get(format!("{base}/artifacts?page=1&page_size=24"))
        .bearer_auth("background-test-token")
        .send()
        .await
        .unwrap();
    assert_eq!(artifacts_resp.status(), StatusCode::OK);
    let artifacts_json: Value = artifacts_resp.json().await.unwrap();
    assert_eq!(artifacts_json["items"], json!([]));
    assert_eq!(artifacts_json["total"], 0);
    assert_eq!(artifacts_json["summary"]["total_artifacts"], 0);

    server.abort();
}

#[test]
fn project_artifact_catalog_page_maps_task_context_and_download_url() {
    use orchestrator_flow_rs::{
        TaskArtifactCatalogEntry, TaskArtifactCatalogPage, TaskArtifactCatalogSummary,
        TaskArtifactKind, TaskArtifactRole, TaskArtifactView, TaskStatus,
    };
    use voice_agent_server::services::task_flow::project_artifact_catalog_page;

    let entry = TaskArtifactCatalogEntry {
        artifact: TaskArtifactView {
            id: "art-1".into(),
            task_id: "job-99".into(),
            root_session_id: "sess-root".into(),
            producer_session_id: "sess-child".into(),
            producer_agent_id: "media_creator".into(),
            turn_id: "turn-1".into(),
            producer_invocation_id: Some("inv-1".into()),
            producer_tool_name: Some("image_create".into()),
            name: "topology.svg".into(),
            mime_type: "image/svg+xml".into(),
            kind: TaskArtifactKind::Image,
            role: TaskArtifactRole::Primary,
            size_bytes: 4096,
            sha256: "abc123def456".into(),
            created_at: 1_700_000_005_000,
        },
        task_title: "B200 Topology".into(),
        task_prompt: "Draw NVLink 5 topology".into(),
        task_status: TaskStatus::Completed,
        flow_id: "daily".into(),
        root_agent_id: "daily_root".into(),
        owner_id: "personal".into(),
        workspace_id: "homelab".into(),
    };

    let mut summary = TaskArtifactCatalogSummary::default();
    summary.record(&entry.artifact);

    let page = TaskArtifactCatalogPage {
        items: vec![entry],
        total: 1,
        summary,
    };

    let json_val = project_artifact_catalog_page(&page, 1, 24);
    assert_eq!(json_val["total"], 1);
    assert_eq!(json_val["summary"]["total_artifacts"], 1);
    assert_eq!(json_val["summary"]["image_artifacts"], 1);
    assert_eq!(json_val["summary"]["primary_artifacts"], 1);
    assert_eq!(json_val["summary"]["total_size_bytes"], 4096);

    let item = &json_val["items"][0];
    assert_eq!(item["id"], "art-1");
    assert_eq!(item["job_id"], "job-99");
    assert_eq!(item["job_title"], "B200 Topology");
    assert_eq!(item["job_status"], "succeeded");
    assert_eq!(item["kind"], "image");
    assert_eq!(item["role"], "primary");
    assert_eq!(item["mime_type"], "image/svg+xml");
    assert_eq!(item["download_url"], "/api/admin/jobs/job-99/artifacts/art-1");
}

#[tokio::test]
async fn voice_task_submit_rejects_task_delete_and_uses_shared_catalog_specs() {
    use std::time::Duration;
    use voice_agent_server::services::task_flow::VoiceTaskSubmit;

    let handle = VoiceTaskSubmit::unbacked_for_test();
    let defs = handle.tool_definitions();
    assert_eq!(defs.len(), 9);
    assert!(!defs.iter().any(|d| d.name == "task_delete"));

    let rejected = handle
        .execute(
            "task_delete",
            json!({ "task_id": "t-123" }),
            Duration::from_secs(1),
        )
        .await;
    assert_eq!(rejected["isError"], true);
    assert_eq!(rejected["error"]["code"], "invalid_arguments");
}

#[test]
fn project_task_events_preserves_subagent_sequence_numbers_when_root_continues() {
    use orchestrator_flow_rs::{
        TaskAgentRunView, TaskAgentSessionView, TaskDetailView, TaskSource, TaskStatus,
        TaskToolInvocationView, TaskView,
    };
    use voice_agent_server::services::task_flow::project_task_events;

    let base_task = TaskView {
        id: "task-1".into(),
        owner_id: "personal".into(),
        workspace_id: "homelab".into(),
        title: "Test".into(),
        prompt: "Test".into(),
        flow_id: "default".into(),
        agent_id: "daily_root".into(),
        source: TaskSource::Api,
        schedule_id: None,
        session_id: "root-sess".into(),
        latest_turn_id: Some("turn-1".into()),
        status: TaskStatus::Running,
        active_subagent: None,
        active_tool: None,
        progress_summary: None,
        output_preview: None,
        related_wiki_links: Vec::new(),
        error_code: None,
        error_message: None,
        created_at: 1_700_000_000_000,
        started_at: Some(1_700_000_000_000),
        updated_at: 1_700_000_000_000,
        finished_at: None,
    };

    let child_session = TaskAgentSessionView {
        session_id: "child-sess".into(),
        parent_session_id: Some("root-sess".into()),
        agent_id: "wiki_researcher".into(),
        is_root: false,
        archived: false,
        state_revision: 1,
        latest_run: Some(TaskAgentRunView {
            run_id: "run-child".into(),
            turn_id: "turn-child".into(),
            status: "completed".into(),
            attempt: 1,
            steps: 1,
            input_tokens: 10,
            output_tokens: 20,
            total_tokens: 30,
            error_code: None,
            error_message: None,
        }),
        invocations: vec![TaskToolInvocationView {
            invocation_id: "inv-child-1".into(),
            run_id: "run-child".into(),
            tool_call_id: "call-child-1".into(),
            tool_name: "wiki_search".into(),
            status: "completed".into(),
            error_code: None,
            arguments: json!({"query": "rust"}),
            result: Some(json!("ok")),
            child_session_id: None,
        }],
        recent_messages: Vec::new(),
    };

    // Snapshot 1: root is currently awaiting `subagent` tool call
    let detail_while_running = TaskDetailView {
        task: base_task.clone(),
        followups: Vec::new(),
        artifacts: Vec::new(),
        execution_summary: None,
        root_agent: Some(TaskAgentSessionView {
            session_id: "root-sess".into(),
            parent_session_id: None,
            agent_id: "daily_root".into(),
            is_root: true,
            archived: false,
            state_revision: 1,
            latest_run: Some(TaskAgentRunView {
                run_id: "run-root".into(),
                turn_id: "turn-1".into(),
                status: "running".into(),
                attempt: 1,
                steps: 1,
                input_tokens: 15,
                output_tokens: 10,
                total_tokens: 25,
                error_code: None,
                error_message: None,
            }),
            invocations: vec![TaskToolInvocationView {
                invocation_id: "inv-root-1".into(),
                run_id: "run-root".into(),
                tool_call_id: "call-root-1".into(),
                tool_name: "subagent".into(),
                status: "running".into(),
                error_code: None,
                arguments: json!({"agent": "wiki_researcher"}),
                result: None,
                child_session_id: Some("child-sess".into()),
            }],
            recent_messages: Vec::new(),
        }),
        subagents: vec![child_session.clone()],
        family_snapshot: None,
    };

    let events_1 = project_task_events(&detail_while_running, 0, 100);
    assert_eq!(events_1.len(), 8);

    // Snapshot 2: `subagent` completed and root called `markdown_create`
    let mut completed_task = base_task;
    completed_task.status = TaskStatus::Completed;
    let detail_after_root_continues = TaskDetailView {
        task: completed_task,
        followups: Vec::new(),
        artifacts: Vec::new(),
        execution_summary: None,
        root_agent: Some(TaskAgentSessionView {
            session_id: "root-sess".into(),
            parent_session_id: None,
            agent_id: "daily_root".into(),
            is_root: true,
            archived: false,
            state_revision: 2,
            latest_run: Some(TaskAgentRunView {
                run_id: "run-root".into(),
                turn_id: "turn-1".into(),
                status: "completed".into(),
                attempt: 1,
                steps: 2,
                input_tokens: 30,
                output_tokens: 25,
                total_tokens: 55,
                error_code: None,
                error_message: None,
            }),
            invocations: vec![
                TaskToolInvocationView {
                    invocation_id: "inv-root-1".into(),
                    run_id: "run-root".into(),
                    tool_call_id: "call-root-1".into(),
                    tool_name: "subagent".into(),
                    status: "completed".into(),
                    error_code: None,
                    arguments: json!({"agent": "wiki_researcher"}),
                    result: Some(json!("done")),
                    child_session_id: Some("child-sess".into()),
                },
                TaskToolInvocationView {
                    invocation_id: "inv-root-2".into(),
                    run_id: "run-root".into(),
                    tool_call_id: "call-root-2".into(),
                    tool_name: "markdown_create".into(),
                    status: "completed".into(),
                    error_code: None,
                    arguments: json!({"filename": "report.md"}),
                    result: Some(json!("created")),
                    child_session_id: None,
                },
            ],
            recent_messages: Vec::new(),
        }),
        subagents: vec![child_session],
        family_snapshot: None,
    };

    let events_2 = project_task_events(&detail_after_root_continues, 0, 100);
    assert_eq!(events_2.len(), 13);
    // The first 8 events must be 100% identical in sequence and content!
    assert_eq!(&events_2[..8], &events_1[..]);
}

#[tokio::test]
async fn background_settings_integrates_llm_providers_and_configures_subagents() {
    let temp_root = std::env::temp_dir().join(format!("background-agents-{}", uuid::Uuid::new_v4()));
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
        temp_root.join("voice-agent.db").display(),
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

    // 2. Fetch GET /api/admin/background/settings and verify provider catalog + 9 agents
    let initial: Value = client
        .get(format!("{base}/background/settings"))
        .bearer_auth("background-test-token")
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
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

    let agents = initial["agents"].as_array().unwrap();
    assert_eq!(agents.len(), 9);
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
                    "enabled": false,
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
    let media_creator = updated_agents
        .iter()
        .find(|a| a["id"] == "media_creator")
        .unwrap();
    assert_eq!(
        media_creator["tool_capability_tags"]["tts_synthesize"],
        "tts"
    );
    assert_eq!(
        media_creator["tool_providers"]["tts_synthesize"],
        db_tts_key
    );
    assert_eq!(
        media_creator["effective_tool_providers"]["tts_synthesize"],
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
    let temp_root = std::env::temp_dir().join(format!("background-skills-{}", uuid::Uuid::new_v4()));
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
        temp_root.join("voice-agent.db").display(),
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

    // 1. GET /background/settings includes 6 built-in skills and dynamic subagent available_skills
    let initial: Value = client
        .get(format!("{base}/background/settings"))
        .bearer_auth("background-test-token")
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let skills = initial["skills"].as_array().expect("skills array in settings");
    assert_eq!(skills.len(), 6);
    for expected_id in [
        "daily-synthesis-staging",
        "daily-task-reminder",
        "wiki-ingest",
        "wiki-lint",
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
    let researcher = agents.iter().find(|a| a["id"] == "wiki_researcher").unwrap();
    let researcher_avail = researcher["available_skills"].as_array().unwrap();
    assert_eq!(researcher_avail.len(), 6);
    assert_eq!(researcher_avail[0], "wiki-query");
    assert_eq!(researcher_avail[1], "wiki-taxonomy");

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
    let default_desc = detail["default"]["description"].as_str().unwrap().to_owned();
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
async fn background_custom_subagents_crud_flow_isolation_and_concurrency() {
    let temp_root =
        std::env::temp_dir().join(format!("background-subagents-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&temp_root).unwrap();
    let prompts_dir = temp_root.join("prompts");
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
        temp_root.join("voice-agent.db").display(),
        temp_root.join("orch-store").display(),
        prompts_dir.display(),
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

    // 1. Initial settings has 9 built-in agents with `is_builtin: true`
    let initial: Value = client
        .get(format!("{base}/background/settings"))
        .bearer_auth("background-test-token")
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(initial["revision"], 1);
    let initial_agents = initial["agents"].as_array().unwrap();
    assert_eq!(initial_agents.len(), 9);
    assert!(initial_agents.iter().all(|a| a["is_builtin"] == true));

    // 2. Create a Custom Sub-Agent in Flow 1 (`daily`) via POST /api/admin/background/agents
    let create_resp = client
        .post(format!("{base}/background/agents"))
        .bearer_auth("background-test-token")
        .header("If-Match", "\"1\"")
        .json(&json!({
            "id": "custom_arxiv_digest",
            "flow_id": "daily",
            "enabled": true,
            "steps": 18,
            "allowed_tools": ["fetch_html", "markdown_create"],
            "allowed_skills": ["wiki-query"],
            "description": "Summarizes arXiv papers into Markdown briefs.",
            "system_prompt": "You are the custom arXiv digest subagent."
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(create_resp.status(), StatusCode::CREATED);
    let after_create: Value = create_resp.json().await.unwrap();
    assert_eq!(after_create["revision"], 2);
    let agents = after_create["agents"].as_array().unwrap();
    assert_eq!(agents.len(), 10);
    let custom = agents
        .iter()
        .find(|a| a["id"] == "custom_arxiv_digest")
        .expect("custom subagent present");
    assert_eq!(custom["is_builtin"], false);
    assert_eq!(custom["flow_id"], "daily");
    assert_eq!(custom["parent_id"], "daily_root");
    assert_eq!(custom["enabled"], true);
    assert_eq!(custom["steps"], 18);
    assert_eq!(custom["default_steps"], 18);
    assert_eq!(
        custom["allowed_tools"],
        json!(["skill", "fetch_html", "markdown_create"])
    );
    assert_eq!(custom["allowed_skills"], json!(["wiki-query"]));
    assert_eq!(custom["default_allowed_skills"], json!(["wiki-query"]));

    let daily_root = agents.iter().find(|a| a["id"] == "daily_root").unwrap();
    assert!(
        daily_root["allowed_subagents"]
            .as_array()
            .unwrap()
            .contains(&json!("custom_arxiv_digest"))
    );
    assert!(prompts_dir.join("custom_arxiv_digest.md").exists());
    let daily_root_md = std::fs::read_to_string(prompts_dir.join("daily_root.md")).unwrap();
    assert!(daily_root_md.contains("`custom_arxiv_digest` (custom)"));

    // 3. Reject duplicate ID, built-in ID collision, invalid flow, or Flow 2 tool violation
    let dup_resp = client
        .post(format!("{base}/background/agents"))
        .bearer_auth("background-test-token")
        .header("If-Match", "\"2\"")
        .json(&json!({
            "id": "custom_arxiv_digest",
            "flow_id": "daily",
            "description": "Dup",
            "system_prompt": "Dup"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(dup_resp.status(), StatusCode::CONFLICT);
    assert_eq!(
        dup_resp.json::<Value>().await.unwrap()["error"]["code"],
        "agent_already_exists"
    );

    let builtin_dup = client
        .post(format!("{base}/background/agents"))
        .bearer_auth("background-test-token")
        .header("If-Match", "\"2\"")
        .json(&json!({
            "id": "source_collector",
            "flow_id": "daily",
            "description": "Dup builtin",
            "system_prompt": "Dup"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(builtin_dup.status(), StatusCode::BAD_REQUEST);
    assert_eq!(
        builtin_dup.json::<Value>().await.unwrap()["error"]["code"],
        "invalid_agent_id"
    );

    let bad_wiki_tool = client
        .post(format!("{base}/background/agents"))
        .bearer_auth("background-test-token")
        .header("If-Match", "\"2\"")
        .json(&json!({
            "id": "wiki_bad_collector",
            "flow_id": "wiki",
            "allowed_tools": ["yt_fetch_transcript"],
            "description": "Tries to use YouTube tool in Flow 2",
            "system_prompt": "Invalid tool for Flow 2"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(bad_wiki_tool.status(), StatusCode::BAD_REQUEST);
    assert_eq!(
        bad_wiki_tool.json::<Value>().await.unwrap()["error"]["code"],
        "invalid_arguments"
    );

    // 4. Partial toggle via PUT /api/admin/background/settings preserves flow_id & prompt
    let toggle_resp = client
        .put(format!("{base}/background/settings"))
        .bearer_auth("background-test-token")
        .header("If-Match", "\"2\"")
        .json(&json!({
            "enabled": true,
            "agents": {
                "custom_arxiv_digest": {
                    "enabled": false
                }
            }
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(toggle_resp.status(), StatusCode::OK);
    let toggled: Value = toggle_resp.json().await.unwrap();
    assert_eq!(toggled["revision"], 3);
    let toggled_agents = toggled["agents"].as_array().unwrap();
    let toggled_custom = toggled_agents
        .iter()
        .find(|a| a["id"] == "custom_arxiv_digest")
        .unwrap();
    assert_eq!(toggled_custom["enabled"], false);
    assert_eq!(
        toggled_custom["description"],
        "Summarizes arXiv papers into Markdown briefs."
    );
    let toggled_daily_root = toggled_agents
        .iter()
        .find(|a| a["id"] == "daily_root")
        .unwrap();
    assert!(
        !toggled_daily_root["allowed_subagents"]
            .as_array()
            .unwrap()
            .contains(&json!("custom_arxiv_digest"))
    );

    // 5. Deleting a Built-in Sub-Agent is rejected with 400 builtin_agent_cannot_be_deleted
    let del_builtin = client
        .delete(format!("{base}/background/agents/source_collector"))
        .bearer_auth("background-test-token")
        .header("If-Match", "\"3\"")
        .send()
        .await
        .unwrap();
    assert_eq!(del_builtin.status(), StatusCode::BAD_REQUEST);
    assert_eq!(
        del_builtin.json::<Value>().await.unwrap()["error"]["code"],
        "builtin_agent_cannot_be_deleted"
    );

    // 6. Delete Custom Sub-Agent with If-Match -> 200 OK, removes agent & prompt file
    let del_custom = client
        .delete(format!("{base}/background/agents/custom_arxiv_digest"))
        .bearer_auth("background-test-token")
        .header("If-Match", "\"3\"")
        .send()
        .await
        .unwrap();
    assert_eq!(del_custom.status(), StatusCode::OK);
    let after_del: Value = del_custom.json().await.unwrap();
    assert_eq!(after_del["revision"], 4);
    assert_eq!(after_del["agents"].as_array().unwrap().len(), 9);
    assert!(!prompts_dir.join("custom_arxiv_digest.md").exists());

    server.abort();
}

