use super::harness::*;

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

#[tokio::test]
async fn recipe_job_http_projection_returns_the_recipe_progress_view() {
    let temp_root =
        std::env::temp_dir().join(format!("background-recipe-http-{}", uuid::Uuid::new_v4()));
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
endpoint = "http://127.0.0.1:9/v1"
api_key = "test-key"
model = "test-model"
store_dir = "{}"
tasks_store_dir = "{}"
reports_dir = "{}"
work_dir = "{}"
prompts_dir = "{}"
skills_dir = "{}"
[wiki]
vault_dir = "{}"
"#,
        temp_root.join("voice-agent.db").display(),
        temp_root.join("orch-store").display(),
        temp_root.join("tasks-store").display(),
        temp_root.join("reports").display(),
        temp_root.join("work").display(),
        temp_root.join("prompts").display(),
        temp_root.join("skills").display(),
        temp_root.join("wiki").display(),
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
        .post(format!("{base}/jobs"))
        .bearer_auth("background-test-token")
        .json(&json!({
            "recipe": {
                "id": "playlist_sources",
                "params": {"url":"https://www.youtube.com/playlist?list=PL1"}
            }
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(created.status(), StatusCode::CREATED);
    let created: Value = created.json().await.unwrap();
    let expected_progress = created["recipe"]["progress"].clone();
    assert_eq!(created["recipe"]["id"], "playlist_sources");
    assert_eq!(created["recipe"]["version"], 1);
    assert_eq!(
        created["recipe"]["params"]["url"],
        "https://www.youtube.com/playlist?list=PL1"
    );
    assert_eq!(expected_progress["current_step"], 0);

    let list: Value = client
        .get(format!("{base}/jobs"))
        .bearer_auth("background-test-token")
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let item = list["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["id"] == created["id"])
        .unwrap();
    assert_eq!(item["recipe"]["progress"], expected_progress);

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
    assert_eq!(
        item["download_url"],
        "/api/admin/jobs/job-99/artifacts/art-1"
    );
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
        active_session_id: "root-sess".into(),
        followup_session_id: None,
        latest_turn_id: Some("turn-1".into()),
        status: TaskStatus::Running,
        active_subagent: None,
        active_tool: None,
        progress_summary: None,
        output_preview: None,
        related_wiki_links: Vec::new(),
        error_code: None,
        error_message: None,
        recipe: None,
        wait_reason: None,
        next_attempt_at: None,
        created_at: 1_700_000_000_000,
        started_at: Some(1_700_000_000_000),
        updated_at: 1_700_000_000_000,
        finished_at: None,
    };

    let child_session = TaskAgentSessionView {
        session_id: "child-sess".into(),
        parent_session_id: Some("root-sess".into()),
        agent_id: "wiki_researcher".into(),
        run_kind: orchestrator_flow_rs::AgentRunKind::Free,
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
            retry_reason: None,
            next_attempt_at: None,
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
        max_resume_attempts: None,
        task: base_task.clone(),
        followups: Vec::new(),
        artifacts: Vec::new(),
        execution_summary: None,
        root_agent: Some(TaskAgentSessionView {
            session_id: "root-sess".into(),
            parent_session_id: None,
            agent_id: "daily_root".into(),
            run_kind: orchestrator_flow_rs::AgentRunKind::Free,
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
                retry_reason: None,
                next_attempt_at: None,
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
        followup_agents: Vec::new(),
        recipe: None,
        family_snapshot: None,
    };

    let events_1 = project_task_events(&detail_while_running, 0, 100);
    assert_eq!(events_1.len(), 8);

    // Snapshot 2: `subagent` completed and root called `markdown_create`
    let mut completed_task = base_task;
    completed_task.status = TaskStatus::Completed;
    let detail_after_root_continues = TaskDetailView {
        max_resume_attempts: None,
        task: completed_task,
        followups: Vec::new(),
        artifacts: Vec::new(),
        execution_summary: None,
        root_agent: Some(TaskAgentSessionView {
            session_id: "root-sess".into(),
            parent_session_id: None,
            agent_id: "daily_root".into(),
            run_kind: orchestrator_flow_rs::AgentRunKind::Free,
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
                retry_reason: None,
                next_attempt_at: None,
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
        followup_agents: Vec::new(),
        recipe: None,
        family_snapshot: None,
    };

    let events_2 = project_task_events(&detail_after_root_continues, 0, 100);
    assert_eq!(events_2.len(), 13);
    // The first 8 events must be 100% identical in sequence and content!
    assert_eq!(&events_2[..8], &events_1[..]);
}
