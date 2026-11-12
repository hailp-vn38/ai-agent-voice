use super::super::*;

#[tokio::test]
async fn first_assignment_to_an_agent_becomes_its_enabled_default() {
    let (base, task) = server(true).await;
    let client = Client::new();
    let web_token = admin_web_token(&client, &base).await;
    let auth = web_token.as_str();
    let agents = format!("{base}/api/admin/agents");
    let templates = format!("{base}/api/admin/templates");
    let providers = format!("{base}/api/admin/providers");

    for (path, body) in [
        (
            &agents,
            serde_json::json!({"key":"kitchen","name":"Kitchen"}),
        ),
        (
            &templates,
            serde_json::json!({"key":"quiet","name":"Quiet","language":"vi-VN","prompt":"Nói ngắn gọn"}),
        ),
        (
            &templates,
            serde_json::json!({"key":"loud","name":"Loud","language":"vi-VN","prompt":"Nói to"}),
        ),
    ] {
        assert_eq!(
            client
                .post(path)
                .bearer_auth(auth)
                .json(&body)
                .send()
                .await
                .unwrap()
                .status(),
            StatusCode::CREATED
        );
    }

    // A first assignment takes the default slot unless an explicit core binding is broken;
    // absent core slots fall back to the server-default providers.
    let mut generated = Vec::new();
    for (name, kind, adapter, config_json) in [
        ("Main VAD", "vad", "silero_onnx", serde_json::json!({})),
        (
            "Main ASR",
            "asr",
            "zipformer_sherpa",
            serde_json::json!({"decoding_method":"greedy_search"}),
        ),
        (
            "Main LLM",
            "llm",
            "openai",
            serde_json::json!({"base_url":"https://example.test/v1","model":"test","max_tokens":8}),
        ),
        (
            "Main TTS",
            "tts",
            "zerotts_onnx",
            serde_json::json!({"voice":"maichi"}),
        ),
    ] {
        let key = create_provider(
            &client,
            &providers,
            serde_json::json!({
                "name":name,"type":kind,"adapter":adapter,"config_json":config_json
            }),
        )
        .await;
        generated.push((kind, key));
    }

    for template_key in ["quiet", "loud"] {
        for (revision, (kind, provider_key)) in (1..).zip(&generated) {
            assert_eq!(
                client
                    .put(format!("{templates}/{template_key}/providers/{kind}"))
                    .bearer_auth(auth)
                    .header("if-match", format!("\"{revision}\""))
                    .json(&serde_json::json!({"provider_key":provider_key}))
                    .send()
                    .await
                    .unwrap()
                    .status(),
                StatusCode::OK
            );
        }
    }

    let assign = |template_key: &'static str, if_match: &'static str| {
        let client = client.clone();
        let url = format!("{agents}/kitchen/templates/{template_key}");
        async move {
            client
                .put(url)
                .bearer_auth(auth)
                .header("if-match", if_match)
                .send()
                .await
                .unwrap()
                .status()
        }
    };

    // The first assignment has no default to preserve, so it must take the default slot.
    assert_eq!(assign("quiet", "\"1\"").await, StatusCode::OK);
    let after_first: serde_json::Value = client
        .get(format!("{agents}/kitchen/templates"))
        .bearer_auth(auth)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let default_count = |items: &serde_json::Value| {
        items["items"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|item| item["is_default"] == true)
            .count()
    };
    assert_eq!(
        default_count(&after_first),
        1,
        "the first assignment must become the default, got {after_first}"
    );
    let find = |items: &serde_json::Value, key: &str| {
        items["items"]
            .as_array()
            .unwrap()
            .iter()
            .find(|item| item["key"] == key)
            .unwrap_or_else(|| panic!("{key} missing from {items}"))
            .clone()
    };
    assert_eq!(find(&after_first, "quiet")["is_default"], true);

    // A second assignment is a switch candidate and must not steal the default slot.
    assert_eq!(assign("loud", "\"2\"").await, StatusCode::OK);
    let after_second: serde_json::Value = client
        .get(format!("{agents}/kitchen/templates"))
        .bearer_auth(auth)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(
        default_count(&after_second),
        1,
        "exactly one enabled default must remain, got {after_second}"
    );
    assert_eq!(find(&after_second, "quiet")["is_default"], true);
    assert_eq!(find(&after_second, "loud")["is_default"], false);
    task.abort();
}

#[tokio::test]
async fn first_assignment_with_a_partial_core_becomes_the_default() {
    let (base, task) = server(true).await;
    let client = Client::new();
    let web_token = admin_web_token(&client, &base).await;
    let auth = web_token.as_str();
    let agents = format!("{base}/api/admin/agents");
    let templates = format!("{base}/api/admin/templates");
    let providers = format!("{base}/api/admin/providers");

    for (path, body) in [
        (
            &agents,
            serde_json::json!({"key":"kitchen","name":"Kitchen"}),
        ),
        (
            &templates,
            serde_json::json!({"key":"partial","name":"Partial","language":"vi-VN","prompt":"Nói ngắn gọn"}),
        ),
    ] {
        assert_eq!(
            client
                .post(path)
                .bearer_auth(auth)
                .json(&body)
                .send()
                .await
                .unwrap()
                .status(),
            StatusCode::CREATED
        );
    }

    let llm = create_provider(
        &client,
        &providers,
        serde_json::json!({
            "name":"Main LLM", "type":"llm", "adapter":"openai",
            "config_json":{"base_url":"https://example.test/v1","model":"test","max_tokens":8}
        }),
    )
    .await;
    let speaker_provider = client
        .post(&providers)
        .bearer_auth(auth)
        .json(&serde_json::json!({"type":"speaker","adapter":"campplus_sherpa","name":"Voice","config_json":{}}))
        .send()
        .await
        .unwrap();
    assert_eq!(speaker_provider.status(), StatusCode::BAD_REQUEST);
    assert_eq!(
        speaker_provider.json::<serde_json::Value>().await.unwrap()["error"]["code"],
        "validation_failed"
    );
    let speaker_slot = client
        .put(format!("{templates}/partial/providers/speaker"))
        .bearer_auth(auth)
        .header("if-match", "\"1\"")
        .json(&serde_json::json!({"provider_key":llm}))
        .send()
        .await
        .unwrap();
    assert_eq!(speaker_slot.status(), StatusCode::BAD_REQUEST);
    assert_eq!(
        client
            .put(format!("{templates}/partial/providers/llm"))
            .bearer_auth(auth)
            .header("if-match", "\"1\"")
            .json(&serde_json::json!({"provider_key":llm}))
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::OK
    );

    assert_eq!(
        client
            .put(format!("{agents}/kitchen/templates/partial"))
            .bearer_auth(auth)
            .header("if-match", "\"1\"")
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::OK
    );
    let assignments: serde_json::Value = client
        .get(format!("{agents}/kitchen/templates"))
        .bearer_auth(auth)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let partial = assignments["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["key"] == "partial")
        .unwrap();
    assert_eq!(
        partial["is_default"], true,
        "a partial core template must still become the default: {assignments}"
    );
    task.abort();
}
