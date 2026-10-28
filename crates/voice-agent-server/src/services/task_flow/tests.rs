use super::*;

#[test]
fn voice_task_submit_advertises_exact_nine_task_suite_tools_and_matching_reminder_schemas() {
    let submit = VoiceTaskSubmit::unbacked_for_test();
    let defs = submit.tool_definitions();
    let names: Vec<&str> = defs.iter().map(|d| d.name.as_str()).collect();
    assert_eq!(names, TASK_SUITE_TOOL_NAMES);
    assert!(!names.contains(&"task_delete"));

    let reminder_create = defs
        .iter()
        .find(|d| d.name == "reminder_create")
        .expect("reminder_create definition");
    let props = reminder_create.parameters["properties"]
        .as_object()
        .expect("reminder_create properties");
    assert!(props.contains_key("due_at"));
    assert!(props.contains_key("delay_seconds"));
    assert!(props.contains_key("repeat_cron"));
    assert!(props.contains_key("action_kind"));
    assert!(props.contains_key("notes"));

    for tool_name in ["reminder_complete", "reminder_delete"] {
        let def = defs
            .iter()
            .find(|d| d.name == tool_name)
            .expect("reminder mutation definition");
        assert_eq!(
            def.parameters["required"],
            serde_json::json!(["reminder_id"])
        );
    }
}

#[test]
fn project_task_to_admin_job_includes_multi_kind_artifacts_and_download_urls() {
    use orchestrator_flow_rs::{
        TaskArtifactKind, TaskArtifactRole, TaskArtifactView, TaskDetailView, TaskExecutionSummary,
    };

    let task = TaskView {
        id: "task-multi-1".to_string(),
        owner_id: "personal".to_string(),
        workspace_id: "homelab".to_string(),
        title: "Multi-modal Briefing".to_string(),
        prompt: "Create report, diagram, and TTS".to_string(),
        flow_id: "default".to_string(),
        agent_id: "daily_root".to_string(),
        source: TaskSource::Api,
        schedule_id: None,
        session_id: "sess-root-1".to_string(),
        latest_turn_id: Some("turn-1".to_string()),
        status: orchestrator_flow_rs::TaskStatus::Completed,
        active_subagent: None,
        active_tool: None,
        progress_summary: Some("Done".to_string()),
        output_preview: Some("Summary".to_string()),
        related_wiki_links: Vec::new(),
        error_code: None,
        error_message: None,
        created_at: 1_760_000_000_000,
        started_at: Some(1_760_000_001_000),
        updated_at: 1_760_000_005_000,
        finished_at: Some(1_760_000_005_000),
    };

    let artifacts = vec![
        TaskArtifactView {
            id: "art-md-1".to_string(),
            task_id: task.id.clone(),
            root_session_id: task.session_id.clone(),
            producer_session_id: "sess-sub-1".to_string(),
            producer_agent_id: "media_creator".to_string(),
            turn_id: "turn-sub-1".to_string(),
            producer_invocation_id: Some("inv-1".to_string()),
            producer_tool_name: Some("markdown_create".to_string()),
            name: "report.md".to_string(),
            mime_type: "text/markdown; charset=utf-8".to_string(),
            kind: TaskArtifactKind::Report,
            role: TaskArtifactRole::Primary,
            size_bytes: 512,
            sha256: "abc123".to_string(),
            created_at: 1_760_000_002_000,
        },
        TaskArtifactView {
            id: "art-png-2".to_string(),
            task_id: task.id.clone(),
            root_session_id: task.session_id.clone(),
            producer_session_id: "sess-sub-1".to_string(),
            producer_agent_id: "media_creator".to_string(),
            turn_id: "turn-sub-1".to_string(),
            producer_invocation_id: Some("inv-2".to_string()),
            producer_tool_name: Some("image_create".to_string()),
            name: "diagram.png".to_string(),
            mime_type: "image/png".to_string(),
            kind: TaskArtifactKind::Image,
            role: TaskArtifactRole::Primary,
            size_bytes: 2048,
            sha256: "def456".to_string(),
            created_at: 1_760_000_003_000,
        },
        TaskArtifactView {
            id: "art-wav-3".to_string(),
            task_id: task.id.clone(),
            root_session_id: task.session_id.clone(),
            producer_session_id: "sess-sub-1".to_string(),
            producer_agent_id: "media_creator".to_string(),
            turn_id: "turn-sub-1".to_string(),
            producer_invocation_id: Some("inv-3".to_string()),
            producer_tool_name: Some("tts_synthesize".to_string()),
            name: "voiceover.wav".to_string(),
            mime_type: "audio/wav".to_string(),
            kind: TaskArtifactKind::Audio,
            role: TaskArtifactRole::Primary,
            size_bytes: 8192,
            sha256: "789012".to_string(),
            created_at: 1_760_000_004_000,
        },
    ];

    let mut summary = TaskExecutionSummary::default();
    summary.apply_artifacts(&artifacts);

    let detail = TaskDetailView {
        task: task.clone(),
        followups: Vec::new(),
        artifacts,
        execution_summary: Some(summary),
        root_agent: None,
        subagents: Vec::new(),
        family_snapshot: None,
    };

    let projected = project_task_to_admin_job(&task, Some(&detail));
    let arr = projected["artifacts"].as_array().expect("artifacts array");
    assert_eq!(arr.len(), 3);
    assert_eq!(arr[0]["kind"], "report");
    assert_eq!(
        arr[0]["download_url"],
        "/api/admin/jobs/task-multi-1/artifacts/art-md-1"
    );
    assert_eq!(arr[1]["kind"], "image");
    assert_eq!(arr[1]["mime_type"], "image/png");
    assert_eq!(arr[2]["kind"], "audio");
    assert_eq!(arr[2]["mime_type"], "audio/wav");
    assert_eq!(projected["budget_used"]["total_artifacts"], 3);
    assert_eq!(projected["budget_used"]["report_artifacts"], 1);
    assert_eq!(projected["budget_used"]["image_artifacts"], 1);
    assert_eq!(projected["budget_used"]["audio_artifacts"], 1);
}

