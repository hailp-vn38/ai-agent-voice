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
