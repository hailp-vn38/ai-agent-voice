use super::super::*;
use super::*;

#[tokio::test]
async fn ask_creates_a_saved_conversation_and_returns_its_message_ids_first() {
    let (base, temp_root, server) = wiki_fixture(
        vec![
            vec![
                LlmEvent::ToolCall(ToolCall {
                    id: "persist-query".into(),
                    name: "wiki_query".into(),
                    arguments: serde_json::json!({"query":"Evidence"}),
                }),
                LlmEvent::Finished,
            ],
            vec![
                LlmEvent::TextDelta(
                    "Evidence supports the answer [[concepts/known.md|Known page]].".into(),
                ),
                LlmEvent::Finished,
            ],
        ],
        "Evidence.",
    )
    .await;
    let web_token = admin_web_token::admin_web_token(
        &reqwest::Client::new(),
        base.trim_end_matches("/api/admin"),
    )
    .await;
    let question = format!("Summarize {}", "evidence ".repeat(12));
    let response = reqwest::Client::new()
        .post(format!("{base}/knowledge/ask"))
        .bearer_auth(&web_token)
        .json(&serde_json::json!({ "question": question }))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = response.text().await.unwrap();
    let event_names = body
        .split("\n\n")
        .filter_map(|frame| frame.lines().find_map(|line| line.strip_prefix("event: ")))
        .collect::<Vec<_>>();
    assert_eq!(event_names.first(), Some(&"turn"));
    assert!(event_names.contains(&"done"));
    let turn = sse_payloads(&body, "turn").remove(0);
    let conversation_id = turn["conversation_id"].as_str().unwrap();
    assert_eq!(turn["user_message_id"].as_str().unwrap().len(), 36);
    assert_eq!(turn["assistant_message_id"].as_str().unwrap().len(), 36);
    let detail = reqwest::Client::new()
        .get(format!("{base}/knowledge/conversations/{conversation_id}"))
        .bearer_auth(&web_token)
        .send()
        .await
        .unwrap();
    assert_eq!(detail.status(), StatusCode::OK);
    let detail: Value = detail.json().await.unwrap();
    assert_eq!(
        detail["title"],
        question.chars().take(80).collect::<String>()
    );
    assert_eq!(detail["messages"][0]["role"], "user");
    assert_eq!(detail["messages"][0]["content"], question);
    assert_eq!(detail["messages"][1]["role"], "assistant");
    assert_eq!(
        detail["messages"][1]["content"],
        "Evidence supports the answer [[concepts/known.md|Known page]]."
    );
    assert_eq!(detail["messages"][1]["status"], "done");
    assert_eq!(
        detail["messages"][1]["citations"],
        serde_json::json!(["wiki/concepts/known.md"])
    );
    assert_eq!(
        detail["messages"][1]["tools"][0]["call_id"],
        "persist-query"
    );
    assert_eq!(detail["messages"][1]["tools"].as_array().unwrap().len(), 1);
    assert_eq!(detail["messages"][1]["tools"][0]["state"], "completed");
    assert!(detail["messages"][1]["graph"].is_object());
    assert_eq!(sse_payloads(&body, "done")[0]["saved"], true);
    let unauthorized = reqwest::Client::new()
        .get(format!("{base}/knowledge/conversations/{conversation_id}"))
        .send()
        .await
        .unwrap();
    assert_eq!(unauthorized.status(), StatusCode::UNAUTHORIZED);
    server.abort();
    let _ = std::fs::remove_dir_all(temp_root);
}

