use super::super::*;
use super::*;

#[test]
fn generated_title_summarizes_recipe_parameters_without_copying_urls() {
    let playlist = PlaylistParams::from_value(&json!({
        "url": "https://www.youtube.com/playlist?list=private-token-value",
        "max_videos": 18,
        "domain": "rust",
        "ingest_to_wiki": true
    }))
    .title();
    assert_eq!(
        playlist,
        "Thu thập playlist · tối đa 18 video · rust · Wiki"
    );
    assert!(!playlist.contains("youtube.com"));
    assert!(!playlist.contains("private-token-value"));

    let list = PlaylistParams::from_value(&json!({
        "video_urls": [
            "https://youtu.be/secret-id-one",
            "https://youtu.be/secret-id-two"
        ],
        "domain": AGENT_AUTO_DOMAIN,
        "ingest_to_wiki": false
    }))
    .title();
    assert_eq!(list, "Thu thập 2 video lẻ");
    assert!(!list.contains("secret-id"));
}

#[test]
fn recipe_accepts_either_playlist_or_video_url_list_but_not_both() {
    let recipe = PlaylistSourcesRecipe::default();
    let playlist = recipe
        .validate_params(&json!({"url":"https://www.youtube.com/playlist?list=abc"}))
        .expect("playlist mode should validate");
    assert_eq!(playlist["ingest_to_wiki"], false);

    let videos = recipe
        .validate_params(&json!({
            "video_urls": ["https://youtu.be/one", "https://www.youtube.com/watch?v=two"]
        }))
        .expect("video list mode should validate");
    assert_eq!(videos["video_urls"].as_array().unwrap().len(), 2);

    let both = recipe
        .validate_params(&json!({
            "url":"https://www.youtube.com/playlist?list=abc",
            "video_urls":["https://youtu.be/one"]
        }))
        .unwrap_err();
    assert_eq!(both.validation_path, "params");

    let neither = recipe.validate_params(&json!({})).unwrap_err();
    assert_eq!(neither.validation_path, "params");
}

#[test]
fn video_url_list_is_collected_through_the_existing_playlist_host_tool() {
    let params = json!({
        "video_urls": ["https://youtu.be/one", "https://youtu.be/two"]
    });
    let RecipeDecision::ToolCalls(calls) = PlaylistSourcesRecipe::default().next(&params, &[])
    else {
        panic!("expected video URLs to be listed through the collection tool")
    };
    assert_eq!(calls.len(), 2);
    assert!(calls.iter().all(|call| call.name == "yt_list_playlist"));
    assert_eq!(calls[0].arguments["url"], "https://youtu.be/one");
    assert_eq!(calls[1].arguments["url"], "https://youtu.be/two");

    let mut transcript = Vec::new();
    for call in calls {
        transcript.push(ModelMessage::assistant_with_calls("", vec![call.clone()]));
        transcript.push(ModelMessage::tool_result(
            &call.id,
            "yt_list_playlist",
            "listed",
        ));
    }
    let RecipeDecision::ToolCalls(status_calls) =
        PlaylistSourcesRecipe::default().next(&params, &transcript)
    else {
        panic!("expected both URL collections to be read")
    };
    assert_eq!(status_calls.len(), 2);
    assert!(
        status_calls
            .iter()
            .all(|call| call.name == "collection_status")
    );
}