#[test]
fn dual_flow_agent_specs_assign_collectors_to_daily_flow_and_keep_wiki_flow_pure() {
    let daily_root = find_agent_spec("daily_root").expect("daily_root spec");
    assert_eq!(daily_root.flow_id, "daily");
    assert!(daily_root.user_routable);
    assert!(is_user_routable_root_agent("daily_root"));
    assert_eq!(
        daily_root.default_subagents,
        &[
            "source_collector",
            "wiki_researcher",
            "report_stager",
            "media_creator",
            "reminder_planner",
        ]
    );

    let collector = find_agent_spec("source_collector").expect("source_collector spec");
    assert_eq!(collector.flow_id, "daily");
    assert_eq!(collector.parent_id, Some("daily_root"));
    for &tool in orchestrator_flow_rs::COLLECTOR_TOOL_NAMES {
        assert!(
            collector.default_allowed_tools.contains(&tool),
            "source_collector in Flow 1 (daily) must have tool '{tool}'"
        );
    }
    assert!(collector.default_allowed_tools.contains(&"add_to_sources"));

    let report_stager = find_agent_spec("report_stager").expect("report_stager spec");
    assert!(report_stager.default_allowed_tools.contains(&"add_to_sources"));
    assert!(report_stager.default_allowed_tools.contains(&"markdown_create"));
    assert!(report_stager.default_allowed_tools.contains(&"chart_write"));
    assert!(report_stager.default_allowed_tools.contains(&"chart_get"));
    assert!(report_stager.default_allowed_skills.contains(&"trading-chart"));
    assert!(
        !report_stager.default_allowed_tools.contains(&"image_create")
            && !report_stager.default_allowed_tools.contains(&"tts_synthesize"),
        "report_stager must not duplicate media_creator tools"
    );

    for wiki_agent_id in ["wiki_ingestor", "wiki_curator"] {
        let spec = find_agent_spec(wiki_agent_id).expect("wiki subagent spec");
        assert!(spec.default_allowed_tools.contains(&"chart_write"));
        assert!(spec.default_allowed_tools.contains(&"chart_get"));
        assert!(spec.default_allowed_skills.contains(&"trading-chart"));
    }
    assert!(BUILTIN_SKILLS.iter().any(|(id, _)| *id == "trading-chart"));

    let media_creator = find_agent_spec("media_creator").expect("media_creator spec");
    assert!(media_creator.default_allowed_tools.contains(&"image_create"));
    assert!(media_creator.default_allowed_tools.contains(&"tts_synthesize"));
    assert!(media_creator.default_allowed_tools.contains(&"artifact_create"));
    assert!(
        !media_creator.default_allowed_tools.contains(&"markdown_create"),
        "media_creator must not duplicate report_stager markdown_create tool"
    );

    let wiki_root = find_agent_spec("wiki_root").expect("wiki_root spec");
    assert_eq!(wiki_root.flow_id, "wiki");
    assert!(!wiki_root.user_routable);
    assert!(!is_user_routable_root_agent("wiki_root"));
    assert_eq!(normalize_user_target_agent("wiki_root"), "daily_root");
    assert_eq!(normalize_user_target_agent("default/wiki_root"), "daily_root");
    assert_eq!(normalize_user_target_agent("daily_root"), "daily_root");
    assert_eq!(
        wiki_root.default_subagents,
        &["wiki_ingestor", "wiki_curator"]
    );

    for spec in AGENT_SPECS.iter().filter(|s| s.flow_id == "wiki") {
        for &forbidden in orchestrator_flow_rs::COLLECTOR_TOOL_NAMES {
            assert!(
                !spec.available_tools.contains(&forbidden)
                    && !spec.default_allowed_tools.contains(&forbidden),
                "Flow 2 wiki agent '{}' must not have external collector tool '{forbidden}'",
                spec.id
            );
        }
    }

    let submit = VoiceTaskSubmit::unbacked_for_test();
    let defs = submit.tool_definitions();
    let task_create = defs
        .iter()
        .find(|d| d.name == "task_create")
        .expect("task_create definition");
    assert!(task_create.description.contains("daily_root"));
    assert!(
        !task_create.description.contains("wiki_root"),
        "Voice task_create must route user tasks to Flow 1 (daily_root) without exposing internal Flow 2 (wiki_root)"
    );
}

