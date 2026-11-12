use super::super::*;
use super::*;

#[tokio::test]
async fn ask_streams_configured_provider_over_authenticated_http() {
    let (base, temp_root, server) = fixture(true).await;
    let web_token = admin_web_token::admin_web_token(
        &reqwest::Client::new(),
        base.trim_end_matches("/api/admin"),
    )
    .await;
    let response = reqwest::Client::new()
        .post(format!("{base}/knowledge/ask"))
        .bearer_auth(&web_token)
        .json(&serde_json::json!({"question": "question"}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()["cache-control"], "no-store");
    assert!(
        response.headers()["content-type"]
            .to_str()
            .unwrap()
            .starts_with("text/event-stream")
    );
    let body = response.text().await.unwrap();
    assert!(body.contains("event: delta"));
    assert!(body.contains("Wiki "));
    assert!(body.contains("event: done"));
    server.abort();
    let _ = std::fs::remove_dir_all(temp_root);
}

#[tokio::test]
async fn ask_answers_after_the_sixth_tool_round() {
    let (base, temp_root, server) =
        wiki_fixture(read_rounds(6), "Known evidence from the wiki.").await;
    let web_token = admin_web_token::admin_web_token(
        &reqwest::Client::new(),
        base.trim_end_matches("/api/admin"),
    )
    .await;
    let body = ask_read_more(&base, &web_token).await;
    assert!(body.contains("\"call_id\":\"read-5\""));
    assert!(body.contains("Wiki answer"));
    assert!(body.contains("\"stop_reason\":\"finished\""));
    assert!(!body.contains("step_limit"));
    server.abort();
    let _ = std::fs::remove_dir_all(temp_root);
}

#[tokio::test]
async fn ask_ends_with_step_limit_when_the_model_exceeds_six_tool_rounds() {
    let (base, temp_root, server) =
        wiki_fixture(read_rounds(7), "Known evidence from the wiki.").await;
    let web_token = admin_web_token::admin_web_token(
        &reqwest::Client::new(),
        base.trim_end_matches("/api/admin"),
    )
    .await;
    let body = ask_read_more(&base, &web_token).await;
    assert!(body.contains("\"call_id\":\"read-5\""));
    assert!(!body.contains("\"call_id\":\"read-6\""));
    assert!(body.contains("event: error"));
    assert!(body.contains("\"code\":\"step_limit\""));
    let turns = sse_payloads(&body, "turn");
    let conversation_id = turns[0]["conversation_id"].as_str().unwrap().to_owned();
    let detail: Value = reqwest::Client::new()
        .get(format!("{base}/knowledge/conversations/{conversation_id}"))
        .bearer_auth(&web_token)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(detail["messages"][1]["status"], "failed");
    assert_eq!(detail["messages"][1]["error_code"], "step_limit");
    server.abort();
    let _ = std::fs::remove_dir_all(temp_root);
}

#[tokio::test]
async fn ask_cancels_provider_stream_when_http_client_disconnects() {
    let started = Arc::new(AtomicBool::new(false));
    let dropped = Arc::new(AtomicBool::new(false));
    let provider: Arc<dyn LlmProvider> = Arc::new(PendingLlm {
        started: Arc::clone(&started),
        dropped: Arc::clone(&dropped),
    });
    let (base, temp_root, server) =
        wiki_fixture_with_provider(provider, "Known evidence from the wiki.").await;
    let web_token = admin_web_token::admin_web_token(
        &reqwest::Client::new(),
        base.trim_end_matches("/api/admin"),
    )
    .await;
    let response = reqwest::Client::new()
        .post(format!("{base}/knowledge/ask"))
        .bearer_auth(&web_token)
        .json(&serde_json::json!({
            "question": "Keep waiting"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let mut response_stream = response.bytes_stream();
    let turn_chunk = tokio::time::timeout(Duration::from_secs(2), response_stream.next())
        .await
        .expect("turn event should arrive")
        .unwrap()
        .unwrap();
    let turn = sse_payloads(std::str::from_utf8(&turn_chunk).unwrap(), "turn").remove(0);
    let conversation_id = turn["conversation_id"].as_str().unwrap();
    tokio::time::timeout(Duration::from_secs(2), async {
        while !started.load(Ordering::SeqCst) {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("provider stream should start");
    drop(response_stream);
    tokio::time::timeout(Duration::from_secs(2), async {
        while !dropped.load(Ordering::SeqCst) {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("disconnect should drop provider stream");
    let detail = tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            let detail: Value = reqwest::Client::new()
                .get(format!("{base}/knowledge/conversations/{conversation_id}"))
                .bearer_auth(&web_token)
                .send()
                .await
                .unwrap()
                .json()
                .await
                .unwrap();
            if detail["messages"][1]["status"] == "stopped" {
                break detail;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("stopped answer should be saved");
    assert_eq!(detail["messages"][1]["status"], "stopped");
    assert_eq!(detail["messages"][1]["content"], "partial answer");
    server.abort();
    let _ = std::fs::remove_dir_all(temp_root);
}

#[tokio::test]
async fn only_one_turn_runs_per_conversation_while_other_conversations_can_run() {
    let started = Arc::new(AtomicBool::new(false));
    let dropped = Arc::new(AtomicBool::new(false));
    let provider: Arc<dyn LlmProvider> = Arc::new(PendingLlm {
        started: Arc::clone(&started),
        dropped: Arc::clone(&dropped),
    });
    let (base, temp_root, server) = wiki_fixture_with_provider(provider, "Known evidence.").await;
    let web_token = admin_web_token::admin_web_token(
        &reqwest::Client::new(),
        base.trim_end_matches("/api/admin"),
    )
    .await;
    let client = reqwest::Client::new();
    let mut first = client
        .post(format!("{base}/knowledge/ask"))
        .bearer_auth(&web_token)
        .json(&serde_json::json!({ "question": "first running question" }))
        .send()
        .await
        .unwrap()
        .bytes_stream();
    let first_turn_chunk = tokio::time::timeout(Duration::from_secs(2), first.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let first_turn =
        sse_payloads(std::str::from_utf8(&first_turn_chunk).unwrap(), "turn").remove(0);
    let conversation_id = first_turn["conversation_id"].as_str().unwrap();
    let retry_busy = client
        .post(format!("{base}/knowledge/ask"))
        .bearer_auth(&web_token)
        .json(&serde_json::json!({
            "conversation_id": conversation_id,
            "retry_message_id": first_turn["assistant_message_id"]
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(retry_busy.status(), StatusCode::CONFLICT);
    let retry_busy_body: Value = retry_busy.json().await.unwrap();
    assert_eq!(retry_busy_body["error"]["code"], "wiki_conversation_busy");

    let busy = client
        .post(format!("{base}/knowledge/ask"))
        .bearer_auth(&web_token)
        .json(&serde_json::json!({
            "conversation_id": conversation_id,
            "question": "must not interleave"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(busy.status(), StatusCode::CONFLICT);
    let busy_body: Value = busy.json().await.unwrap();
    assert_eq!(busy_body["error"]["code"], "wiki_conversation_busy");

    let mut second = client
        .post(format!("{base}/knowledge/ask"))
        .bearer_auth(&web_token)
        .json(&serde_json::json!({ "question": "another conversation can run" }))
        .send()
        .await
        .unwrap()
        .bytes_stream();
    let second_turn_chunk = tokio::time::timeout(Duration::from_secs(2), second.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let second_turn =
        sse_payloads(std::str::from_utf8(&second_turn_chunk).unwrap(), "turn").remove(0);
    assert_ne!(second_turn["conversation_id"], conversation_id);
    assert!(started.load(Ordering::SeqCst));
    drop(first);
    drop(second);
    tokio::time::timeout(Duration::from_secs(2), async {
        while !dropped.load(Ordering::SeqCst) {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("closing both streams cancels both providers");
    server.abort();
    let _ = std::fs::remove_dir_all(temp_root);
}

#[tokio::test]
async fn ask_reports_timeout_when_provider_stream_exceeds_total_deadline() {
    let started = Arc::new(AtomicBool::new(false));
    let dropped = Arc::new(AtomicBool::new(false));
    let provider: Arc<dyn LlmProvider> = Arc::new(PendingLlm {
        started: Arc::clone(&started),
        dropped: Arc::clone(&dropped),
    });
    let (base, temp_root, server) =
        wiki_fixture_with_provider(provider, "Known evidence from the wiki.").await;
    let web_token = admin_web_token::admin_web_token(
        &reqwest::Client::new(),
        base.trim_end_matches("/api/admin"),
    )
    .await;
    let response = reqwest::Client::new()
        .post(format!("{base}/knowledge/ask"))
        .bearer_auth(&web_token)
        .json(&serde_json::json!({
            "question": "Wait for the provider"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    tokio::time::pause();
    let body_task = tokio::spawn(response.text());
    for _ in 0..1_000 {
        if started.load(Ordering::SeqCst) {
            break;
        }
        tokio::task::yield_now().await;
    }
    assert!(started.load(Ordering::SeqCst));
    tokio::time::advance(MAX_ASK_TIME + Duration::from_secs(1)).await;
    let body = body_task.await.unwrap().unwrap();
    assert!(body.contains("event: error"));
    assert!(body.contains("\"code\":\"timeout\""));
    assert!(dropped.load(Ordering::SeqCst));
    server.abort();
    let _ = std::fs::remove_dir_all(temp_root);
}

#[tokio::test]
async fn ask_streams_provider_from_managed_runtime_lease() {
    let (base, temp_root, server) = managed_fixture().await;
    let web_token = admin_web_token::admin_web_token(
        &reqwest::Client::new(),
        base.trim_end_matches("/api/admin"),
    )
    .await;
    let response = reqwest::Client::new()
        .post(format!("{base}/knowledge/ask"))
        .bearer_auth(&web_token)
        .json(&serde_json::json!({"question": "question"}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = response.text().await.unwrap();
    assert!(body.contains("event: delta"));
    assert!(body.contains("event: done"));
    server.abort();
    let _ = std::fs::remove_dir_all(temp_root);
}

#[tokio::test]
async fn ask_without_provider_returns_not_configured() {
    let (base, temp_root, server) = fixture(false).await;
    let web_token = admin_web_token::admin_web_token(
        &reqwest::Client::new(),
        base.trim_end_matches("/api/admin"),
    )
    .await;
    let response = reqwest::Client::new()
        .post(format!("{base}/knowledge/ask"))
        .bearer_auth(&web_token)
        .json(&serde_json::json!({"question": "question"}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert!(
        response
            .text()
            .await
            .unwrap()
            .contains("wiki_chat_not_configured")
    );
    server.abort();
    let _ = std::fs::remove_dir_all(temp_root);
}

#[tokio::test]
async fn chat_settings_reject_provider_without_tool_support() {
    let (base, temp_root, server) = fixture_with_tool_support(false, false).await;
    let web_token = admin_web_token::admin_web_token(
        &reqwest::Client::new(),
        base.trim_end_matches("/api/admin"),
    )
    .await;
    let response = reqwest::Client::new()
        .put(format!("{base}/background/settings"))
        .bearer_auth(&web_token)
        .header("if-match", "\"1\"")
        .json(&serde_json::json!({
            "enabled": true,
            "wiki_chat_llm_provider": "config:llm:test"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert_eq!(
        response.json::<serde_json::Value>().await.unwrap()["error"]["code"],
        "provider_capability_mismatch"
    );
    server.abort();
    let _ = std::fs::remove_dir_all(temp_root);
}
