use super::super::*;

#[tokio::test]
async fn templates_and_provider_desired_configuration_are_bounded_and_restart_honest() {
    let (base, task) = server(true).await;
    let client = Client::new();
    let web_token = admin_web_token(&client, &base).await;
    let auth = web_token.as_str();

    let agent = client
        .post(format!("{base}/api/admin/agents"))
        .bearer_auth(auth)
        .json(&serde_json::json!({"key":"living_room","name":"Living room"}))
        .send()
        .await
        .unwrap()
        .json::<serde_json::Value>()
        .await
        .unwrap();
    assert_eq!(agent["revision"], 1);

    let provider_url = format!("{base}/api/admin/providers");
    let protected = client
        .post(&provider_url)
        .bearer_auth(auth)
        .json(&serde_json::json!({
            "name":"Bad", "type":"llm", "adapter":"openai",
            "config_json":{"model":"x","nested":{"api_key":"nope"}}
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(protected.status(), StatusCode::BAD_REQUEST);
    assert_eq!(
        protected.json::<serde_json::Value>().await.unwrap()["error"]["code"],
        "provider_config_invalid"
    );

    let llm = client
        .post(&provider_url)
        .bearer_auth(auth)
        .json(&serde_json::json!({
            "name":"LLM", "type":"llm", "adapter":"openai",
            "config_json":{"base_url":"https://example.test/v1","model":"test","max_tokens":8}
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(llm.status(), StatusCode::CREATED);
    let llm = llm.json::<serde_json::Value>().await.unwrap();
    let llm_key = llm["key"].as_str().expect("a key is generated").to_owned();
    assert_eq!(
        llm["credential_env"],
        format!("VOICE_PROVIDER_{}_API_KEY", llm_key.to_ascii_uppercase())
    );
    assert!(llm.get("secret_ref").is_none());
    // Old clients must not be able to store an operator-chosen secret reference.
    let old_provider_payload = client
        .post(&provider_url)
        .bearer_auth(auth)
        .json(&serde_json::json!({
            "name":"Legacy", "type":"llm", "adapter":"openai",
            "config_json":{"base_url":"https://example.test/v1","model":"test"},
            "secret_ref":"LEGACY_SECRET"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(old_provider_payload.status(), StatusCode::BAD_REQUEST);

    assert_eq!(llm["runtime_status"], "not_loaded");
    assert_eq!(llm["runtime_matches_desired"], false);
    assert_eq!(llm["requires_restart"], true);

    let mut generated = vec![("llm".to_owned(), llm_key)];
    for (name, kind, adapter, config_json) in [
        ("Main VAD", "vad", "silero_onnx", serde_json::json!({})),
        (
            "Main ASR",
            "asr",
            "zipformer_sherpa",
            serde_json::json!({"decoding_method":"greedy_search"}),
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
            &provider_url,
            serde_json::json!({
                "name":name,"type":kind,"adapter":adapter,"config_json":config_json
            }),
        )
        .await;
        generated.push((kind.to_owned(), key));
    }

    let templates_url = format!("{base}/api/admin/templates");
    let template = client
        .post(&templates_url)
        .bearer_auth(auth)
        .json(&serde_json::json!({
            "key":"quiet","name":"Quiet","language":"vi-VN","prompt":"Nói ngắn gọn"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(template.status(), StatusCode::CREATED);
    let mut template = template.json::<serde_json::Value>().await.unwrap();

    for (kind, provider_key) in &generated {
        let response = client
            .put(format!("{base}/api/admin/templates/quiet/providers/{kind}"))
            .bearer_auth(auth)
            .header("if-match", format!("\"{}\"", template["revision"]))
            .json(&serde_json::json!({"provider_key":provider_key}))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        template = response.json().await.unwrap();
    }
    let assigned = client
        .put(format!(
            "{base}/api/admin/agents/living_room/default-template/quiet"
        ))
        .bearer_auth(auth)
        .header("if-match", "\"1\"")
        .send()
        .await
        .unwrap();
    assert_eq!(assigned.status(), StatusCode::OK);
    task.abort();
}

#[tokio::test]
async fn kokoro_desired_configuration_accepts_catalog_selection_and_rejects_factory_incompatible_voice()
 {
    let (base, task) = server(true).await;
    let client = Client::new();
    let web_token = admin_web_token(&client, &base).await;
    for (voice, status) in [
        ("diem_trinh", StatusCode::CREATED),
        ("unknown", StatusCode::BAD_REQUEST),
    ] {
        let response = client.post(format!("{base}/api/admin/providers"))
            .bearer_auth(&web_token)
            .json(&serde_json::json!({"name":"Kokoro","type":"tts","adapter":"kokoro_vi_onnx","config_json":{"voice":voice,"language":"vi-VN","speed_percent":100}}))
            .send().await.unwrap();
        assert_eq!(response.status(), status);
    }
    task.abort();
}

#[tokio::test]
async fn openai_provider_mutations_accept_http_and_https_base_urls() {
    let (base, task) = server(true).await;
    let client = Client::new();
    let web_token = admin_web_token(&client, &base).await;
    let providers = format!("{base}/api/admin/providers");
    for endpoint in [
        "http://localhost:8000/v1",
        "http://192.168.1.158:20128/v1",
        "http://llm:8000/v1",
        "http://example.test/v1",
        "https://example.test/v1",
        "ftp://example.test/v1",
        "file:///tmp/model",
        "not-a-url",
    ] {
        let config = serde_json::json!({"base_url":endpoint,"model":"test"});
        let accepted = endpoint.starts_with("http://") || endpoint.starts_with("https://");
        let response = client
            .post(&providers)
            .bearer_auth(&web_token)
            .json(&serde_json::json!({"name":"OpenAI","type":"llm","adapter":"openai","config_json":config}))
            .send()
            .await
            .unwrap();
        assert_eq!(
            response.status(),
            if accepted {
                StatusCode::CREATED
            } else {
                StatusCode::BAD_REQUEST
            },
            "create: {endpoint}"
        );
        let key = create_provider(
            &client,
            &providers,
            serde_json::json!({"name":"OpenAI","type":"llm","adapter":"openai","config_json":{"base_url":"https://example.test/v1","model":"test"}}),
        )
        .await;
        let response = client
            .patch(format!("{providers}/{key}"))
            .bearer_auth(&web_token)
            .header("if-match", "\"1\"")
            .json(&serde_json::json!({"config_json":config}))
            .send()
            .await
            .unwrap();
        assert_eq!(
            response.status(),
            if accepted {
                StatusCode::OK
            } else {
                StatusCode::BAD_REQUEST
            },
            "update: {endpoint}"
        );
        if accepted {
            let row = response.json::<serde_json::Value>().await.unwrap();
            let saved: serde_json::Value =
                serde_json::from_str(row["config_json"].as_str().unwrap()).unwrap();
            assert_eq!(saved["base_url"], endpoint);
        }
    }
    task.abort();
}

#[tokio::test]
async fn provider_mutations_reject_server_owned_configuration() {
    let (base, task) = server(true).await;
    let client = Client::new();
    let web_token = admin_web_token(&client, &base).await;
    for (kind, adapter, valid, overrides) in [
        (
            "vad",
            "silero_onnx",
            serde_json::json!({}),
            vec![
                ("num_threads", serde_json::json!(128)),
                ("model", serde_json::json!("silero_vad_v5")),
            ],
        ),
        (
            "asr",
            "zipformer_sherpa",
            serde_json::json!({"decoding_method":"greedy_search"}),
            vec![
                ("num_threads", serde_json::json!(128)),
                ("model", serde_json::json!("zipformer_vi_streaming")),
                ("decoding_method", serde_json::json!("arbitrary")),
            ],
        ),
        (
            "asr",
            "gipformer_sherpa_offline",
            serde_json::json!({}),
            vec![
                ("num_threads", serde_json::json!(128)),
                ("model", serde_json::json!("gipformer15_vi_int8")),
                ("decoding_method", serde_json::json!("arbitrary")),
                ("max_active_paths", serde_json::json!(10001)),
            ],
        ),
        (
            "asr",
            "sensevoice_sherpa_offline",
            serde_json::json!({"language":"en","use_itn":true}),
            vec![
                ("num_threads", serde_json::json!(128)),
                ("model", serde_json::json!("sensevoice_small_int8")),
                ("language", serde_json::json!("fr")),
            ],
        ),
        (
            "tts",
            "zerotts_onnx",
            serde_json::json!({"voice":"maichi"}),
            vec![
                ("num_threads", serde_json::json!(128)),
                ("model", serde_json::json!("zerotts_default")),
                ("voice", serde_json::json!("unknown")),
            ],
        ),
        (
            "tts",
            "kokoro_vi_onnx",
            serde_json::json!({"voice":"diem_trinh"}),
            vec![
                ("num_threads", serde_json::json!(128)),
                ("model", serde_json::json!("kokoro_vi_contextbox")),
            ],
        ),
        (
            "tts",
            "chillaudio_ws",
            serde_json::json!({"voice":"BV421_vivn_streaming"}),
            vec![
                ("ws_url", serde_json::json!("wss://attacker.example/socket")),
                ("timeout_ms", serde_json::json!(1)),
                ("voice", serde_json::json!("unknown")),
            ],
        ),
        (
            "tts",
            "edge_tts",
            serde_json::json!({"voice":"vi-VN-HoaiMyNeural","language":"vi-VN","rate_percent":0,"pitch_hz":0}),
            vec![
                ("ws_url", serde_json::json!("wss://attacker.example/socket")),
                ("timeout_ms", serde_json::json!(1)),
                ("voice", serde_json::json!("unknown")),
                ("language", serde_json::json!("en-US")),
                ("rate_percent", serde_json::json!(150)),
                ("pitch_hz", serde_json::json!(-80)),
            ],
        ),
    ]
    .into_iter()
    {
        let providers = format!("{base}/api/admin/providers");
        let key = create_provider(
            &client,
            &providers,
            serde_json::json!({"name":"Valid","type":kind,"adapter":adapter,"config_json":valid}),
        )
        .await;
        for (field, value) in overrides {
            let mut invalid = valid.clone();
            invalid[field] = value;
            for update in [false, true] {
                let request = if update {
                    client
                        .patch(format!("{providers}/{key}"))
                        .header("if-match", "\"1\"")
                        .json(&serde_json::json!({"config_json":invalid}))
                } else {
                    client.post(&providers)
                        .json(&serde_json::json!({"name":"Invalid","type":kind,"adapter":adapter,"config_json":invalid}))
                };
                let response = request
                    .bearer_auth(&web_token)
                    .send()
                    .await
                    .unwrap();
                assert_eq!(
                    response.status(),
                    StatusCode::BAD_REQUEST,
                    "{adapter}.{field}, update={update}"
                );
                assert_eq!(
                    response.json::<serde_json::Value>().await.unwrap()["error"]["code"],
                    "provider_config_invalid"
                );
            }
        }
        let row = client
            .get(format!("{providers}/{key}"))
            .bearer_auth(&web_token)
            .send()
            .await
            .unwrap()
            .json::<serde_json::Value>()
            .await
            .unwrap();
        assert_eq!(row["revision"], 1);
    }
    task.abort();
}

#[tokio::test]
async fn provider_key_is_generated_by_the_server_and_never_accepted_from_a_client() {
    let (base, task) = server(true).await;
    let client = Client::new();
    let web_token = admin_web_token(&client, &base).await;
    let providers = format!("{base}/api/admin/providers");
    let body = serde_json::json!({"name":"OpenAI LLM","type":"llm","adapter":"openai","config_json":{"base_url":"https://example.test/v1","model":"test","max_tokens":8}});
    let key = create_provider(&client, &providers, body.clone()).await;
    assert!(key.starts_with("llm_"), "{key}");
    assert!(key.len() <= 64, "{key}");
    assert!(
        key.bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_'),
        "{key}"
    );
    let identity = key.strip_prefix("llm_").expect("the type prefixes the key");
    assert_eq!(identity.len(), 32, "{key}");
    assert!(
        identity
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)),
        "{key}"
    );

    let twin = create_provider(&client, &providers, body).await;
    assert_ne!(twin, key, "identity never comes from the display name");

    let chosen = client
        .post(&providers)
        .bearer_auth(&web_token)
        .json(&serde_json::json!({"key":"user_controlled_key","name":"OpenAI LLM","type":"llm","adapter":"openai","config_json":{}}))
        .send()
        .await
        .unwrap();
    assert_eq!(chosen.status(), StatusCode::BAD_REQUEST);
    assert_eq!(
        chosen.json::<serde_json::Value>().await.unwrap()["error"]["code"],
        "invalid_json"
    );

    let fetched = client
        .get(format!("{providers}/{key}"))
        .bearer_auth(&web_token)
        .send()
        .await
        .unwrap();
    assert_eq!(fetched.status(), StatusCode::OK);
    assert_eq!(
        fetched.json::<serde_json::Value>().await.unwrap()["key"],
        key
    );

    let renamed = client
        .patch(format!("{providers}/{key}"))
        .bearer_auth(&web_token)
        .header("if-match", "\"1\"")
        .json(&serde_json::json!({"name":"Renamed"}))
        .send()
        .await
        .unwrap();
    assert_eq!(renamed.status(), StatusCode::OK);
    // A display name is the user's to change; identity is not.
    assert_eq!(
        renamed.json::<serde_json::Value>().await.unwrap()["key"],
        key
    );

    let immovable = client
        .patch(format!("{providers}/{key}"))
        .bearer_auth(&web_token)
        .header("if-match", "\"2\"")
        .json(&serde_json::json!({"key":"new_key"}))
        .send()
        .await
        .unwrap();
    assert_eq!(immovable.status(), StatusCode::BAD_REQUEST);
    assert_eq!(
        immovable.json::<serde_json::Value>().await.unwrap()["error"]["code"],
        "immutable_field"
    );
    task.abort();
}

#[tokio::test]
async fn a_generated_provider_key_binds_a_template_without_being_retyped() {
    use vela_server::config::DatabaseConfig;
    let database_uri = database_url();
    let (base, task) = server_with_database(true, &database_uri).await;
    let client = Client::new();
    let web_token = admin_web_token(&client, &base).await;
    let templates = format!("{base}/api/admin/templates");
    let providers = format!("{base}/api/admin/providers");
    assert_eq!(
        client
            .post(&templates)
            .bearer_auth(&web_token)
            .json(&serde_json::json!({"key":"quiet","name":"Quiet","language":"vi-VN","prompt":"Be concise"}))
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::CREATED
    );
    let key = create_provider(
        &client,
        &providers,
        serde_json::json!({"name":"Main LLM","type":"llm","adapter":"openai","config_json":{"base_url":"https://example.test/v1","model":"test","max_tokens":8}}),
    )
    .await;
    assert_eq!(
        client
            .put(format!("{templates}/quiet/providers/llm"))
            .bearer_auth(&web_token)
            .header("if-match", "\"1\"")
            .json(&serde_json::json!({"provider_key":key}))
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::OK
    );
    let bindings: serde_json::Value = client
        .get(format!("{templates}/quiet/providers"))
        .bearer_auth(&web_token)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(bindings["bindings"]["llm"]["provider_key"], key);
    let using: serde_json::Value = client
        .get(format!("{providers}/{key}/templates"))
        .bearer_auth(&web_token)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(using["items"][0]["key"], "quiet");

    // An echoed key would still pass above if the row resolved to a different provider, so the
    // binding is checked against the row the generated key actually names.
    let database = Database::connect(&DatabaseConfig {
        url: database_uri,
        ..Default::default()
    })
    .await
    .unwrap();
    let provider_id: i64 = sqlx::query_scalar("SELECT id FROM providers WHERE key = ?")
        .bind(&key)
        .fetch_one(database.pool())
        .await
        .unwrap();
    let bound_id: i64 = sqlx::query_scalar(
        "SELECT b.provider_id FROM template_provider_bindings b \
         JOIN agent_templates t ON t.id = b.template_id WHERE t.key = ? AND b.provider_type = ?",
    )
    .bind("quiet")
    .bind("llm")
    .fetch_one(database.pool())
    .await
    .unwrap();
    assert_eq!(bound_id, provider_id);
    task.abort();
}

#[tokio::test]
async fn legacy_provider_runtime_fields_are_removed_by_migration_before_admin_reads() {
    use vela_server::config::DatabaseConfig;
    let database_uri = database_url();
    let config = DatabaseConfig {
        url: database_uri.clone(),
        ..Default::default()
    };
    let database = Database::connect(&config).await.unwrap();
    let cases = [
        (
            "legacy_vad",
            "vad",
            "silero_onnx",
            serde_json::json!({"model":"silero_vad_v5","num_threads":128}),
            serde_json::json!({}),
        ),
        (
            "legacy_asr",
            "asr",
            "zipformer_sherpa",
            serde_json::json!({"model":"zipformer_vi_streaming","num_threads":128,"decoding_method":"greedy_search"}),
            serde_json::json!({"decoding_method":"greedy_search"}),
        ),
        (
            "legacy_gip",
            "asr",
            "gipformer_sherpa_offline",
            serde_json::json!({"model":"gipformer15_vi_int8","num_threads":128}),
            serde_json::json!({}),
        ),
        (
            "legacy_zero",
            "tts",
            "zerotts_onnx",
            serde_json::json!({"model":"zerotts_default","num_threads":128,"voice":"maichi"}),
            serde_json::json!({"voice":"maichi"}),
        ),
        (
            "legacy_kokoro",
            "tts",
            "kokoro_vi_onnx",
            serde_json::json!({"model":"kokoro_vi_contextbox","num_threads":128,"voice":"duc_an"}),
            serde_json::json!({"voice":"duc_an"}),
        ),
        (
            "legacy_chill",
            "tts",
            "chillaudio_ws",
            serde_json::json!({"ws_url":"wss://attacker.example/ws","timeout_ms":1,"voice":"BV421_vivn_streaming"}),
            serde_json::json!({"voice":"BV421_vivn_streaming"}),
        ),
    ];
    for (key, kind, adapter, legacy, _) in &cases {
        sqlx::query("INSERT INTO providers (key,name,type,adapter,config_json,revision,created_at,updated_at) VALUES (?,?,?,?,?,1,1,1)")
            .bind(*key).bind(*key).bind(*kind).bind(*adapter).bind(legacy.to_string())
            .execute(database.pool()).await.unwrap();
    }
    // Fixture represents the previous version: migration 0006 changes data only.
    sqlx::query("DELETE FROM _sqlx_migrations WHERE version = 6")
        .execute(database.pool())
        .await
        .unwrap();
    database.pool().close().await;
    let (base, task) = server_with_database(true, &database_uri).await;
    let client = Client::new();
    let web_token = admin_web_token(&client, &base).await;
    for (key, _, _, _, expected) in &cases {
        let response = client
            .get(format!("{base}/api/admin/providers/{key}"))
            .bearer_auth(&web_token)
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let row = response.json::<serde_json::Value>().await.unwrap();
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(row["config_json"].as_str().unwrap())
                .unwrap(),
            *expected
        );
        assert_eq!(row["revision"], 2);
    }
    task.abort();
}