#[test]
fn project_task_to_admin_job_and_events_include_agent_messages_for_tool_calls() {
    use orchestrator_flow_rs::{
        TaskAgentMessageView, TaskAgentRunView, TaskAgentSessionView, TaskDetailView,
        TaskMessageToolCallView, TaskToolInvocationView,
    };

    let task = TaskView {
        id: "task-msg-1".to_string(),
        owner_id: "personal".to_string(),
        workspace_id: "homelab".to_string(),
        title: "Research AI".to_string(),
        prompt: "Search wiki for Rust".to_string(),
        flow_id: "daily".to_string(),
        agent_id: "daily_root".to_string(),
        source: TaskSource::Api,
        schedule_id: None,
        session_id: "sess-root-1".to_string(),
        latest_turn_id: Some("turn-1".to_string()),
        status: orchestrator_flow_rs::TaskStatus::Completed,
        active_subagent: None,
        active_tool: None,
        progress_summary: Some("Đã tìm xong tài liệu Rust.".to_string()),
        output_preview: Some("Đã tìm xong tài liệu Rust.".to_string()),
        related_wiki_links: Vec::new(),
        error_code: None,
        error_message: None,
        created_at: 1_760_000_000_000,
        started_at: Some(1_760_000_001_000),
        updated_at: 1_760_000_005_000,
        finished_at: Some(1_760_000_005_000),
    };

    let root_session = TaskAgentSessionView {
        session_id: "sess-root-1".to_string(),
        parent_session_id: None,
        agent_id: "daily_root".to_string(),
        is_root: true,
        archived: false,
        state_revision: 3,
        latest_run: Some(TaskAgentRunView {
            run_id: "run-1".to_string(),
            turn_id: "turn-1".to_string(),
            attempt: 1,
            status: "completed".to_string(),
            steps: 2,
            input_tokens: 100,
            output_tokens: 50,
            total_tokens: 150,
            error_code: None,
            error_message: None,
        }),
        invocations: vec![TaskToolInvocationView {
            invocation_id: "inv-1".to_string(),
            run_id: "run-1".to_string(),
            tool_call_id: "call-wiki-1".to_string(),
            tool_name: "wiki_search".to_string(),
            arguments: serde_json::json!({ "query": "Rust" }),
            status: "completed".to_string(),
            result: Some(serde_json::json!({ "hits": 3 })),
            child_session_id: None,
            error_code: None,
        }],
        recent_messages: vec![
            TaskAgentMessageView {
                id: "msg-user-1".to_string(),
                sequence: 1,
                role: "user".to_string(),
                text: "Search wiki for Rust".to_string(),
                run_id: Some("run-1".to_string()),
                tool_call_id: None,
                tool_calls: Vec::new(),
            },
            TaskAgentMessageView {
                id: "msg-asst-1".to_string(),
                sequence: 2,
                role: "assistant".to_string(),
                text: "Tôi sẽ tra cứu Wiki về Rust trước.".to_string(),
                run_id: Some("run-1".to_string()),
                tool_call_id: None,
                tool_calls: vec![TaskMessageToolCallView {
                    call_id: "call-wiki-1".to_string(),
                    name: "wiki_search".to_string(),
                    arguments: serde_json::json!({ "query": "Rust" }),
                }],
            },
            TaskAgentMessageView {
                id: "msg-asst-2".to_string(),
                sequence: 4,
                role: "assistant".to_string(),
                text: "Đã tìm xong tài liệu Rust.".to_string(),
                run_id: Some("run-1".to_string()),
                tool_call_id: None,
                tool_calls: Vec::new(),
            },
        ],
    };

    let detail = TaskDetailView {
        task: task.clone(),
        followups: Vec::new(),
        artifacts: vec![orchestrator_flow_rs::TaskArtifactView {
            id: "art-1".to_string(),
            task_id: "task-msg-1".to_string(),
            root_session_id: "sess-root-1".to_string(),
            producer_session_id: "sess-root-1".to_string(),
            producer_agent_id: "daily_root".to_string(),
            turn_id: "turn-1".to_string(),
            producer_invocation_id: Some("inv-1".to_string()),
            producer_tool_name: Some("wiki_search".to_string()),
            name: "rust_summary.md".to_string(),
            mime_type: "text/markdown".to_string(),
            kind: orchestrator_flow_rs::TaskArtifactKind::Report,
            role: orchestrator_flow_rs::TaskArtifactRole::Primary,
            size_bytes: 2048,
            sha256: "0".repeat(64),
            created_at: 1_760_000_004_000,
        }],
        execution_summary: None,
        root_agent: Some(root_session),
        subagents: Vec::new(),
        family_snapshot: None,
    };

    let projected = project_task_to_admin_job(&task, Some(&detail));
    let artifact = &projected["artifacts"][0];
    assert_eq!(artifact["filename"], "rust_summary.md");
    assert_eq!(artifact["media_type"], "text/markdown");
    assert_eq!(artifact["step_key"], "daily_root:1:wiki_search");
    let agent_run = &projected["agent_runs"][0];
    let msgs = agent_run["messages"].as_array().expect("agent_run.messages array");
    assert_eq!(msgs.len(), 3);
    assert_eq!(msgs[1]["role"], "assistant");
    assert_eq!(msgs[1]["text"], "Tôi sẽ tra cứu Wiki về Rust trước.");
    assert_eq!(msgs[1]["tool_call_ids"], serde_json::json!(["call-wiki-1"]));

    let step = &projected["steps"][0];
    assert_eq!(
        step["assistant_message"],
        "Tôi sẽ tra cứu Wiki về Rust trước."
    );

    let events = project_task_events(&detail, 0, 100);
    let step_started = events
        .iter()
        .find(|e| e["event_type"] == "step_started")
        .expect("step_started event");
    assert_eq!(
        step_started["payload"]["assistant_message"],
        "Tôi sẽ tra cứu Wiki về Rust trước."
    );
    let run_finished = events
        .iter()
        .find(|e| e["event_type"] == "agent_run_finished")
        .expect("agent_run_finished event");
    assert_eq!(
        run_finished["payload"]["assistant_message"],
        "Đã tìm xong tài liệu Rust."
    );

    // Khi run còn đang `running`, `agent_run.result` phải là `null` (docs/17 §17.6),
    // nhưng `messages` và `step.assistant_message` vẫn phản ánh thông điệp gọi tool.
    let mut running_detail = detail;
    if let Some(ref mut root) = running_detail.root_agent {
        root.latest_run.as_mut().unwrap().status = "running".to_string();
    }
    let running_projected = project_task_to_admin_job(&task, Some(&running_detail));
    assert!(running_projected["agent_runs"][0]["result"].is_null());
    assert_eq!(
        running_projected["steps"][0]["assistant_message"],
        "Tôi sẽ tra cứu Wiki về Rust trước."
    );
}

