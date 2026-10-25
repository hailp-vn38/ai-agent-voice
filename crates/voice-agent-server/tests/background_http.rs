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
    server.abort();
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
                    "enabled": false
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

