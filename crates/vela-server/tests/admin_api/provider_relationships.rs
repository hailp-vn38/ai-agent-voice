use super::super::*;

#[tokio::test]
async fn admin_relationship_reads_and_unlinks_are_revisioned_public_contracts() {
    let (base, task) = server(true).await;
    let client = Client::new();
    let web_token = admin_web_token(&client, &base).await;
    let auth = web_token.as_str();
    let agents = format!("{base}/api/admin/agents");
    let templates = format!("{base}/api/admin/templates");
    let providers = format!("{base}/api/admin/providers");

    assert_eq!(
        client
            .post(&agents)
            .bearer_auth(auth)
            .json(&serde_json::json!({"key":"kitchen","name":"Kitchen"}))
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::CREATED
    );
    assert_eq!(
        client
            .post(&templates)
            .bearer_auth(auth)
            .json(&serde_json::json!({
                "key":"quiet","name":"Quiet","language":"vi-VN","prompt":"Nói ngắn gọn"
            }))
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::CREATED
    );

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
        assert!(key.starts_with(kind));
        generated.push((kind, key));
    }

    for (template_revision, (kind, provider_key)) in (1..).zip(&generated) {
        let response = client
            .put(format!("{templates}/quiet/providers/{kind}"))
            .bearer_auth(auth)
            .header("if-match", format!("\"{template_revision}\""))
            .json(&serde_json::json!({"provider_key":provider_key}))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
    }

    let assigned = client
        .put(format!("{agents}/kitchen/templates/quiet"))
        .bearer_auth(auth)
        .header("if-match", "\"1\"")
        .send()
        .await
        .unwrap();
    assert_eq!(assigned.status(), StatusCode::OK);

    let agent_templates = client
        .get(format!("{agents}/kitchen/templates"))
        .bearer_auth(auth)
        .send()
        .await
        .unwrap()
        .json::<serde_json::Value>()
        .await
        .unwrap();
    assert_eq!(agent_templates["items"][0]["key"], "quiet");
    assert_eq!(agent_templates["items"][0]["is_default"], true);
    assert_eq!(agent_templates["revision"], 2);

    let template_agents = client
        .get(format!("{templates}/quiet/agents"))
        .bearer_auth(auth)
        .send()
        .await
        .unwrap()
        .json::<serde_json::Value>()
        .await
        .unwrap();
    assert_eq!(template_agents["items"][0]["key"], "kitchen");
    assert_eq!(template_agents["items"][0]["is_default"], true);
    assert_eq!(template_agents["revision"], 5);
    assert_eq!(template_agents["total"], 1);

    let bindings = client
        .get(format!("{templates}/quiet/providers"))
        .bearer_auth(auth)
        .send()
        .await
        .unwrap()
        .json::<serde_json::Value>()
        .await
        .unwrap();
    assert_eq!(bindings["template_key"], "quiet");
    assert_eq!(bindings["revision"], 5);
    let llm_key = &generated[2].1;
    assert_eq!(bindings["bindings"]["llm"]["provider_key"], *llm_key);

    let provider_templates = client
        .get(format!("{providers}/{llm_key}/templates"))
        .bearer_auth(auth)
        .send()
        .await
        .unwrap()
        .json::<serde_json::Value>()
        .await
        .unwrap();
    assert_eq!(provider_templates["items"][0]["key"], "quiet");
    assert_eq!(provider_templates["revision"], 1);
    assert_eq!(provider_templates["total"], 1);

    let missing_if_match = client
        .delete(format!("{agents}/kitchen/templates/quiet"))
        .bearer_auth(auth)
        .send()
        .await
        .unwrap();
    assert_eq!(missing_if_match.status(), StatusCode::BAD_REQUEST);

    // Agents may have no Templates. Unlinking their sole default clears the assignment and lets
    // subsequent sessions resolve through the server defaults.
    let unlinked_default = client
        .delete(format!("{agents}/kitchen/templates/quiet"))
        .bearer_auth(auth)
        .header("if-match", "\"2\"")
        .send()
        .await
        .unwrap();
    assert_eq!(unlinked_default.status(), StatusCode::OK);
    assert_eq!(
        client
            .get(format!("{agents}/kitchen/templates"))
            .bearer_auth(auth)
            .send()
            .await
            .unwrap()
            .json::<serde_json::Value>()
            .await
            .unwrap()["items"]
            .as_array()
            .unwrap()
            .len(),
        0
    );
    assert_eq!(
        client
            .put(format!("{agents}/kitchen/templates/quiet"))
            .bearer_auth(auth)
            .header("if-match", "\"3\"")
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::OK
    );
    assert_eq!(
        client
            .post(&templates)
            .bearer_auth(auth)
            .json(&serde_json::json!({
                "key":"loud","name":"Loud","language":"vi-VN","prompt":"Nói to"
            }))
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::CREATED
    );
    // A default Template may inherit missing provider slots from server defaults.
    let default_loud = client
        .put(format!("{agents}/kitchen/default-template/loud"))
        .bearer_auth(auth)
        .header("if-match", "\"4\"")
        .send()
        .await
        .unwrap();
    assert_eq!(
        default_loud.status(),
        StatusCode::OK,
        "promoting a partial template failed: {:?}",
        default_loud.text().await
    );
    for (revision, (kind, provider_key)) in (1..).zip(&generated) {
        assert_eq!(
            client
                .put(format!("{templates}/loud/providers/{kind}"))
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
    let unlinked = client
        .delete(format!("{agents}/kitchen/templates/quiet"))
        .bearer_auth(auth)
        .header("if-match", "\"5\"")
        .send()
        .await
        .unwrap();
    assert_eq!(unlinked.status(), StatusCode::OK);
    assert_eq!(
        client
            .get(format!("{agents}/kitchen/templates"))
            .bearer_auth(auth)
            .send()
            .await
            .unwrap()
            .json::<serde_json::Value>()
            .await
            .unwrap()["items"]
            .as_array()
            .unwrap()
            .len(),
        1
    );

    let reassigned = client
        .put(format!("{agents}/kitchen/templates/quiet"))
        .bearer_auth(auth)
        .header("if-match", "\"6\"")
        .send()
        .await
        .unwrap();
    assert_eq!(reassigned.status(), StatusCode::OK);
    let stale_unlink = client
        .delete(format!("{agents}/kitchen/templates/quiet"))
        .bearer_auth(auth)
        .header("if-match", "\"6\"")
        .send()
        .await
        .unwrap();
    assert_eq!(stale_unlink.status(), StatusCode::CONFLICT);
    assert_eq!(
        stale_unlink.json::<serde_json::Value>().await.unwrap()["error"]["code"],
        "revision_conflict"
    );
    let defaulted = client
        .put(format!("{agents}/kitchen/default-template/quiet"))
        .bearer_auth(auth)
        .header("if-match", "\"7\"")
        .send()
        .await
        .unwrap();
    assert_eq!(defaulted.status(), StatusCode::OK);
    let unlinked_default = client
        .delete(format!("{agents}/kitchen/templates/quiet"))
        .bearer_auth(auth)
        .header("if-match", "\"8\"")
        .send()
        .await
        .unwrap();
    assert_eq!(unlinked_default.status(), StatusCode::OK);

    let missing_binding_if_match = client
        .delete(format!("{templates}/quiet/providers/llm"))
        .bearer_auth(auth)
        .send()
        .await
        .unwrap();
    assert_eq!(missing_binding_if_match.status(), StatusCode::BAD_REQUEST);

    let removed_binding = client
        .delete(format!("{templates}/quiet/providers/llm"))
        .bearer_auth(auth)
        .header("if-match", "\"5\"")
        .send()
        .await
        .unwrap();
    assert_eq!(removed_binding.status(), StatusCode::OK);
    let bindings = client
        .get(format!("{templates}/quiet/providers"))
        .bearer_auth(auth)
        .send()
        .await
        .unwrap()
        .json::<serde_json::Value>()
        .await
        .unwrap();
    assert!(bindings["bindings"].get("llm").is_none());
    let stale_binding_unlink = client
        .delete(format!("{templates}/quiet/providers/tts"))
        .bearer_auth(auth)
        .header("if-match", "\"5\"")
        .send()
        .await
        .unwrap();
    assert_eq!(stale_binding_unlink.status(), StatusCode::CONFLICT);
    assert_eq!(
        stale_binding_unlink
            .json::<serde_json::Value>()
            .await
            .unwrap()["error"]["code"],
        "revision_conflict"
    );
    let bindings = client
        .get(format!("{templates}/quiet/providers"))
        .bearer_auth(auth)
        .send()
        .await
        .unwrap()
        .json::<serde_json::Value>()
        .await
        .unwrap();
    assert_eq!(bindings["bindings"]["tts"]["provider_key"], generated[3].1);

    task.abort();
}

#[tokio::test]
async fn conditional_delete_requires_a_current_revision_and_preserves_linked_templates() {
    let (base, task) = server(true).await;
    let client = Client::new();
    let web_token = admin_web_token(&client, &base).await;
    let auth = web_token.as_str();
    let agents = format!("{base}/api/admin/agents");
    let templates = format!("{base}/api/admin/templates");
    let providers = format!("{base}/api/admin/providers");

    for (url, body) in [
        (
            agents.as_str(),
            serde_json::json!({"key":"kitchen","name":"Kitchen"}),
        ),
        (
            templates.as_str(),
            serde_json::json!({"key":"quiet","name":"Quiet","language":"vi-VN","prompt":"Be concise"}),
        ),
    ] {
        assert_eq!(
            client
                .post(url)
                .bearer_auth(auth)
                .json(&body)
                .send()
                .await
                .unwrap()
                .status(),
            StatusCode::CREATED
        );
    }
    let llm_key = create_provider(
        &client,
        &providers,
        serde_json::json!({"name":"LLM","type":"llm","adapter":"openai","config_json":{"base_url":"https://example.test/v1","model":"test","max_tokens":8}}),
    )
    .await;

    let missing_match = client
        .delete(format!("{agents}/kitchen"))
        .bearer_auth(auth)
        .send()
        .await
        .unwrap();
    assert_eq!(missing_match.status(), StatusCode::BAD_REQUEST);
    let stale = client
        .delete(format!("{agents}/kitchen"))
        .bearer_auth(auth)
        .header("if-match", "\"2\"")
        .send()
        .await
        .unwrap();
    assert_eq!(stale.status(), StatusCode::CONFLICT);
    assert_eq!(
        stale.json::<serde_json::Value>().await.unwrap()["error"]["code"],
        "revision_conflict"
    );

    assert_eq!(
        client
            .put(format!("{templates}/quiet/providers/llm"))
            .bearer_auth(auth)
            .header("if-match", "\"1\"")
            .json(&serde_json::json!({"provider_key":llm_key}))
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::OK
    );
    let provider_in_use = client
        .delete(format!("{providers}/{llm_key}"))
        .bearer_auth(auth)
        .header("if-match", "\"1\"")
        .send()
        .await
        .unwrap();
    assert_eq!(provider_in_use.status(), StatusCode::CONFLICT);
    assert_eq!(
        provider_in_use.json::<serde_json::Value>().await.unwrap()["error"]["code"],
        "provider_in_use"
    );
    assert_eq!(
        client
            .delete(format!("{templates}/quiet/providers/llm"))
            .bearer_auth(auth)
            .header("if-match", "\"2\"")
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::OK
    );
    assert_eq!(
        client
            .delete(format!("{providers}/{llm_key}"))
            .bearer_auth(auth)
            .header("if-match", "\"1\"")
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::NO_CONTENT
    );

    assert_eq!(
        client
            .put(format!("{agents}/kitchen/templates/quiet"))
            .bearer_auth(auth)
            .header("if-match", "\"1\"")
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::OK
    );
    let template_in_use = client
        .delete(format!("{templates}/quiet"))
        .bearer_auth(auth)
        .header("if-match", "\"3\"")
        .send()
        .await
        .unwrap();
    assert_eq!(template_in_use.status(), StatusCode::CONFLICT);
    assert_eq!(
        template_in_use.json::<serde_json::Value>().await.unwrap()["error"]["code"],
        "template_in_use"
    );
    // A Template is globally reusable. Deleting an Agent only removes that Agent's assignment;
    // it must not delete or otherwise mutate the Template itself.
    assert_eq!(
        client
            .delete(format!("{agents}/kitchen"))
            .bearer_auth(auth)
            .header("if-match", "\"2\"")
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        client
            .get(format!("{templates}/quiet"))
            .bearer_auth(auth)
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::OK
    );
    let template_agents = client
        .get(format!("{templates}/quiet/agents"))
        .bearer_auth(auth)
        .send()
        .await
        .unwrap();
    assert_eq!(template_agents.status(), StatusCode::OK);
    assert_eq!(
        template_agents.json::<serde_json::Value>().await.unwrap()["items"],
        serde_json::json!([])
    );
    assert_eq!(
        client
            .post(&agents)
            .bearer_auth(auth)
            .json(&serde_json::json!({"key":"kitchen","name":"Kitchen"}))
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::CREATED
    );
    let devices = format!("{base}/api/admin/devices");
    assert_eq!(
        client
            .post(&devices)
            .bearer_auth(auth)
            .json(&serde_json::json!({"device_id":"kitchen-speaker","agent_key":"kitchen"}))
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::CREATED
    );
    let agent_in_use = client
        .delete(format!("{agents}/kitchen"))
        .bearer_auth(auth)
        .header("if-match", "\"1\"")
        .send()
        .await
        .unwrap();
    assert_eq!(agent_in_use.status(), StatusCode::CONFLICT);
    assert_eq!(
        agent_in_use.json::<serde_json::Value>().await.unwrap()["error"]["code"],
        "agent_in_use"
    );
    assert_eq!(
        client
            .delete(format!("{devices}/kitchen-speaker"))
            .bearer_auth(auth)
            .header("if-match", "\"1\"")
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        client
            .delete(format!("{agents}/kitchen"))
            .bearer_auth(auth)
            .header("if-match", "\"1\"")
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::NO_CONTENT
    );
    task.abort();
}

#[tokio::test]
async fn failed_success_audit_rolls_back_each_new_desired_resource() {
    let database_uri = database_url();
    let (base, task) = server_with_database(true, &database_uri).await;
    let client = Client::new();
    let web_token = admin_web_token(&client, &base).await;
    let pool = sqlx::SqlitePool::connect(&database_uri).await.unwrap();
    sqlx::query("CREATE TRIGGER reject_success_audit BEFORE INSERT ON admin_audit_events WHEN NEW.outcome='success' BEGIN SELECT RAISE(ABORT,'test audit unavailable'); END")
        .execute(&pool).await.unwrap();
    for (route, table, body) in [
        (
            "agents",
            "agents",
            serde_json::json!({"key":"audit_agent","name":"Agent"}),
        ),
        (
            "templates",
            "agent_templates",
            serde_json::json!({"key":"audit_template","name":"Template","language":"en","prompt":"Hello"}),
        ),
        (
            "providers",
            "providers",
            serde_json::json!({"name":"Provider","type":"llm","adapter":"openai","config_json":{"base_url":"https://api.example.com/v1","model":"test"}}),
        ),
        (
            "mcp-servers",
            "mcp_servers",
            serde_json::json!({"key":"audit_mcp","name":"MCP","url":"https://example.com/mcp","auth":{"type":"none"}}),
        ),
        (
            "speakers",
            "speakers",
            serde_json::json!({"key":"audit_speaker","name":"Speaker"}),
        ),
    ] {
        let response = client
            .post(format!("{base}/api/admin/{route}"))
            .bearer_auth(&web_token)
            .json(&body)
            .send()
            .await
            .unwrap();
        assert_eq!(
            response.status(),
            StatusCode::SERVICE_UNAVAILABLE,
            "{route}"
        );
        let error: serde_json::Value = response.json().await.unwrap();
        assert_eq!(error["error"]["code"], "database_unavailable", "{route}");
        let count: i64 = sqlx::query_scalar(&format!("SELECT COUNT(*) FROM {table}"))
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(count, 0, "{route} survived an audit failure");
    }
    task.abort();
}

#[tokio::test]
async fn admin_mutations_wait_for_a_concurrent_writer() {
    let database_uri = database_url();
    let (base, task) = server_with_database(true, &database_uri).await;
    let client = Client::new();
    let web_token = admin_web_token(&client, &base).await;
    let admin = format!("{base}/api/admin");
    let llm = create_provider(
        &client,
        &format!("{admin}/providers"),
        serde_json::json!({
            "name":"Main LLM","type":"llm","adapter":"openai",
            "config_json":{"base_url":"https://example.test/v1","model":"test","max_tokens":8}
        }),
    )
    .await;
    for (path, payload) in [
        (
            "agents",
            serde_json::json!({"key":"kitchen","name":"Kitchen"}),
        ),
        (
            "templates",
            serde_json::json!({"key":"quiet","name":"Quiet","language":"vi-VN","prompt":"Short"}),
        ),
    ] {
        client
            .post(format!("{admin}/{path}"))
            .bearer_auth(&web_token)
            .json(&payload)
            .send()
            .await
            .unwrap()
            .error_for_status()
            .unwrap();
    }

    let create_device = client
        .post(format!("{admin}/devices"))
        .bearer_auth(&web_token)
        .json(&serde_json::json!({"device_id":"device-1","agent_key":"kitchen"}));
    assert_waits_for_writer(&database_uri, create_device, StatusCode::CREATED).await;

    let update_device = client
        .patch(format!("{admin}/devices/device-1"))
        .bearer_auth(&web_token)
        .header("if-match", "\"1\"")
        .json(&serde_json::json!({"name":"Kitchen speaker"}));
    assert_waits_for_writer(&database_uri, update_device, StatusCode::OK).await;

    let bind_provider = client
        .put(format!("{admin}/templates/quiet/providers/llm"))
        .bearer_auth(&web_token)
        .header("if-match", "\"1\"")
        .json(&serde_json::json!({"provider_key":llm}));
    assert_waits_for_writer(&database_uri, bind_provider, StatusCode::OK).await;

    let assign_template = client
        .put(format!("{admin}/agents/kitchen/templates/quiet"))
        .bearer_auth(&web_token)
        .header("if-match", "\"1\"");
    assert_waits_for_writer(&database_uri, assign_template, StatusCode::OK).await;

    let delete_device = client
        .delete(format!("{admin}/devices/device-1"))
        .bearer_auth(&web_token)
        .header("if-match", "\"2\"");
    assert_waits_for_writer(&database_uri, delete_device, StatusCode::NO_CONTENT).await;
    task.abort();
}