#[test]
fn resolved_llm_provider_exposes_capability_tags_and_enforces_agent_compatibility() {
    use crate::services::task_flow::agent_settings::{
        AgentConfigOverride, ResolvedLlmProvider, find_provider_by_tag, validate_agent_overrides,
    };
    use std::collections::BTreeMap;

    let chat_provider = ResolvedLlmProvider {
        provider_ref: "llm_chat".into(),
        name: "GPT-4o".into(),
        source: "database",
        adapter: "openai".into(),
        model: "gpt-4o".into(),
        base_url: "https://api.example.com/v1".into(),
        api_key: None,
        gateway_binding: None,
        timeout_ms: 30_000,
        max_tokens: 4_096,
        context_limit_tokens: 128_000,
        reasoning_effort: None,
        temperature_percent: None,
        tags: vec!["chat".into(), "vision".into()],
        muse_options: None,
    };
    let image_only_provider = ResolvedLlmProvider {
        provider_ref: "llm_image".into(),
        name: "Flux Image".into(),
        source: "database",
        adapter: "openai".into(),
        model: "flux-1".into(),
        base_url: "https://api.example.com/v1".into(),
        api_key: None,
        gateway_binding: None,
        timeout_ms: 30_000,
        max_tokens: 4_096,
        context_limit_tokens: 32_000,
        reasoning_effort: None,
        temperature_percent: None,
        tags: vec!["image".into()],
        muse_options: None,
    };

    let summary = chat_provider.to_json_summary();
    assert_eq!(summary["tags"], serde_json::json!(["chat", "vision"]));

    let catalog = vec![chat_provider, image_only_provider];
    assert_eq!(
        find_provider_by_tag(&catalog, "image").map(|p| p.provider_ref.as_str()),
        Some("llm_image")
    );
    assert_eq!(
        find_provider_by_tag(&catalog, "vision").map(|p| p.provider_ref.as_str()),
        Some("llm_chat")
    );
    assert!(find_provider_by_tag(&catalog, "video").is_none());

    // Orchestrator provider must support `chat`
    assert_eq!(
        validate_agent_overrides("llm_image", "llm_chat", &BTreeMap::new(), &catalog),
        Err("provider_capability_mismatch")
    );

    // Media creator can use an `image`- or `video`-tagged provider, whereas `wiki_researcher` requires `chat`
    let video_only_provider = ResolvedLlmProvider {
        provider_ref: "llm_video".into(),
        name: "Sora Video".into(),
        source: "database",
        adapter: "openai".into(),
        model: "sora-1".into(),
        base_url: "https://api.example.com/v1".into(),
        api_key: None,
        gateway_binding: None,
        timeout_ms: 60_000,
        max_tokens: 4_096,
        context_limit_tokens: 32_000,
        reasoning_effort: None,
        temperature_percent: None,
        tags: vec!["video".into()],
        muse_options: None,
    };
    let audio_only_provider = ResolvedLlmProvider {
        provider_ref: "llm_audio".into(),
        name: "TTS Model".into(),
        source: "database",
        adapter: "openai".into(),
        model: "tts-1".into(),
        base_url: "https://api.example.com/v1".into(),
        api_key: None,
        gateway_binding: None,
        timeout_ms: 30_000,
        max_tokens: 4_096,
        context_limit_tokens: 32_000,
        reasoning_effort: None,
        temperature_percent: None,
        tags: vec!["audio".into()],
        muse_options: None,
    };
    let mut catalog = catalog;
    catalog.push(video_only_provider);
    catalog.push(audio_only_provider);

    let mut media_overrides = BTreeMap::new();
    media_overrides.insert(
        "media_creator".to_owned(),
        AgentConfigOverride {
            enabled: true,
            provider_ref: Some("llm_image".to_owned()),
            ..Default::default()
        },
    );
    assert!(validate_agent_overrides("llm_chat", "llm_chat", &media_overrides, &catalog).is_ok());

    media_overrides.insert(
        "media_creator".to_owned(),
        AgentConfigOverride {
            enabled: true,
            provider_ref: Some("llm_video".to_owned()),
            ..Default::default()
        },
    );
    assert!(validate_agent_overrides("llm_chat", "llm_chat", &media_overrides, &catalog).is_ok());

    // `media_creator` LLM provider only serves chat/reasoning/image/video; TTS uses `tts_synthesize` tool
    let mut audio_media_overrides = BTreeMap::new();
    audio_media_overrides.insert(
        "media_creator".to_owned(),
        AgentConfigOverride {
            enabled: true,
            provider_ref: Some("llm_audio".to_owned()),
            ..Default::default()
        },
    );
    assert_eq!(
        validate_agent_overrides("llm_chat", "llm_chat", &audio_media_overrides, &catalog),
        Err("provider_capability_mismatch")
    );

    // When `media_creator` is assigned a dedicated `video` (or `image`) provider without `chat`,
    // `project_agents_json` reports `llm_video`, while `build_agent_definitions` binds the
    // Orchestrator subagent's tool-calling session to the worker `chat` provider (`llm_chat`).
    let settings = crate::services::task_flow::PersistedBackgroundSettings {
        orchestrator_llm_provider: "llm_chat".into(),
        worker_llm_provider: "llm_chat".into(),
        agents: media_overrides,
        ..Default::default()
    };
    let projected_agents = crate::services::task_flow::project_agents_json(
        &crate::config::OrchestratorSectionConfig::default(),
        &settings,
        &catalog,
    );
    let media_json = projected_agents
        .iter()
        .find(|a| a["id"] == "media_creator")
        .unwrap();
    assert_eq!(media_json["effective_provider_ref"], "llm_video");
    assert_eq!(media_json["effective_model"], "sora-1");
    assert_eq!(
        media_json["allowed_capability_tags"],
        serde_json::json!(["chat", "reasoning", "image", "video"])
    );

    let (_, defs) = crate::services::task_flow::build_agent_definitions(
        &crate::config::OrchestratorSectionConfig::default(),
        &settings,
        &catalog,
    );
    let media_def = defs.iter().find(|d| d.id == "media_creator").unwrap();
    assert_eq!(
        media_def.provider_id, "llm_chat",
        "non-chat media provider must fall back to worker chat provider for Orchestrator tool-calling loop"
    );

    let mut wiki_overrides = BTreeMap::new();
    wiki_overrides.insert(
        "wiki_researcher".to_owned(),
        AgentConfigOverride {
            enabled: true,
            provider_ref: Some("llm_image".to_owned()),
            ..Default::default()
        },
    );
    assert_eq!(
        validate_agent_overrides("llm_chat", "llm_chat", &wiki_overrides, &catalog),
        Err("provider_capability_mismatch")
    );

    // Per-tool provider configuration (`tool_providers`) on `media_creator`:
    // allows binding `media_creator` brain to `llm_chat` while binding `image_create` -> `llm_image`,
    // `video_create` -> `llm_video`, and `tts_synthesize` -> `llm_audio`.
    let mut per_tool_overrides = BTreeMap::new();
    per_tool_overrides.insert(
        "media_creator".to_owned(),
        AgentConfigOverride {
            enabled: true,
            provider_ref: Some("llm_chat".to_owned()),
            tool_providers: Some(BTreeMap::from([
                ("image_create".to_owned(), "llm_image".to_owned()),
                ("video_create".to_owned(), "llm_video".to_owned()),
                ("tts_synthesize".to_owned(), "llm_audio".to_owned()),
            ])),
            ..Default::default()
        },
    );
    assert!(validate_agent_overrides("llm_chat", "llm_chat", &per_tool_overrides, &catalog).is_ok());

    let per_tool_settings = crate::services::task_flow::PersistedBackgroundSettings {
        orchestrator_llm_provider: "llm_chat".into(),
        worker_llm_provider: "llm_chat".into(),
        agents: per_tool_overrides,
        ..Default::default()
    };
    let projected_per_tool = crate::services::task_flow::project_agents_json(
        &crate::config::OrchestratorSectionConfig::default(),
        &per_tool_settings,
        &catalog,
    );
    let media_per_tool_json = projected_per_tool
        .iter()
        .find(|a| a["id"] == "media_creator")
        .unwrap();
    assert_eq!(
        media_per_tool_json["tool_capability_tags"],
        serde_json::json!({
            "image_create": "image",
            "video_create": "video",
            "tts_synthesize": "tts"
        })
    );
    assert_eq!(
        media_per_tool_json["tool_providers"],
        serde_json::json!({
            "image_create": "llm_image",
            "video_create": "llm_video",
            "tts_synthesize": "llm_audio"
        })
    );
    assert_eq!(
        media_per_tool_json["effective_tool_providers"],
        serde_json::json!({
            "image_create": "llm_image",
            "video_create": "llm_video",
            "tts_synthesize": "llm_audio"
        })
    );

    // Assigning a non-image provider (`llm_video`) to `image_create` must fail with `provider_capability_mismatch`
    let mut bad_tool_overrides = BTreeMap::new();
    bad_tool_overrides.insert(
        "media_creator".to_owned(),
        AgentConfigOverride {
            enabled: true,
            tool_providers: Some(BTreeMap::from([(
                "image_create".to_owned(),
                "llm_video".to_owned(),
            )])),
            ..Default::default()
        },
    );
    assert_eq!(
        validate_agent_overrides("llm_chat", "llm_chat", &bad_tool_overrides, &catalog),
        Err("provider_capability_mismatch")
    );

    assert!(crate::providers::llm::valid_llm_tags(Some(&[
        "tts".to_owned()
    ])));
    assert!(crate::providers::llm::valid_llm_tags(Some(&[
        "TTS".to_owned()
    ])));
    assert_eq!(
        crate::providers::llm::llm_config_tags(r#"{"tags":["tts"]}"#),
        vec!["tts".to_owned()]
    );
    assert!(crate::providers::llm::llm_config_supports_tag(
        r#"{"tags":["tts"]}"#,
        "audio"
    ));
}

#[tokio::test]
async fn catalog_media_backend_and_reasoning_tag_wire_provider_capabilities() {
    use crate::config::SecretString;
    use axum::{
        Json, Router,
        http::{HeaderMap, StatusCode, header},
        response::IntoResponse,
        routing::post,
    };
    use base64::{Engine as _, engine::general_purpose::STANDARD};
    use orchestrator_agent_rs::ReasoningEffort;
    use orchestrator_flow_rs::{
        ImageGenerationRequest, ImageGeneratorBackend, TtsSynthesisRequest, TtsSynthesizerBackend,
        VideoGenerationRequest, VideoGeneratorBackend,
    };

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base_url = format!("http://{}/v1", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            Router::new()
                .route(
                    "/v1/images/generations",
                    post(|| async {
                        Json(serde_json::json!({
                            "data": [{ "b64_json": STANDARD.encode(b"UPSTREAM_PNG_BYTES") }]
                        }))
                    }),
                )
                .route(
                    "/v1/videos/generations",
                    post(|| async {
                        Json(serde_json::json!({
                            "data": [{ "b64_json": STANDARD.encode(b"UPSTREAM_MP4_BYTES") }]
                        }))
                    }),
                )
                .route(
                    "/v1/audio/speech",
                    post(|| async {
                        let mut headers = HeaderMap::new();
                        headers.insert(header::CONTENT_TYPE, "audio/wav".parse().unwrap());
                        (StatusCode::OK, headers, b"RIFF_UPSTREAM_WAV".to_vec()).into_response()
                    }),
                ),
        )
        .await
        .unwrap();
    });

    let reasoning_provider = ResolvedLlmProvider {
        provider_ref: "llm_reasoner".into(),
        name: "Reasoner".into(),
        source: "database",
        adapter: "openai".into(),
        model: "o3-mini".into(),
        base_url: base_url.clone(),
        api_key: Some(SecretString::new("sk-reasoner")),
        gateway_binding: None,
        timeout_ms: 10_000,
        max_tokens: 8_192,
        context_limit_tokens: 128_000,
        reasoning_effort: None,
        temperature_percent: None,
        tags: vec!["reasoning".into()],
        muse_options: None,
    };
    let media_provider = ResolvedLlmProvider {
        provider_ref: "llm_media".into(),
        name: "Omni Media".into(),
        source: "database",
        adapter: "openai".into(),
        model: "gpt-omni-media".into(),
        base_url,
        api_key: Some(SecretString::new("sk-media")),
        gateway_binding: None,
        timeout_ms: 10_000,
        max_tokens: 4_096,
        context_limit_tokens: 64_000,
        reasoning_effort: None,
        temperature_percent: None,
        tags: vec!["image".into(), "video".into(), "audio".into()],
        muse_options: None,
    };

    let catalog = vec![reasoning_provider, media_provider];
    let settings = PersistedBackgroundSettings {
        orchestrator_llm_provider: "llm_reasoner".into(),
        worker_llm_provider: "llm_reasoner".into(),
        ..PersistedBackgroundSettings::default()
    };
    let (_, defs) = build_agent_definitions(
        &OrchestratorSectionConfig::default(),
        &settings,
        &catalog,
    );
    let daily_root = defs.iter().find(|d| d.id == "daily_root").unwrap();
    assert_eq!(
        daily_root.llm.reasoning_effort,
        Some(ReasoningEffort::Medium),
        "reasoning-tagged provider should default AgentDefinition reasoning_effort to Medium"
    );

    let backend = CatalogMediaBackend::from_catalog(catalog).with_settings(settings);
    let img = backend
        .generate_image(&ImageGenerationRequest {
            prompt: "A diagram".into(),
            title: None,
            filename: "diagram.png".into(),
            format: "png".into(),
            width: 256,
            height: 256,
            svg_content: None,
            reference_images: Vec::new(),
            transparent_background: false,
        })
        .await
        .unwrap();
    assert_eq!(img.provider, "llm_media");
    assert_eq!(img.bytes, b"UPSTREAM_PNG_BYTES");

    let vid = backend
        .generate_video(&VideoGenerationRequest {
            prompt: "A rack flythrough".into(),
            title: Some("Flythrough".into()),
            filename: "flythrough.mp4".into(),
            width: 640,
            height: 360,
            duration_secs: 4,
            first_frame: None,
        })
        .await
        .unwrap();
    assert_eq!(vid.provider, "llm_media");
    assert_eq!(vid.bytes, b"UPSTREAM_MP4_BYTES");
    assert_eq!(vid.mime_type, "video/mp4");

    let audio = backend
        .synthesize_tts(&TtsSynthesisRequest {
            text: "Xin chào".into(),
            filename: "greeting.wav".into(),
            voice: "alloy".into(),
            language: "vi".into(),
            speed: 1.0,
        })
        .await
        .unwrap();
    assert_eq!(audio.provider, "llm_media");
    assert_eq!(audio.bytes, b"RIFF_UPSTREAM_WAV");

    // Server native TTS provider (`type = "tts"`, `tags: ["tts"]`, no API key required)
    struct TestServerTts;
    impl crate::providers::TtsProvider for TestServerTts {
        fn adapter(&self) -> &'static str {
            "vieneu"
        }
        fn synthesize_stream(
            &self,
            _text: &str,
            on_pcm: &mut dyn FnMut(crate::audio::PcmF32Mono) -> Result<(), crate::providers::TtsError>,
        ) -> Result<(), crate::providers::TtsError> {
            on_pcm(crate::audio::PcmF32Mono::new(vec![0.15; 2_400], 24_000))
        }
    }

    let tts_runtime = std::sync::Arc::new(crate::workers::TtsWorkerRuntime::new(
        std::sync::Arc::new(TestServerTts),
        crate::workers::WorkerRuntimeConfig {
            max_workers: 1,
            final_timeout: std::time::Duration::from_secs(10),
            cleanup_grace: std::time::Duration::from_secs(5),
            ..crate::workers::WorkerRuntimeConfig::default()
        },
    ));
    let server_tts_provider = ResolvedLlmProvider {
        provider_ref: "tts_vieneu_local".into(),
        name: "VieNeu Local TTS".into(),
        source: "database",
        adapter: "vieneu".into(),
        model: "Binh".into(),
        base_url: String::new(),
        api_key: None,
        gateway_binding: None,
        timeout_ms: 15_000,
        max_tokens: 4_096,
        context_limit_tokens: 64_000,
        reasoning_effort: None,
        temperature_percent: None,
        tags: vec!["tts".into()],
        muse_options: None,
    };
    let tts_backend = CatalogMediaBackend::from_catalog(vec![server_tts_provider])
        .with_tts_runtime("tts_vieneu_local", tts_runtime.clone());
    let server_audio = tts_backend
        .synthesize_tts(&TtsSynthesisRequest {
            text: "Xin chào từ server TTS".into(),
            filename: "server-tts.wav".into(),
            voice: "vi-default".into(),
            language: "vi-VN".into(),
            speed: 1.0,
        })
        .await
        .unwrap();
    assert_eq!(server_audio.provider, "tts_vieneu_local");
    assert_eq!(server_audio.mime_type, "audio/wav");
    assert_eq!(server_audio.sample_rate_hz, 24_000);
    assert!(server_audio.bytes.starts_with(b"RIFF"));
    let _ = tts_runtime.shutdown_acknowledged();

    // Fallback when catalog has no image/video/audio tagged provider
    let empty_backend = CatalogMediaBackend::from_catalog(Vec::new());
    let fallback_img = empty_backend
        .generate_image(&ImageGenerationRequest {
            prompt: "Fallback".into(),
            title: None,
            filename: "fallback.png".into(),
            format: "png".into(),
            width: 64,
            height: 64,
            svg_content: None,
            reference_images: Vec::new(),
            transparent_background: false,
        })
        .await
        .unwrap();
    assert_eq!(fallback_img.provider, "builtin_png_renderer");

    let fallback_vid = empty_backend
        .generate_video(&VideoGenerationRequest {
            prompt: "Fallback video".into(),
            title: None,
            filename: "fallback.mp4".into(),
            width: 64,
            height: 64,
            duration_secs: 2,
            first_frame: None,
        })
        .await
        .unwrap();
    assert_eq!(fallback_vid.provider, "builtin_mp4_renderer");
    assert_eq!(&fallback_vid.bytes[4..8], b"ftyp");

    // Default dimensions when width/height are 0 (1024x1024 for PNG, 640x360 for MP4)
    let default_dim_img = backend
        .generate_image(&ImageGenerationRequest {
            prompt: "Default size diagram".into(),
            title: None,
            filename: "default-size.png".into(),
            format: "png".into(),
            width: 0,
            height: 0,
            svg_content: None,
            reference_images: Vec::new(),
            transparent_background: false,
        })
        .await
        .unwrap();
    assert_eq!(default_dim_img.width, orchestrator_flow_rs::DEFAULT_PNG_WIDTH);
    assert_eq!(default_dim_img.height, orchestrator_flow_rs::DEFAULT_PNG_HEIGHT);

    server.abort();
}

