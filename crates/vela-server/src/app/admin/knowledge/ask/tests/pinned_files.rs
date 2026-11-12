use super::super::*;
use super::*;

#[tokio::test]
async fn ask_reads_pinned_files_first_and_lets_the_answer_cite_them() {
    let fake = Arc::new(ScriptedLlm::new(
        true,
        vec![vec![
            LlmEvent::TextDelta("From the file: [[concepts/known.md|Known page]].".into()),
            LlmEvent::Finished,
        ]],
    ));
    let (base, temp_root, server) =
        wiki_fixture_with_provider(fake.clone(), "Known evidence from the wiki.").await;
    let web_token = admin_web_token::admin_web_token(
        &reqwest::Client::new(),
        base.trim_end_matches("/api/admin"),
    )
    .await;

    let body = reqwest::Client::new()
        .post(format!("{base}/knowledge/ask"))
        .bearer_auth(&web_token)
        .json(&serde_json::json!({
            "question": "Summarize this file",
            "pinned_paths": ["wiki/concepts/known.md"]
        }))
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();

    let (_, results) = fake.last_request.lock().unwrap().clone().unwrap();
    assert_eq!(results.len(), 1);
    assert!(results[0].contains("Known evidence from the wiki."));
    let completed = sse_payloads(&body, "tool")
        .into_iter()
        .find(|tool| tool["call_id"] == "pinned-0" && tool["state"] == "completed")
        .expect("pinned read is shown as a completed tool");
    assert_eq!(completed["name"], "wiki_get");
    assert_eq!(
        completed["pages"],
        serde_json::json!(["wiki/concepts/known.md"])
    );
    assert_eq!(
        sse_payloads(&body, "done")[0]["citations"],
        serde_json::json!(["wiki/concepts/known.md"])
    );
    server.abort();
    let _ = std::fs::remove_dir_all(temp_root);
}

#[tokio::test]
async fn ask_tells_the_model_when_a_pinned_file_is_gone() {
    let fake = Arc::new(ScriptedLlm::new(true, Vec::new()));
    let (base, temp_root, server) =
        wiki_fixture_with_provider(fake.clone(), "Known evidence from the wiki.").await;
    let web_token = admin_web_token::admin_web_token(
        &reqwest::Client::new(),
        base.trim_end_matches("/api/admin"),
    )
    .await;

    let body = reqwest::Client::new()
        .post(format!("{base}/knowledge/ask"))
        .bearer_auth(&web_token)
        .json(&serde_json::json!({
            "question": "Summarize this file",
            "pinned_paths": ["wiki/concepts/deleted.md"]
        }))
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();

    let (_, results) = fake.last_request.lock().unwrap().clone().unwrap();
    assert!(results[0].contains("\"error\":\"not_found\""));
    assert!(body.contains("\"state\":\"failed\""));
    assert!(body.contains("\"stop_reason\":\"finished\""));
    server.abort();
    let _ = std::fs::remove_dir_all(temp_root);
}

#[tokio::test]
async fn pinned_read_does_not_use_up_the_model_tool_rounds() {
    let (base, temp_root, server) =
        wiki_fixture(read_rounds(6), "Known evidence from the wiki.").await;
    let web_token = admin_web_token::admin_web_token(
        &reqwest::Client::new(),
        base.trim_end_matches("/api/admin"),
    )
    .await;
    let body = reqwest::Client::new()
        .post(format!("{base}/knowledge/ask"))
        .bearer_auth(&web_token)
        .json(&serde_json::json!({
            "question": "Read more",
            "pinned_paths": ["wiki/concepts/known.md"]
        }))
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    assert!(body.contains("\"call_id\":\"read-5\""));
    assert!(body.contains("\"stop_reason\":\"finished\""));
    server.abort();
    let _ = std::fs::remove_dir_all(temp_root);
}

#[test]
fn pinned_paths_must_be_vault_pages_within_the_limit() {
    for bad in [
        "../secret.md",
        "wiki/../../secret.md",
        "/etc/passwd",
        "logs/app.md",
        "wiki\\known.md",
    ] {
        assert!(pinned_read_calls(vec![bad.to_owned()]).is_err(), "{bad}");
    }
    let too_many = (0..=MAX_PINNED_FILES)
        .map(|index| format!("wiki/page-{index}.md"))
        .collect();
    assert!(pinned_read_calls(too_many).is_err());
    let calls = pinned_read_calls(vec![
        "wiki/a.md".to_owned(),
        "sources/b.md".to_owned(),
        "wiki/a.md".to_owned(),
    ])
    .unwrap();
    assert_eq!(calls.len(), 2);
    assert_eq!(calls[1].arguments["file"], "sources/b.md");
}
