use super::super::*;
use super::*;

#[tokio::test]
async fn ask_emits_query_graph_before_answer_and_only_cites_retrieved_pages() {
    let (base, temp_root, server) = wiki_fixture(vec![
            vec![
                LlmEvent::ToolCall(ToolCall {
                    id: "query-1".into(),
                    name: "wiki_query".into(),
                    arguments: serde_json::json!({
                        "query": "Known evidence",
                        "collections": ["wiki", "sources"],
                        "limit": 12
                    }),
                }),
                LlmEvent::Finished,
            ],
            vec![
                LlmEvent::TextDelta(
                    "Evidence: [[concepts/known.md|Known page]]; invented: [[concepts/invented.md]]."
                        .into(),
                ),
                LlmEvent::Finished,
            ],
        ], "Known evidence from the wiki.")
        .await;
    let web_token = admin_web_token::admin_web_token(
        &reqwest::Client::new(),
        base.trim_end_matches("/api/admin"),
    )
    .await;
    let response = reqwest::Client::new()
        .post(format!("{base}/knowledge/ask"))
        .bearer_auth(&web_token)
        .json(&serde_json::json!({
            "question": "What is known?"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = response.text().await.unwrap();
    assert!(body.find("event: tool").unwrap() < body.find("event: graph").unwrap());
    assert!(body.find("event: graph").unwrap() < body.find("event: delta").unwrap());
    assert!(body.rfind("event: graph").unwrap() < body.find("event: done").unwrap());
    let graphs = sse_payloads(&body, "graph");
    assert!(graphs.len() >= 2);
    let first_node = graphs[0]["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|node| node["id"] == "wiki/concepts/known.md")
        .unwrap();
    assert_eq!(first_node["tier"], "retrieved");
    assert!(first_node["score"].as_f64().unwrap_or_default() > 0.0);
    assert_eq!(graphs[0]["revision"], 1);
    let final_node = graphs.last().unwrap()["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|node| node["id"] == "wiki/concepts/known.md")
        .unwrap();
    assert_eq!(final_node["tier"], "cited");
    assert_eq!(graphs.last().unwrap()["revision"], 2);
    assert_eq!(final_node["score"], first_node["score"]);
    let done = sse_payloads(&body, "done");
    assert_eq!(
        done[0]["citations"],
        serde_json::json!(["wiki/concepts/known.md"])
    );
    server.abort();
    let _ = std::fs::remove_dir_all(temp_root);
}

#[tokio::test]
async fn ask_keeps_all_multi_get_evidence_when_tool_result_is_truncated_for_provider() {
    let long_content = "Evidence paragraph. ".repeat(350);
    let (base, temp_root, server) = wiki_fixture(
        vec![
            vec![
                LlmEvent::ToolCall(ToolCall {
                    id: "read-1".into(),
                    name: "wiki_multi_get".into(),
                    arguments: serde_json::json!({"pattern":"concepts/*.md", "max_bytes":65536}),
                }),
                LlmEvent::Finished,
            ],
            vec![
                LlmEvent::TextDelta(
                    "The page supports this: [[concepts/known.md|Known page]].".into(),
                ),
                LlmEvent::Finished,
            ],
        ],
        &long_content,
    )
    .await;
    let web_token = admin_web_token::admin_web_token(
        &reqwest::Client::new(),
        base.trim_end_matches("/api/admin"),
    )
    .await;
    let response = reqwest::Client::new()
        .post(format!("{base}/knowledge/ask"))
        .bearer_auth(&web_token)
        .json(&serde_json::json!({
            "question": "Read the page"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = response.text().await.unwrap();
    let completed_tool = sse_payloads(&body, "tool")
        .into_iter()
        .find(|tool| tool["state"] == "completed")
        .expect("completed wiki_multi_get tool event");
    assert_eq!(
        completed_tool["pages"],
        serde_json::json!([
            "wiki/concepts/known.md",
            "wiki/concepts/related-a.md",
            "wiki/concepts/related-b.md"
        ])
    );
    let graphs = sse_payloads(&body, "graph");
    let final_node = graphs.last().unwrap()["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|node| node["id"] == "wiki/concepts/known.md")
        .unwrap();
    assert_eq!(final_node["tier"], "cited");
    assert_eq!(graphs.last().unwrap()["revision"], 2);
    assert_eq!(final_node["score"], Value::Null);
    let read_paths: Vec<_> = graphs[0]["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|node| node["tier"] == "read")
        .filter_map(|node| node["id"].as_str())
        .collect();
    assert_eq!(
        read_paths,
        [
            "wiki/concepts/known.md",
            "wiki/concepts/related-a.md",
            "wiki/concepts/related-b.md"
        ]
    );
    let done = sse_payloads(&body, "done");
    assert_eq!(
        done[0]["citations"],
        serde_json::json!(["wiki/concepts/known.md"])
    );
    assert!(body.contains("wiki/concepts/related-a.md"));
    assert!(body.contains("wiki/concepts/related-b.md"));
    server.abort();
    let _ = std::fs::remove_dir_all(temp_root);
}

#[tokio::test]
async fn ask_refuses_write_tool_and_leaves_wiki_unchanged() {
    let (base, temp_root, server) = wiki_fixture(
        vec![
            vec![
                LlmEvent::ToolCall(ToolCall {
                    id: "write-1".into(),
                    name: "wiki_delete_file".into(),
                    arguments: serde_json::json!({
                        "scope": "wiki",
                        "rel_path": "concepts/known.md"
                    }),
                }),
                LlmEvent::Finished,
            ],
            vec![
                LlmEvent::TextDelta("I could not run that tool.".into()),
                LlmEvent::Finished,
            ],
        ],
        "Known evidence from the wiki.",
    )
    .await;
    let web_token = admin_web_token::admin_web_token(
        &reqwest::Client::new(),
        base.trim_end_matches("/api/admin"),
    )
    .await;
    let page_path = temp_root.join("vault/wiki/concepts/known.md");
    let before = std::fs::read(&page_path).unwrap();
    let response = reqwest::Client::new()
        .post(format!("{base}/knowledge/ask"))
        .bearer_auth(&web_token)
        .json(&serde_json::json!({
            "question": "Delete the page"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = response.text().await.unwrap();
    assert!(body.contains("wiki_delete_file"));
    assert!(body.contains("\"name\":\"wiki_delete_file\""));
    assert!(body.contains("\"state\":\"failed\""));
    assert_eq!(std::fs::read(&page_path).unwrap(), before);
    server.abort();
    let _ = std::fs::remove_dir_all(temp_root);
}

#[tokio::test]
async fn ask_lets_the_model_correct_rejected_tool_calls() {
    let call = |id: &str, name: &str, arguments: Value| {
        vec![
            LlmEvent::ToolCall(ToolCall {
                id: id.into(),
                name: name.into(),
                arguments,
            }),
            LlmEvent::Finished,
        ]
    };
    let fake = Arc::new(ScriptedLlm::new(
        true,
        vec![
            call(
                "bad-args",
                "wiki_query",
                serde_json::json!({"searches": ["Known"]}),
            ),
            call(
                "missing",
                "wiki_get",
                serde_json::json!({"file": "concepts/missing.md"}),
            ),
            call(
                "retry",
                "wiki_query",
                serde_json::json!({"query": "Known evidence"}),
            ),
        ],
    ));
    let (base, temp_root, server) =
        wiki_fixture_with_provider(fake.clone(), "Known evidence from the wiki.").await;
    let web_token = admin_web_token::admin_web_token(
        &reqwest::Client::new(),
        base.trim_end_matches("/api/admin"),
    )
    .await;

    let body = ask_read_more(&base, &web_token).await;

    assert!(body.contains("\"stop_reason\":\"finished\""));
    let (tools, results) = fake.last_request.lock().unwrap().clone().unwrap();
    assert_eq!(results.len(), 3);
    assert!(results[0].contains("\"error\":\"invalid_arguments\""));
    assert!(results[0].contains("searches"));
    assert!(results[1].contains("\"error\":\"not_found\""));
    assert!(results[1].contains("concepts/missing.md"));
    assert!(results[2].contains("Known page"));
    assert!(!results.iter().any(|result| {
        result.contains("knowledge_unavailable") || result.contains(temp_root.to_str().unwrap())
    }));
    let query_properties = &tools[0].parameters["properties"];
    assert!(query_properties.get("query").is_some());
    assert!(query_properties.get("filter").is_none());
    server.abort();
    let _ = std::fs::remove_dir_all(temp_root);
}

#[test]
fn query_tool_result_lists_every_hit_within_the_bound() {
    let results: Vec<_> = (0..10)
        .map(|index| {
            serde_json::json!({
                "rel_path": format!("wiki/page-{index}.md"),
                "title": format!("Page {index}"),
                "score": 0.5,
                "snippet": format!("\"evidence {index}\"\n{}", "x\n".repeat(3_000)),
                "outgoing_links": vec!["wiki/other.md"; 200],
            })
        })
        .collect();
    let outcome = LlmToolOutcome {
        content: String::new(),
        metadata: serde_json::json!({"count": 10, "results": results}),
        progress_message: None,
    };

    let bounded = tool_result_for_llm("wiki_query", outcome, MAX_TOOL_RESULT_CHARS);

    assert!(serde_json::to_string(&bounded).unwrap().len() <= MAX_TOOL_RESULT_CHARS + 32);
    assert_eq!(bounded["untrusted_data"], true);
    let content = bounded["content"].as_str().unwrap();
    for index in 0..10 {
        assert!(content.contains(&format!("wiki/page-{index}.md | Page {index}")));
        assert!(content.contains(&format!("\"evidence {index}\"")));
    }
}

#[tokio::test]
async fn ask_graph_shows_only_the_latest_tool_call() {
    let get = |id: &str, file: &str| LlmEvent::ToolCall(ToolCall {
        id: id.into(),
        name: "wiki_get".into(),
        arguments: serde_json::json!({ "file": file }),
    });
    let (base, temp_root, server) = wiki_fixture(
        vec![
            vec![get("read-a", "concepts/related-a.md"), LlmEvent::Finished],
            vec![get("read-b", "concepts/related-b.md"), LlmEvent::Finished],
            vec![
                LlmEvent::TextDelta("See [[concepts/related-a.md]] and [[concepts/related-b.md]].".into()),
                LlmEvent::Finished,
            ],
        ],
        "Known evidence from the wiki.",
    )
    .await;
    let web_token = admin_web_token::admin_web_token(
        &reqwest::Client::new(),
        base.trim_end_matches("/api/admin"),
    )
    .await;
    let response = reqwest::Client::new()
        .post(format!("{base}/knowledge/ask"))
        .bearer_auth(&web_token)
        .json(&serde_json::json!({ "question": "Read both" }))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = response.text().await.unwrap();
    let graphs = sse_payloads(&body, "graph");
    let evidence_of = |graph: &Value| -> Vec<String> {
        graph["nodes"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|node| node["tier"] != "context")
            .filter_map(|node| node["id"].as_str().map(str::to_owned))
            .collect()
    };
    assert_eq!(evidence_of(&graphs[0]), ["wiki/concepts/related-a.md"]);
    assert_eq!(evidence_of(&graphs[1]), ["wiki/concepts/related-b.md"]);
    let last = graphs.last().unwrap();
    assert_eq!(evidence_of(last), ["wiki/concepts/related-b.md"]);
    assert_eq!(last["nodes"][0]["tier"], "cited");
    // Citations still reach pages from earlier calls.
    let done = sse_payloads(&body, "done");
    assert_eq!(
        done[0]["citations"],
        serde_json::json!(["wiki/concepts/related-a.md", "wiki/concepts/related-b.md"])
    );
    server.abort();
    let _ = std::fs::remove_dir_all(temp_root);
}
