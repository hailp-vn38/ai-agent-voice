use super::super::*;
use super::*;

#[test]
fn playlist_progress_reports_repeated_items_and_completion_from_transcript() {
    let recipe = PlaylistSourcesRecipe::default();
    let params = json!({"url":"https://www.youtube.com/playlist?list=PL1"});
    let list = ModelMessage::tool_result("list", "yt_list_playlist", "listed");
    let status = ModelMessage::tool_result(
        "status",
        "collection_status",
        "TOTAL: 2 | REMAINING: 2\nNEXT:\n1. video_id=v1 title=One",
    );
    let fetched = ModelMessage::tool_result("fetch-v1", "yt_fetch_transcript", "saved v1");
    let running = vec![
        ModelMessage::assistant_with_calls(
            "",
            vec![ModelToolCall {
                id: "fetch-v1".into(),
                name: "yt_fetch_transcript".into(),
                arguments: json!({"video_id":"v1"}),
            }],
        ),
        list.clone(),
        status,
        fetched,
    ];

    let progress = recipe.progress(&params, &running);
    assert_eq!(progress.current_step, Some(2));
    assert_eq!(progress.completed_items, Some(0));
    assert_eq!(progress.total_items, Some(2));
    assert!(progress.stop_reason.is_none());

    let completed_status = ModelMessage::tool_result(
        "status-done",
        "collection_status",
        "TOTAL: 1 | REMAINING: 0\nNEXT:",
    );
    let selection = selector_result(
        "select-topic",
        "selector-session",
        r#"{"domain":"youtube","topic":"playlist"}"#,
    );
    let completed = vec![
        list,
        completed_status,
        selection[0].clone(),
        selection[1].clone(),
        ModelMessage::assistant_with_calls(
            "",
            vec![
                ModelToolCall {
                    id: "fetch-one".into(),
                    name: "yt_fetch_transcript".into(),
                    arguments: json!({"video_id":"v1"}),
                },
                ModelToolCall {
                    id: "save-one".into(),
                    name: "add_to_sources".into(),
                    arguments: json!({"video_id":"v1"}),
                },
            ],
        ),
        ModelMessage::tool_result("fetch-one", "yt_fetch_transcript", "ok"),
        ModelMessage::tool_result("save-one", "add_to_sources", "ok"),
    ];
    let progress = recipe.progress(&params, &completed);
    assert_eq!(progress.current_step, None);
    assert_eq!(progress.completed_steps, vec![0, 1, 2, 3]);
    assert_eq!(progress.completed_items, Some(1));
    assert_eq!(progress.total_items, Some(1));
    assert!(progress.stop_reason.is_none());
}

#[test]
fn playlist_progress_reports_stopped_step_and_reason() {
    let recipe = PlaylistSourcesRecipe::default();
    let params = json!({"url":"https://www.youtube.com/playlist?list=PL1"});
    let transcript = vec![ModelMessage::tool_result(
        "list",
        "yt_list_playlist",
        "error playlist is private",
    )];

    let progress = recipe.progress(&params, &transcript);
    assert_eq!(progress.current_step, None);
    assert_eq!(
        progress.stop_reason.as_deref(),
        Some("error playlist is private")
    );
    assert_eq!(progress.failed_step, Some(0));
}
