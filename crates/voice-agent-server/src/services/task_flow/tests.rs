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
            "media_processor",
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
    assert!(
        report_stager
            .default_allowed_tools
            .contains(&"add_to_sources")
    );
    assert!(
        report_stager
            .default_allowed_tools
            .contains(&"markdown_create")
    );
    assert!(report_stager.default_allowed_tools.contains(&"chart_write"));
    assert!(report_stager.default_allowed_tools.contains(&"chart_get"));
    assert!(
        report_stager
            .default_allowed_skills
            .contains(&"trading-chart")
    );
    assert!(
        !report_stager
            .default_allowed_tools
            .contains(&"image_create")
            && !report_stager
                .default_allowed_tools
                .contains(&"tts_synthesize"),
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
    assert_eq!(
        media_creator.default_allowed_tools,
        &["image_create", "video_create"]
    );
    assert_eq!(
        media_creator.available_tools,
        &["skill", "image_create", "video_create"]
    );
    assert_eq!(
        media_creator.tool_required_tag("image_create"),
        Some("image")
    );
    assert_eq!(
        media_creator.tool_required_tag("video_create"),
        Some("video")
    );
    assert_eq!(media_creator.tool_required_tag("asr_transcribe"), None);
    assert_eq!(media_creator.tool_required_tag("tts_synthesize"), None);
    assert!(
        !media_creator
            .default_allowed_tools
            .contains(&"markdown_create"),
        "media_creator must not duplicate report_stager markdown_create tool"
    );

    let media_processor = find_agent_spec("media_processor").expect("media_processor spec");
    assert_eq!(media_processor.flow_id, "daily");
    assert_eq!(media_processor.parent_id, Some("daily_root"));
    assert_eq!(
        media_processor.default_allowed_tools,
        &[
            "asr_transcribe",
            "tts_synthesize",
            "video_concat",
            "artifact_create"
        ]
    );
    assert_eq!(
        media_processor.available_tools,
        &[
            "skill",
            "asr_transcribe",
            "tts_synthesize",
            "video_concat",
            "artifact_create"
        ]
    );
    assert_eq!(
        media_processor.tool_required_tag("asr_transcribe"),
        Some("asr")
    );
    assert_eq!(
        media_processor.tool_required_tag("tts_synthesize"),
        Some("tts")
    );
    assert_eq!(
        find_tool_owner_spec("image_create").map(|s| s.id),
        Some("media_creator")
    );
    assert_eq!(
        find_tool_owner_spec("video_create").map(|s| s.id),
        Some("media_creator")
    );
    assert_eq!(
        find_tool_owner_spec("tts_synthesize").map(|s| s.id),
        Some("media_processor")
    );
    assert_eq!(
        find_tool_owner_spec("asr_transcribe").map(|s| s.id),
        Some("media_processor")
    );

    let wiki_root = find_agent_spec("wiki_root").expect("wiki_root spec");
    assert_eq!(wiki_root.flow_id, "wiki");
    assert!(!wiki_root.user_routable);
    assert!(!is_user_routable_root_agent("wiki_root"));
    assert_eq!(normalize_user_target_agent("wiki_root"), "daily_root");
    assert_eq!(
        normalize_user_target_agent("default/wiki_root"),
        "daily_root"
    );
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
    let msgs = agent_run["messages"]
        .as_array()
        .expect("agent_run.messages array");
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
fn canonical_prompts_enforce_summarize_analyze_visualize_pipeline_and_flexible_dimensions() {
    assert!(
        DAILY_ROOT_PROMPT.contains("Standard Multi-Step Pipeline: Summarize → Analyze → Visualize")
    );
    assert!(DAILY_ROOT_PROMPT.contains("STRICT SEQUENTIAL RULE"));
    assert!(DAILY_ROOT_PROMPT.contains("Visual Brief"));
    assert!(DAILY_ROOT_PROMPT.contains("Video Built-in Audio & Subtitles vs. Standalone TTS"));
    assert!(DAILY_ROOT_PROMPT.contains("media_processor"));
    assert!(SOURCE_COLLECTOR_PROMPT.contains("Phân tích chi tiết (Deep Analysis)"));
    assert!(SOURCE_COLLECTOR_PROMPT.contains("Điểm nhấn trực quan (Key Visual Concepts)"));
    assert!(WIKI_RESEARCHER_PROMPT.contains("Điểm nhấn trực quan (Key Visual Concepts)"));
    assert!(MEDIA_CREATOR_PROMPT.contains("Agent-Selectable or Smart Default"));
    assert!(MEDIA_CREATOR_PROMPT.contains("svg_content"));
    assert!(MEDIA_CREATOR_PROMPT.contains("Built-in Audio, Subtitles & Auto-Merge"));
    assert!(
        MEDIA_CREATOR_PROMPT
            .contains("do **NOT** split `prompt` into separate 10-second sub-prompts")
    );
    assert!(MEDIA_PROCESSOR_PROMPT.contains("asr_transcribe"));
    assert!(MEDIA_PROCESSOR_PROMPT.contains("tts_synthesize"));
    assert!(MEDIA_PROCESSOR_PROMPT.contains("video_concat"));
    assert!(MEDIA_PROCESSOR_PROMPT.contains("artifact_create"));
}

#[test]
fn custom_subagents_enforce_flow_tool_boundaries_and_sync_active_subagents_prompts() {
    let temp_dir =
        std::env::temp_dir().join(format!("custom-subagent-unit-{}", uuid::Uuid::new_v4()));
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
    bad_agents
        .get_mut("wiki_fact_auditor")
        .unwrap()
        .allowed_tools = Some(vec!["yt_fetch_transcript".to_owned()]);
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
    assert!(
        daily_root_disk_prompt
            .contains("`arxiv_digest` (custom): Summarizes arXiv papers into Markdown briefs.")
    );

    // Project JSON and verify `is_builtin`, auto-injected `skill` tool, and `allowed_subagents`
    let projected = project_agents_json(&cfg, &loaded, &catalog);
    assert_eq!(projected.len(), 12);
    let daily_root_json = projected.iter().find(|a| a["id"] == "daily_root").unwrap();
    assert_eq!(daily_root_json["is_builtin"], true);
    assert!(
        daily_root_json["allowed_subagents"]
            .as_array()
            .unwrap()
            .contains(&serde_json::json!("arxiv_digest"))
    );
    let arxiv_json = projected
        .iter()
        .find(|a| a["id"] == "arxiv_digest")
        .unwrap();
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