#[tokio::test]
async fn a_running_answer_older_than_two_minutes_is_interrupted_before_a_new_turn() {
    let (base, temp_root, server) = wiki_fixture(Vec::new(), "Known evidence.").await;
    let web_token = admin_web_token::admin_web_token(
        &reqwest::Client::new(),
        base.trim_end_matches("/api/admin"),
    )
    .await;
    let database = test_database(&temp_root).await;
    sqlx::query(
            "INSERT INTO wiki_conversations (id, title, created_at_ms, updated_at_ms) VALUES ('stale-conversation', 'Stale', 1, 1)",
        )
        .execute(database.pool())
        .await
        .unwrap();
    sqlx::query(
            "INSERT INTO wiki_conversation_messages (id, conversation_id, seq, role, content, status, created_at_ms, updated_at_ms) VALUES ('stale-user', 'stale-conversation', 1, 'user', 'Prior question', 'done', 1, 1), ('stale-assistant', 'stale-conversation', 2, 'assistant', 'Partial', 'running', 1, 1)",
        )
        .execute(database.pool())
        .await
        .unwrap();
    drop(database);

    let response = reqwest::Client::new()
        .post(format!("{base}/knowledge/ask"))
        .bearer_auth(&web_token)
        .json(&serde_json::json!({
            "conversation_id": "stale-conversation",
            "question": "Continue after a stale answer"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert!(response.text().await.unwrap().contains("event: done"));
    let detail: Value = reqwest::Client::new()
        .get(format!("{base}/knowledge/conversations/stale-conversation"))
        .bearer_auth(&web_token)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(detail["messages"][1]["status"], "interrupted");
    assert_eq!(detail["messages"][3]["status"], "done");
    server.abort();
    let _ = std::fs::remove_dir_all(temp_root);
}

#[tokio::test]
async fn retry_reuses_the_latest_failed_message_and_its_pinned_files() {
    let provider = Arc::new(RetryLlm {
        calls: AtomicUsize::new(0),
        requests: Mutex::new(Vec::new()),
    });
    let (base, temp_root, server) =
        wiki_fixture_with_provider(provider.clone(), "Pinned evidence.").await;
    let web_token = admin_web_token::admin_web_token(
        &reqwest::Client::new(),
        base.trim_end_matches("/api/admin"),
    )
    .await;
    let client = reqwest::Client::new();
    let first = client
        .post(format!("{base}/knowledge/ask"))
        .bearer_auth(&web_token)
        .json(&serde_json::json!({
            "question": "Explain this page",
            "pinned_paths": ["wiki/concepts/known.md"]
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(first.status(), StatusCode::OK);
    let first_body = first.text().await.unwrap();
    assert!(first_body.contains("\"code\":\"provider_failed\""));
    let turn = sse_payloads(&first_body, "turn").remove(0);
    let conversation_id = turn["conversation_id"].as_str().unwrap();
    let assistant_message_id = turn["assistant_message_id"].as_str().unwrap();

    let retried = client
        .post(format!("{base}/knowledge/ask"))
        .bearer_auth(&web_token)
        .json(&serde_json::json!({
            "conversation_id": conversation_id,
            "retry_message_id": assistant_message_id
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(retried.status(), StatusCode::OK);
    let retried_body = retried.text().await.unwrap();
    assert!(retried_body.contains("event: delta"));
    assert!(retried_body.contains("Recovered answer"));
    let retry_turn = sse_payloads(&retried_body, "turn").remove(0);
    assert_eq!(retry_turn["conversation_id"], conversation_id);
    assert_eq!(retry_turn["user_message_id"], turn["user_message_id"]);
    assert_eq!(retry_turn["assistant_message_id"], assistant_message_id);

    let detail: Value = client
        .get(format!("{base}/knowledge/conversations/{conversation_id}"))
        .bearer_auth(&web_token)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(detail["messages"].as_array().unwrap().len(), 2);
    assert_eq!(
        detail["messages"][0]["pinned_paths"],
        serde_json::json!(["wiki/concepts/known.md"])
    );
    assert_eq!(detail["messages"][1]["id"], assistant_message_id);
    assert_eq!(detail["messages"][1]["status"], "done");
    assert_eq!(detail["messages"][1]["content"], "Recovered answer");
    assert_eq!(provider.calls.load(Ordering::SeqCst), 2);
    let retry_request = provider.requests.lock().unwrap().last().unwrap().clone();
    assert!(retry_request.iter().any(|message| matches!(
        message,
        ChatMessage::User { content } if content == "Explain this page"
    )));

    let retry_done = client
        .post(format!("{base}/knowledge/ask"))
        .bearer_auth(&web_token)
        .json(&serde_json::json!({
            "conversation_id": conversation_id,
            "retry_message_id": assistant_message_id
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(retry_done.status(), StatusCode::BAD_REQUEST);
    assert_eq!(
        retry_done.json::<Value>().await.unwrap()["error"]["code"],
        "invalid_input"
    );
    assert_eq!(provider.calls.load(Ordering::SeqCst), 2);
    server.abort();
    let _ = std::fs::remove_dir_all(temp_root);
}

#[tokio::test]
async fn ask_rejects_a_conversation_at_the_message_limit_without_writing_rows() {
    let (base, temp_root, server) = wiki_fixture(Vec::new(), "Evidence.").await;
    let web_token = admin_web_token::admin_web_token(
        &reqwest::Client::new(),
        base.trim_end_matches("/api/admin"),
    )
    .await;
    let database = test_database(&temp_root).await;
    sqlx::query(
            "INSERT INTO wiki_conversations (id, title, created_at_ms, updated_at_ms) VALUES ('full-conversation', 'Full', 1, 1)",
        )
        .execute(database.pool())
        .await
        .unwrap();
    for seq in 0..50_i64 {
        let user_seq = seq * 2 + 1;
        let assistant_seq = user_seq + 1;
        sqlx::query(
                "INSERT INTO wiki_conversation_messages (id, conversation_id, seq, role, content, status, created_at_ms, updated_at_ms) VALUES (?, 'full-conversation', ?, 'user', 'question', 'done', 1, 1)",
            )
            .bind(format!("user-{seq}"))
            .bind(user_seq)
            .execute(database.pool())
            .await
            .unwrap();
        sqlx::query(
                "INSERT INTO wiki_conversation_messages (id, conversation_id, seq, role, content, status, created_at_ms, updated_at_ms) VALUES (?, 'full-conversation', ?, 'assistant', 'answer', 'done', 1, 1)",
            )
            .bind(format!("assistant-{seq}"))
            .bind(assistant_seq)
            .execute(database.pool())
            .await
            .unwrap();
    }
    drop(database);

    let response = reqwest::Client::new()
        .post(format!("{base}/knowledge/ask"))
        .bearer_auth(&web_token)
        .json(&serde_json::json!({
            "conversation_id": "full-conversation",
            "question": "one more question"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CONFLICT);
    let body: Value = response.json().await.unwrap();
    assert_eq!(body["error"]["code"], "wiki_conversation_full");

    let database = test_database(&temp_root).await;
    let message_count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM wiki_conversation_messages WHERE conversation_id = 'full-conversation'",
        )
        .fetch_one(database.pool())
        .await
        .unwrap();
    assert_eq!(message_count, 100);
    drop(database);
    server.abort();
    let _ = std::fs::remove_dir_all(temp_root);
}

#[tokio::test]
async fn follow_up_history_comes_from_saved_messages() {
    let fake = Arc::new(ScriptedLlm::new(true, Vec::new()));
    let (base, temp_root, server) = wiki_fixture_with_provider(fake.clone(), "Evidence.").await;
    let web_token = admin_web_token::admin_web_token(
        &reqwest::Client::new(),
        base.trim_end_matches("/api/admin"),
    )
    .await;
    let client = reqwest::Client::new();
    let first = client
        .post(format!("{base}/knowledge/ask"))
        .bearer_auth(&web_token)
        .json(&serde_json::json!({ "question": "first saved question" }))
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    let turn = sse_payloads(&first, "turn").remove(0);
    let conversation_id = turn["conversation_id"].as_str().unwrap();

    let second = client
        .post(format!("{base}/knowledge/ask"))
        .bearer_auth(&web_token)
        .json(&serde_json::json!({
            "conversation_id": conversation_id,
            "question": "follow-up question"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(second.status(), StatusCode::OK);
    let _ = second.text().await.unwrap();
    let sent = fake.last_messages.lock().unwrap().clone().unwrap();
    let conversational = sent
        .iter()
        .filter_map(|message| match message {
            ChatMessage::User { content } => Some(("user", content.as_str())),
            ChatMessage::AssistantText { content } => Some(("assistant", content.as_str())),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(
        conversational,
        [
            ("user", "first saved question"),
            ("assistant", "Wiki answer"),
            ("user", "follow-up question")
        ]
    );
    server.abort();
    let _ = std::fs::remove_dir_all(temp_root);
}

#[tokio::test]
async fn database_history_is_bounded_to_twelve_messages_and_sixteen_kibibytes() {
    let responses = (0..7)
        .map(|index| {
            vec![
                LlmEvent::TextDelta(format!("answer-{index} {}", "é".repeat(3_000))),
                LlmEvent::Finished,
            ]
        })
        .collect();
    let fake = Arc::new(ScriptedLlm::new(true, responses));
    let (base, temp_root, server) = wiki_fixture_with_provider(fake.clone(), "Evidence.").await;
    let web_token = admin_web_token::admin_web_token(
        &reqwest::Client::new(),
        base.trim_end_matches("/api/admin"),
    )
    .await;
    let client = reqwest::Client::new();
    let mut conversation_id = None;
    for index in 0..7 {
        let mut body = serde_json::json!({ "question": format!("question-{index}") });
        if let Some(id) = conversation_id.as_deref() {
            body["conversation_id"] = json!(id);
        }
        let response = client
            .post(format!("{base}/knowledge/ask"))
            .bearer_auth(&web_token)
            .json(&body)
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let events = response.text().await.unwrap();
        if conversation_id.is_none() {
            conversation_id = Some(
                sse_payloads(&events, "turn")[0]["conversation_id"]
                    .as_str()
                    .unwrap()
                    .to_owned(),
            );
        }
    }
    let sent = fake.last_messages.lock().unwrap().clone().unwrap();
    let conversational = sent
        .iter()
        .filter_map(|message| match message {
            ChatMessage::User { content } => Some(("user", content.as_str())),
            ChatMessage::AssistantText { content } => Some(("assistant", content.as_str())),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert!(conversational.len() <= 12);
    assert_eq!(conversational.first().unwrap().0, "user");
    assert_eq!(conversational.last().unwrap().1, "question-6");
    let history_bytes = conversational
        .iter()
        .map(|(_, text)| text.len())
        .sum::<usize>();
    assert!(history_bytes <= 16 * 1024);
    server.abort();
    let _ = std::fs::remove_dir_all(temp_root);
}

#[tokio::test]
async fn ask_rejects_browser_history_and_invalid_question_retry_combinations() {
    let (base, temp_root, server) = fixture(true).await;
    let web_token = admin_web_token::admin_web_token(
        &reqwest::Client::new(),
        base.trim_end_matches("/api/admin"),
    )
    .await;
    let client = reqwest::Client::new();
    for body in [
        serde_json::json!({ "messages": [{"role":"user","content":"untrusted"}] }),
        serde_json::json!({ "question": "question", "retry_message_id": "message" }),
        serde_json::json!({ "question": "question", "extra": true }),
        serde_json::json!({}),
        serde_json::json!({ "question": "x".repeat(MAX_HISTORY_BYTES + 1) }),
    ] {
        let response = client
            .post(format!("{base}/knowledge/ask"))
            .bearer_auth(&web_token)
            .json(&body)
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        assert_eq!(
            response.json::<Value>().await.unwrap()["error"]["code"],
            "invalid_input"
        );
    }

    let missing = client
        .get(format!("{base}/knowledge/conversations/does-not-exist"))
        .bearer_auth(&web_token)
        .send()
        .await
        .unwrap();
    assert_eq!(missing.status(), StatusCode::NOT_FOUND);
    assert_eq!(missing.headers()["cache-control"], "no-store");
    assert_eq!(
        missing.json::<Value>().await.unwrap()["error"]["code"],
        "wiki_conversation_not_found"
    );
    server.abort();
    let _ = std::fs::remove_dir_all(temp_root);
}

#[tokio::test]
async fn ask_without_a_database_rejects_every_bearer() {
    let (base, temp_root, server) = fixture_without_database().await;
    // Without a database there is no admin_auth row, so no Admin Web Token can be issued and no
    // static credential exists to take its place: the request never reaches the ask handler.
    let response = reqwest::Client::new()
        .post(format!("{base}/knowledge/ask"))
        .bearer_auth("wiki-chat-test-token")
        .json(&serde_json::json!({ "question": "not saved" }))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    server.abort();
    let _ = std::fs::remove_dir_all(temp_root);
}