#[test]
fn canonical_prompts_enforce_summarize_analyze_visualize_pipeline_and_flexible_dimensions() {
    assert!(DAILY_ROOT_PROMPT.contains("Standard Multi-Step Pipeline: Summarize → Analyze → Visualize"));
    assert!(DAILY_ROOT_PROMPT.contains("STRICT SEQUENTIAL RULE"));
    assert!(DAILY_ROOT_PROMPT.contains("Visual Brief"));
    assert!(SOURCE_COLLECTOR_PROMPT.contains("Phân tích chi tiết (Deep Analysis)"));
    assert!(SOURCE_COLLECTOR_PROMPT.contains("Điểm nhấn trực quan (Key Visual Concepts)"));
    assert!(WIKI_RESEARCHER_PROMPT.contains("Điểm nhấn trực quan (Key Visual Concepts)"));
    assert!(MEDIA_CREATOR_PROMPT.contains("Agent-Selectable or Smart Default"));
    assert!(MEDIA_CREATOR_PROMPT.contains("svg_content"));
}

#[test]
fn custom_subagents_enforce_flow_tool_boundaries_and_sync_active_subagents_prompts() {
    let temp_dir = std::env::temp_dir().join(format!("custom-subagent-unit-{}", uuid::Uuid::new_v4()));
    let store_dir = temp_dir.join("store");
    let prompts_dir = temp_dir.join("prompts");
    std::fs::create_dir_all(&store_dir).unwrap();
    std::fs::create_dir_all(&prompts_dir).unwrap();

    // 1. Validate custom subagent ID rules
    assert!(valid_custom_subagent_id("arxiv_digest"));
    assert!(valid_custom_subagent_id("wiki-fact-checker-2"));
    assert!(!valid_custom_subagent_id(""));
    assert!(!valid_custom_subagent_id("-bad-start"));
    assert!(!valid_custom_subagent_id("Upper_Case"));
    assert!(!valid_custom_subagent_id("daily_root"));
    assert!(!valid_custom_subagent_id("source_collector"));
    assert!(!valid_custom_subagent_id("subagent"));
    assert!(!valid_custom_subagent_id("skill"));

    // 2. Flow 2 (`wiki`) custom subagent pool must never expose external collector tools
    for &forbidden in orchestrator_flow_rs::COLLECTOR_TOOL_NAMES {
        assert!(
            !CUSTOM_WIKI_SUBAGENT_TOOLS.contains(&forbidden),
            "CUSTOM_WIKI_SUBAGENT_TOOLS must not contain collector tool '{forbidden}'"
        );
    }
    assert!(CUSTOM_DAILY_SUBAGENT_TOOLS.contains(&"yt_fetch_transcript"));
    assert!(CUSTOM_WIKI_SUBAGENT_TOOLS.contains(&"wiki_lint"));

    let cfg = OrchestratorSectionConfig {
        store_dir: store_dir.clone(),
        prompts_dir: prompts_dir.clone(),
        ..OrchestratorSectionConfig::default()
    };
    let catalog = vec![default_orchestrator_provider(&cfg)];

    // 3. Configure one Flow 1 custom subagent and one Flow 2 custom subagent
    let mut settings = PersistedBackgroundSettings::default();
    settings.agents.insert(
        "arxiv_digest".to_owned(),
        AgentConfigOverride {
            enabled: true,
            flow_id: Some("daily".to_owned()),
            steps: Some(15),
            allowed_tools: Some(vec!["fetch_html".to_owned(), "markdown_create".to_owned()]),
            allowed_skills: Some(vec!["wiki-query".to_owned()]),
            description: Some("Summarizes arXiv papers into Markdown briefs.".to_owned()),
            system_prompt: Some("You are the arXiv digest specialist.".to_owned()),
            ..Default::default()
        },
    );
    settings.agents.insert(
        "wiki_fact_auditor".to_owned(),
        AgentConfigOverride {
            enabled: true,
            flow_id: Some("wiki".to_owned()),
            steps: Some(25),
            allowed_tools: Some(vec!["wiki_query".to_owned(), "wiki_lint".to_owned()]),
            allowed_skills: Some(Vec::new()),
            description: Some("Audits Wiki pages for citation consistency.".to_owned()),
            system_prompt: Some("You are the Wiki fact auditor.".to_owned()),
            ..Default::default()
        },
    );

    assert!(
        validate_agent_overrides(
            &settings.orchestrator_llm_provider,
            &settings.worker_llm_provider,
            &settings.agents,
            &catalog,
        )
        .is_ok()
    );

    // Flow 2 custom subagent with collector tool must fail validation
    let mut bad_agents = settings.agents.clone();
    bad_agents.get_mut("wiki_fact_auditor").unwrap().allowed_tools =
        Some(vec!["yt_fetch_transcript".to_owned()]);
    assert_eq!(
        validate_agent_overrides(
            &settings.orchestrator_llm_provider,
            &settings.worker_llm_provider,
            &bad_agents,
            &catalog,
        ),
        Err("invalid_arguments")
    );

    // 4. Persistence roundtrip & prompt file generation + cleanup
    save_persisted_settings(&store_dir, &settings).unwrap();
    let loaded = load_persisted_settings(&store_dir).expect("loaded settings");
    assert_eq!(loaded.agents.len(), 2);

    write_agent_prompts(&prompts_dir, &loaded).unwrap();
    assert_eq!(
        std::fs::read_to_string(prompts_dir.join("arxiv_digest.md")).unwrap(),
        "You are the arXiv digest specialist."
    );
    let daily_root_disk_prompt =
        std::fs::read_to_string(prompts_dir.join("daily_root.md")).unwrap();
    assert!(daily_root_disk_prompt.contains("<active_subagents>"));
    assert!(daily_root_disk_prompt.contains("`arxiv_digest` (custom): Summarizes arXiv papers into Markdown briefs."));

    // Project JSON and verify `is_builtin`, auto-injected `skill` tool, and `allowed_subagents`
    let projected = project_agents_json(&cfg, &loaded, &catalog);
    assert_eq!(projected.len(), 11);
    let daily_root_json = projected.iter().find(|a| a["id"] == "daily_root").unwrap();
    assert_eq!(daily_root_json["is_builtin"], true);
    assert!(
        daily_root_json["allowed_subagents"]
            .as_array()
            .unwrap()
            .contains(&serde_json::json!("arxiv_digest"))
    );
    let arxiv_json = projected.iter().find(|a| a["id"] == "arxiv_digest").unwrap();
    assert_eq!(arxiv_json["is_builtin"], false);
    assert_eq!(arxiv_json["flow_id"], "daily");
    assert_eq!(arxiv_json["parent_id"], "daily_root");
    assert_eq!(
        arxiv_json["allowed_tools"],
        serde_json::json!(["skill", "fetch_html", "markdown_create"])
    );

    // Remove `arxiv_digest` and verify its `.md` file is pruned on `write_agent_prompts`
    let mut reduced = loaded;
    reduced.agents.remove("arxiv_digest");
    write_agent_prompts(&prompts_dir, &reduced).unwrap();
    assert!(!prompts_dir.join("arxiv_digest.md").exists());

    let _ = std::fs::remove_dir_all(&temp_dir);
}

