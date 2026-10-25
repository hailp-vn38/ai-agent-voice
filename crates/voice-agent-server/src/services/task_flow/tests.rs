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
    for tool in [
        "yt_list_playlist",
        "yt_fetch_transcript",
        "anydoc_inspect_pdf",
        "anydoc_save_to_sources",
        "fetch_html",
        "add_to_sources",
    ] {
        assert!(
            collector.default_allowed_tools.contains(&tool),
            "source_collector in Flow 1 (daily) must have tool '{tool}'"
        );
    }

    let report_stager = find_agent_spec("report_stager").expect("report_stager spec");
    assert!(report_stager.default_allowed_tools.contains(&"add_to_sources"));
    assert!(report_stager.default_allowed_tools.contains(&"markdown_create"));
    assert!(
        !report_stager.default_allowed_tools.contains(&"image_create")
            && !report_stager.default_allowed_tools.contains(&"tts_synthesize"),
        "report_stager must not duplicate media_creator tools"
    );

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
    assert_eq!(
        wiki_root.default_subagents,
        &["wiki_ingestor", "wiki_curator"]
    );

    for spec in AGENT_SPECS.iter().filter(|s| s.flow_id == "wiki") {
        for forbidden in [
            "yt_list_playlist",
            "yt_fetch_transcript",
            "anydoc_inspect_pdf",
            "anydoc_save_to_sources",
            "fetch_html",
        ] {
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
