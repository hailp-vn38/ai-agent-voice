use super::super::*;
use super::*;

#[test]
fn recipe_fetches_then_saves_a_bounded_ledger_batch() {
    let recipe = PlaylistSourcesRecipe::default();
    let params = json!({"url":"https://www.youtube.com/playlist?list=PL1"});
    let listed = ModelMessage::tool_result("list", "yt_list_playlist", "1. video_id=v1 title=One");
    let decision = recipe.next(&params, std::slice::from_ref(&listed));
    let RecipeDecision::ToolCalls(calls) = decision else {
        panic!("expected status lookup")
    };
    assert_eq!(calls[0].name, "collection_status");
    let status = ModelMessage::tool_result(
        "status",
        "collection_status",
        "TOTAL: 1 | REMAINING: 1\nNEXT:\n1. video_id=v1 attempts=0 title=One",
    );
    let decision = recipe.next(&params, &[listed.clone(), status.clone()]);
    let RecipeDecision::ToolCalls(calls) = decision else {
        panic!("expected transcript fetch")
    };
    assert_eq!(calls[0].name, "yt_fetch_transcript");

    let fetch_call = ModelMessage::assistant_with_calls("", vec![calls[0].clone()]);
    let fetched =
        ModelMessage::tool_result(calls[0].id.clone(), "yt_fetch_transcript", "TRANSCRIPT");
    let transcript = vec![listed, status, fetch_call, fetched];
    let decision = recipe.next(&params, &transcript);
    let RecipeDecision::ToolCalls(calls) = decision else {
        panic!("expected refreshed collection status")
    };
    assert_eq!(calls[0].name, "collection_status");

    let refresh_call = ModelMessage::assistant_with_calls("", vec![calls[0].clone()]);
    let refreshed = ModelMessage::tool_result(
        calls[0].id.clone(),
        "collection_status",
        "TOTAL: 1 | REMAINING: 1\nNEXT:\n1. video_id=v1 attempts=0 title=One",
    );
    let mut transcript = [transcript, vec![refresh_call, refreshed]].concat();
    transcript.extend(selector_result(
        "select-topic",
        "selector-session",
        r#"{"domain":"youtube","topic":"playlist"}"#,
    ));
    let decision = recipe.next(&params, &transcript);
    let RecipeDecision::ToolCalls(calls) = decision else {
        panic!("expected source save after refresh")
    };
    assert_eq!(calls[0].name, "add_to_sources");
}

#[test]
fn failed_video_does_not_prevent_successful_sibling_from_being_saved() {
    let recipe = PlaylistSourcesRecipe::default();
    let params = json!({"url":"https://www.youtube.com/playlist?list=PL1"});
    let list_call = ModelToolCall {
        id: "list".into(),
        name: "yt_list_playlist".into(),
        arguments: json!({"url":"https://www.youtube.com/playlist?list=PL1"}),
    };
    let list = ModelMessage::assistant_with_calls("", vec![list_call]);
    let listed = ModelMessage::tool_result("list", "yt_list_playlist", "listed");
    let status_call = ModelToolCall {
        id: "status".into(),
        name: "collection_status".into(),
        arguments: json!({"playlist_url":"https://www.youtube.com/playlist?list=PL1"}),
    };
    let status_assistant = ModelMessage::assistant_with_calls("", vec![status_call]);
    let status = ModelMessage::tool_result(
        "status",
        "collection_status",
        "TOTAL: 2 | REMAINING: 2\nNEXT:\n1. video_id=bad attempts=0 title=Bad\n2. video_id=good attempts=0 title=Good",
    );
    let fetch_calls = vec![
        ModelToolCall {
            id: "f-bad".into(),
            name: "yt_fetch_transcript".into(),
            arguments: json!({"video_id":"bad", "title":"Bad"}),
        },
        ModelToolCall {
            id: "f-good".into(),
            name: "yt_fetch_transcript".into(),
            arguments: json!({"video_id":"good", "title":"Good"}),
        },
    ];
    let fetch_assistant = ModelMessage::assistant_with_calls("", fetch_calls);
    let failed = ModelMessage::tool_result(
        "f-bad",
        "yt_fetch_transcript",
        "error NO_TRANSCRIPT: unavailable",
    );
    let success = ModelMessage::tool_result("f-good", "yt_fetch_transcript", "TRANSCRIPT");
    let transcript = vec![
        list,
        listed,
        status_assistant,
        status,
        fetch_assistant,
        failed,
        success,
    ];
    let decision = recipe.next(&params, &transcript);
    let RecipeDecision::ToolCalls(calls) = decision else {
        panic!("expected refreshed collection status")
    };
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].name, "collection_status");

    let refresh_call = ModelMessage::assistant_with_calls("", vec![calls[0].clone()]);
    let refreshed = ModelMessage::tool_result(
        calls[0].id.clone(),
        "collection_status",
        "TOTAL: 2 | REMAINING: 1\nNEXT:\n2. video_id=good attempts=0 title=Good",
    );
    let mut transcript = [transcript, vec![refresh_call, refreshed]].concat();
    transcript.extend(selector_result(
        "select-topic",
        "selector-session",
        r#"{"domain":"youtube","topic":"playlist"}"#,
    ));
    let decision = recipe.next(&params, &transcript);
    let RecipeDecision::ToolCalls(calls) = decision else {
        panic!("good sibling should be saved after refresh")
    };
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].name, "add_to_sources");
    assert_eq!(calls[0].arguments["video_id"], "good");
}

#[test]
fn compacted_transcript_uses_the_durable_ledger_for_the_next_batch() {
    let recipe = PlaylistSourcesRecipe::default();
    let params = json!({
        "url":"https://www.youtube.com/playlist?list=PL1",
        "max_videos":50
    });
    let compacted = ModelMessage::text(
        ModelRole::Assistant,
        "Earlier batches were saved; continue from the durable collection ledger.",
    );
    let listed = ModelMessage::tool_result(
        "list",
        "yt_list_playlist",
        "Playlist listed; collection ledger is durable.",
    );
    let mut next_items = String::from("TOTAL: 50 | REMAINING: 30\nNEXT:\n");
    for position in 21..=50 {
        next_items.push_str(&format!(
            "{position}. video_id=v{position} attempts=0 title=Video {position}\n"
        ));
    }
    let ledger = ModelMessage::tool_result("status-next", "collection_status", next_items);

    let RecipeDecision::ToolCalls(calls) = recipe.next(&params, &[compacted, listed, ledger])
    else {
        panic!("the compacted transcript should select remaining ledger items")
    };
    assert_eq!(calls.len(), 10);
    assert!(calls.iter().all(|call| call.name == "yt_fetch_transcript"));
    assert_eq!(calls.first().unwrap().arguments["video_id"], "v21");
    assert_eq!(calls.last().unwrap().arguments["video_id"], "v30");
}