#[test]
fn muse_gateway_provider_in_catalog_supports_media_and_custom_executor() {
    use std::collections::BTreeMap;

    // 1. Standalone `muse` adapter is removed; Muse is configured via `llm_gateway` (`pool:muse`)
    let valid_json = r#"{"gateway_binding":"pool:muse","model":"muse-image","account_concurrency":2,"cdp_port":19222,"headless":true,"tags":["image","video"]}"#;
    assert!(crate::database::provider_config::validate_raw("muse", valid_json).is_err());
    let canonical = crate::database::provider_config::validate_raw("llm_gateway", valid_json)
        .expect("valid llm_gateway muse config_json");
    assert!(canonical.contains("pool:muse"));
    assert!(canonical.contains("muse-image"));

    // Rejects embedded credentials in `config_json`
    let with_secret = r#"{"gateway_binding":"pool:muse","model":"muse-image","api_key":"secret_token"}"#;
    assert!(crate::database::provider_config::validate_raw("llm_gateway", with_secret).is_err());

    // 2. Verify `llm_gateway` bound to `pool:muse` in Task Flow catalog exposes `image` & `video`
    // and uses `ProviderAuth::CustomExecutor` (never `ProviderAuth::ApiKey` or plain OpenAI).
    let cfg = OrchestratorSectionConfig::default();
    let chat_prov = default_orchestrator_provider(&cfg);
    let muse_prov = ResolvedLlmProvider {
        provider_ref: "muse_media".into(),
        name: "Muse Media".into(),
        source: "database",
        adapter: "llm_gateway".into(),
        model: "muse-image".into(),
        base_url: "https://muse.ai".into(),
        api_key: None,
        gateway_binding: Some(crate::services::llm_gateway::GatewayBinding::Pool(
            crate::services::llm_gateway::GatewayVendor::Muse,
        )),
        timeout_ms: 300_000,
        max_tokens: 4_096,
        context_limit_tokens: 128_000,
        reasoning_effort: None,
        temperature_percent: None,
        tags: vec!["image".into(), "video".into()],
        muse_options: Some(crate::services::task_flow::agent_settings::MuseCatalogOptions {
            account_concurrency: 2,
            cdp_port: 19222,
            headless: true,
        }),
    };

    assert!(muse_prov.supports_tag("image"));
    assert!(muse_prov.supports_tag("video"));
    assert!(!muse_prov.supports_tag("chat"));
    assert!(muse_prov.is_muse_provider());
    let muse_vc = muse_prov.video_constraints();
    assert_eq!(muse_vc.max_clip_duration_secs, 10);
    assert_eq!(muse_vc.max_clips_per_call, 6);
    assert_eq!(muse_vc.max_total_duration_secs, 60);
    assert!(muse_vc.provider_guidance.contains("video_concat"));

    let openai_vc = chat_prov.video_constraints();
    assert_eq!(openai_vc.max_clip_duration_secs, 10);
    assert_eq!(openai_vc.max_clips_per_call, 1);
    assert_eq!(openai_vc.max_total_duration_secs, 10);

    let catalog = vec![chat_prov, muse_prov];
    let mut agents = BTreeMap::new();
    agents.insert(
        "media_creator".to_owned(),
        AgentConfigOverride {
            enabled: true,
            provider_ref: Some("muse_media".to_owned()),
            tool_providers: Some(BTreeMap::from([
                ("image_create".to_owned(), "muse_media".to_owned()),
                ("video_create".to_owned(), "muse_media".to_owned()),
            ])),
            ..Default::default()
        },
    );
    assert!(
        validate_agent_overrides(
            DEFAULT_ORCHESTRATOR_PROVIDER_REF,
            DEFAULT_ORCHESTRATOR_PROVIDER_REF,
            &agents,
            &catalog,
        )
        .is_ok()
    );

    let settings = PersistedBackgroundSettings {
        orchestrator_llm_provider: DEFAULT_ORCHESTRATOR_PROVIDER_REF.into(),
        worker_llm_provider: DEFAULT_ORCHESTRATOR_PROVIDER_REF.into(),
        agents,
        ..Default::default()
    };
    let (orch_providers, defs) = build_agent_definitions(&cfg, &settings, &catalog);
    let muse_orch = orch_providers
        .iter()
        .find(|p| p.id == "muse_media")
        .expect("muse_media registered with CustomExecutor");
    assert!(matches!(
        muse_orch.auth,
        orchestrator_agent_rs::ProviderAuth::CustomExecutor(_)
    ));
    let media_def = defs.iter().find(|d| d.id == "media_creator").unwrap();
    assert_eq!(media_def.provider_id, DEFAULT_ORCHESTRATOR_PROVIDER_REF);
    assert!(
        media_def
            .description
            .contains("[Active video_create provider: Muse Media (model: muse-image); max 10s/clip, up to 6 clip(s) = 60s per call; merge multi-clip outputs with video_concat]")
    );
    let media_spec = crate::services::task_flow::find_agent_spec("media_creator").unwrap();
    assert!(media_spec.default_allowed_tools.contains(&"video_concat"));
    assert!(media_spec.available_tools.contains(&"video_concat"));

    let backend = CatalogMediaBackend::from_catalog(catalog).with_settings(settings);
    let backend_vc = orchestrator_flow_rs::VideoGeneratorBackend::video_constraints(&backend);
    assert_eq!(backend_vc.max_clip_duration_secs, 10);
    assert_eq!(backend_vc.max_clips_per_call, 6);
    assert_eq!(backend_vc.max_total_duration_secs, 60);

    // 3. Verify `build_generated_video_payload` converts multi-part `LlmOutput::Media` + `assistant_message`
    // into `GeneratedVideoPayload` with accurate clip durations parsed via `inspect_mp4_duration_secs`.
    let mut mvhd_10s = Vec::new();
    let mvhd_body_len = 4 + 16 + 4; // version/flags + 4 u32s + duration
    let mvhd_box_len = (8 + mvhd_body_len) as u32;
    let moov_box_len = 8 + mvhd_box_len;
    mvhd_10s.extend_from_slice(&20u32.to_be_bytes());
    mvhd_10s.extend_from_slice(b"ftypisom\x00\x00\x02\x00isom");
    mvhd_10s.extend_from_slice(&moov_box_len.to_be_bytes());
    mvhd_10s.extend_from_slice(b"moov");
    mvhd_10s.extend_from_slice(&mvhd_box_len.to_be_bytes());
    mvhd_10s.extend_from_slice(b"mvhd");
    mvhd_10s.extend_from_slice(&[0, 0, 0, 0]); // version 0
    mvhd_10s.extend_from_slice(&[0u8; 8]); // creation + modification time
    mvhd_10s.extend_from_slice(&1_000u32.to_be_bytes()); // timescale = 1000
    mvhd_10s.extend_from_slice(&10_000u32.to_be_bytes()); // duration = 10_000 -> 10s
    let multi_media = crate::providers::LlmOutput::from_media_parts(
        vec![
            crate::providers::MediaPart {
                bytes: mvhd_10s.clone(),
                mime_type: "video/mp4".into(),
            },
            crate::providers::MediaPart {
                bytes: mvhd_10s,
                mime_type: "video/mp4".into(),
            },
        ],
        Some("Scene 1 (0-10s)\n\nScene 2 (10-20s)".into()),
    )
    .unwrap();
    let multi_payload =
        crate::services::task_flow::agent_settings::build_generated_video_payload(
            &orchestrator_flow_rs::VideoGenerationRequest {
                prompt: "Two scenes".into(),
                title: None,
                filename: "two-scenes.mp4".into(),
                width: 1280,
                height: 720,
                duration_secs: 20,
                first_frame: None,
            },
            "muse_media",
            &backend_vc,
            1280,
            720,
            20,
            multi_media,
        )
        .unwrap();
    assert_eq!(multi_payload.clips.len(), 2);
    assert_eq!(multi_payload.clips[0].filename, "two-scenes-part-1.mp4");
    assert_eq!(multi_payload.clips[1].filename, "two-scenes-part-2.mp4");
    assert_eq!(multi_payload.clips[0].duration_secs, 10);
    assert_eq!(multi_payload.clips[1].duration_secs, 10);
    assert_eq!(multi_payload.duration_secs, 20);
    assert_eq!(
        multi_payload.provider_message.as_deref(),
        Some("Scene 1 (0-10s)\n\nScene 2 (10-20s)")
    );
}
