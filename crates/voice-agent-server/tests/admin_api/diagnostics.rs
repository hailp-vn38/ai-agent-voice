use super::harness::*;

#[tokio::test]
async fn vad_provider_test_uses_one_canonical_silent_frame_from_the_loaded_runtime() {
    let (database_uri, key) = seeded_provider(serde_json::json!({
        "name":"Loaded VAD", "type":"vad", "adapter":"silero_onnx",
        "config_json":{}
    }))
    .await;
    let (base, inputs, task) = server_with_loaded_vad(&database_uri, &key).await;
    let client = Client::new();
    let providers = format!("{base}/api/admin/providers");
    let response = client
        .post(format!("{providers}/{key}/test/vad"))
        .bearer_auth("admin-test-token")
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let response = response.json::<serde_json::Value>().await.unwrap();
    assert_eq!(response["provider_key"], key);
    assert_eq!(response["type"], "vad");
    assert_eq!(response["result"]["probability"], 0.25);
    assert_eq!(response["result"]["start_sample"], 0);
    assert_eq!(response["result"]["end_sample"], 512);
    assert_eq!(
        inputs.lock().unwrap().as_slice(),
        &[VadInput {
            pcm: vec![0.0; 512],
            start_sample: 0
        }]
    );
    let nonempty = client
        .post(format!("{providers}/{key}/test/vad"))
        .bearer_auth("admin-test-token")
        .body("caller pcm is not accepted")
        .send()
        .await
        .unwrap();
    assert_eq!(nonempty.status(), StatusCode::BAD_REQUEST);
    assert_eq!(
        nonempty.json::<serde_json::Value>().await.unwrap()["error"]["code"],
        "invalid_test_input"
    );
    let other = create_provider(
        &client,
        &providers,
        serde_json::json!({
            "name":"Other ASR", "type":"asr", "adapter":"gipformer_sherpa_offline",
            "config_json":{"decoding_method":"greedy_search","max_active_paths":4}
        }),
    )
    .await;
    let wrong_type = client
        .post(format!("{providers}/{other}/test/vad"))
        .bearer_auth("admin-test-token")
        .send()
        .await
        .unwrap();
    assert_eq!(wrong_type.status(), StatusCode::BAD_REQUEST);
    assert_eq!(
        wrong_type.json::<serde_json::Value>().await.unwrap()["error"]["code"],
        "provider_type_mismatch"
    );
    assert_eq!(
        client
            .post(format!("{providers}/missing/test/vad"))
            .bearer_auth("admin-test-token")
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::NOT_FOUND
    );
    task.abort();
}

#[tokio::test]
async fn llm_provider_test_uses_only_the_loaded_runtime_and_a_tool_free_request() {
    let (database_uri, key) = seeded_provider(serde_json::json!({
        "name": "Loaded LLM", "type": "llm", "adapter": "openai",
        "config_json": {"base_url":"https://example.test/v1","model":"test","max_tokens":8}
    }))
    .await;
    let (base, requests, task) = server_with_loaded_llm(&database_uri, &key).await;
    let client = Client::new();
    let providers = format!("{base}/api/admin/providers");

    let response = client
        .post(format!("{providers}/{key}/test/llm"))
        .bearer_auth("admin-test-token")
        .json(&serde_json::json!({"input":"xin chao"}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let response = response.json::<serde_json::Value>().await.unwrap();
    assert_eq!(response["provider_key"], key);
    assert_eq!(response["type"], "llm");
    assert_eq!(response["status"], "success");
    assert_eq!(response["result"]["text"], "diagnostic answer");
    assert_eq!(response["runtime"]["runtime_status"], "loaded");
    assert_eq!(response["runtime"]["tested_runtime"], "loaded");
    assert_eq!(response["runtime"]["runtime_matches_desired"], true);
    assert_eq!(response["runtime"]["requires_restart"], false);
    assert!(response["metrics"]["elapsed_ms"].is_u64());

    let changed = client
        .patch(format!("{providers}/{key}"))
        .bearer_auth("admin-test-token")
        .header("if-match", "\"1\"")
        .json(&serde_json::json!({"name":"Changed desired state"}))
        .send()
        .await
        .unwrap();
    assert_eq!(changed.status(), StatusCode::OK);
    let stale = client
        .post(format!("{providers}/{key}/test/llm"))
        .bearer_auth("admin-test-token")
        .json(&serde_json::json!({"input":"still loaded"}))
        .send()
        .await
        .unwrap()
        .json::<serde_json::Value>()
        .await
        .unwrap();
    assert_eq!(stale["status"], "success");
    assert_eq!(stale["runtime"]["runtime_matches_desired"], false);
    assert_eq!(stale["runtime"]["requires_restart"], true);

    {
        let requests = requests.lock().unwrap();
        assert_eq!(requests.len(), 2);
        assert!(requests[0].tools.is_empty());
        assert_eq!(
            requests[0].messages,
            vec![ChatMessage::User {
                content: "xin chao".into()
            }]
        );
    }

    let invalid = client
        .post(format!("{providers}/{key}/test/llm"))
        .bearer_auth("admin-test-token")
        .json(&serde_json::json!({"input":""}))
        .send()
        .await
        .unwrap();
    assert_eq!(invalid.status(), StatusCode::BAD_REQUEST);
    assert_eq!(
        invalid.json::<serde_json::Value>().await.unwrap()["error"]["code"],
        "invalid_test_input"
    );
    task.abort();
}

#[tokio::test]
async fn admin_p1_read_models_and_device_template_override_are_public_contracts() {
    let (base, task) = server(true).await;
    let client = Client::new();
    let auth = "admin-test-token";
    let agents = format!("{base}/api/admin/agents");
    let templates = format!("{base}/api/admin/templates");
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
    for (key, name, language) in [
        ("default", "Default assistant", "vi-VN"),
        ("kids", "Kids assistant", "en-US"),
    ] {
        assert_eq!(client.post(&templates).bearer_auth(auth).json(&serde_json::json!({"key":key,"name":name,"language":language,"prompt":"A bounded prompt"})).send().await.unwrap().status(), StatusCode::CREATED);
    }
    let assigned = client
        .put(format!("{base}/api/admin/agents/kitchen/templates/default"))
        .bearer_auth(auth)
        .header("if-match", "\"1\"")
        .send()
        .await
        .unwrap();
    assert_eq!(assigned.status(), StatusCode::OK);
    assert_eq!(
        client
            .put(format!("{base}/api/admin/agents/kitchen/templates/kids"))
            .bearer_auth(auth)
            .header("if-match", "\"2\"")
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::OK
    );

    let filtered: serde_json::Value = client
        .get(format!(
            "{templates}?q=Kids&language=en-US&page_size=1&sort=name"
        ))
        .bearer_auth(auth)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(filtered["total"], 1);
    assert_eq!(filtered["total_pages"], 1);
    assert_eq!(filtered["items"][0]["key"], "kids");
    assert_eq!(
        client
            .get(format!("{templates}?sort=unsafe"))
            .bearer_auth(auth)
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::BAD_REQUEST
    );

    let providers = format!("{base}/api/admin/providers");
    let mut kids_vad = String::new();
    for (name, kind, adapter, config_json) in [
        ("Kids VAD", "vad", "silero_onnx", serde_json::json!({})),
        (
            "Main LLM",
            "llm",
            "openai",
            serde_json::json!({"base_url":"https://example.test/v1","model":"test","max_tokens":8}),
        ),
    ] {
        let key = create_provider(
            &client,
            &providers,
            serde_json::json!({"name":name,"type":kind,"adapter":adapter,"config_json":config_json}),
        )
        .await;
        if kind == "vad" {
            kids_vad = key;
        }
    }
    let provider_page: serde_json::Value = client
        .get(format!("{providers}?type=vad&q=Kids&page_size=1&sort=name"))
        .bearer_auth(auth)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(provider_page["total"], 1);
    assert_eq!(provider_page["facets"]["vad"], 1);
    assert_eq!(provider_page["facets"]["llm"], 0);
    assert_eq!(provider_page["items"][0]["key"], kids_vad);

    let device_url = format!("{base}/api/admin/devices");
    let device: serde_json::Value = client.post(&device_url).bearer_auth(auth).json(&serde_json::json!({"device_id":"kitchen-speaker","agent_key":"kitchen","template_key":"kids"})).send().await.unwrap().json().await.unwrap();
    assert_eq!(device["template_key"], "kids");
    let cleared: serde_json::Value = client
        .patch(format!("{device_url}/kitchen-speaker"))
        .bearer_auth(auth)
        .header("if-match", "\"1\"")
        .json(&serde_json::json!({"template_key":null}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(cleared["template_key"].is_null());
    assert_eq!(
        client
            .patch(format!("{device_url}/kitchen-speaker"))
            .bearer_auth(auth)
            .header("if-match", "\"2\"")
            .json(&serde_json::json!({"template_key":"default"}))
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::OK
    );
    assert_eq!(
        client
            .patch(format!("{device_url}/kitchen-speaker"))
            .bearer_auth(auth)
            .header("if-match", "\"3\"")
            .json(&serde_json::json!({"template_key":"missing"}))
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::BAD_REQUEST
    );

    let system: serde_json::Value = client
        .get(format!("{base}/api/admin/system"))
        .bearer_auth(auth)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(system["database"]["enabled"], true);
    assert!(system["uptime_seconds"].is_u64());
    task.abort();
}

#[tokio::test]
async fn admin_system_reports_database_unavailable_without_leaking_runtime_details() {
    let (base, task) = server_with_closed_database().await;
    let response = Client::new()
        .get(format!("{base}/api/admin/system"))
        .bearer_auth("admin-test-token")
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body: serde_json::Value = response.json().await.unwrap();
    assert_eq!(body["database"]["status"], "unavailable");
    assert!(body["providers"]["configured"].is_null());
    assert!(body["providers"]["loaded"].is_null());
    assert!(!body.to_string().contains("sqlite"));
    task.abort();
}

#[tokio::test]
async fn tts_provider_test_passes_typed_override_to_loaded_adapter_and_returns_provider_wav() {
    let (database_uri, key) = seeded_provider(serde_json::json!({
        "name":"Loaded TTS", "type":"tts", "adapter":"zerotts_onnx",
        "config_json":{"voice":"maichi","language":"vi-VN"}
    }))
    .await;
    let (base, requests, task) = server_with_loaded_tts(&database_uri, &key).await;
    let client = Client::new();
    let providers = format!("{base}/api/admin/providers");
    let response = client
        .post(format!("{providers}/{key}/test/tts"))
        .bearer_auth("admin-test-token")
        .json(&serde_json::json!({"text":"xin chao","voice":"maichi","language":"vi-VN"}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()["content-type"], "audio/wav");
    assert_eq!(
        response.headers()["x-provider-runtime-matches-desired"],
        "true"
    );
    let wav = response.bytes().await.unwrap();
    assert_eq!(&wav[..4], b"RIFF");
    assert_eq!(&wav[8..12], b"WAVE");
    assert_eq!(u32::from_le_bytes(wav[24..28].try_into().unwrap()), 48_000);
    assert_eq!(
        requests.lock().unwrap().as_slice(),
        &[TtsDiagnosticRequest {
            text: "xin chao".into(),
            voice: Some("maichi".into()),
            language: Some("vi-VN".into()),
        }]
    );
    task.abort();
}

#[tokio::test]
async fn asr_provider_test_accepts_bounded_pcm_wav_and_rejects_other_media() {
    let (database_uri, key) = seeded_provider(serde_json::json!({
        "name":"Loaded ASR", "type":"asr", "adapter":"gipformer_sherpa_offline",
        "config_json":{"language":"vi-VN","decoding_method":"greedy_search","max_active_paths":4}
    }))
    .await;
    let (base, received_samples, task) = server_with_loaded_asr(&database_uri, &key).await;
    let client = Client::new();
    let providers = format!("{base}/api/admin/providers");
    let url = format!("{providers}/{key}/test/asr");

    let response = client
        .post(&url)
        .bearer_auth("admin-test-token")
        .header("content-type", "audio/wav")
        .body(wav_pcm16_mono(16_000, 16_000))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let response = response.json::<serde_json::Value>().await.unwrap();
    assert_eq!(response["provider_key"], key);
    assert_eq!(response["type"], "asr");
    assert_eq!(response["result"]["text"], "xin chao");
    assert_eq!(response["result"]["language"], "vi-VN");
    assert_eq!(response["metrics"]["audio_duration_ms"], 1_000);
    assert!(response["metrics"]["rtf"].is_number());
    assert_eq!(*received_samples.lock().unwrap(), vec![16_000]);

    for (content_type, body) in [
        ("audio/mpeg", wav_pcm16_mono(16_000, 1)),
        ("audio/wav", wav_pcm16_mono(8_000, 8_000)),
        ("audio/wav", wav_pcm16_mono(16_000, 16_000 * 31)),
    ] {
        let response = client
            .post(&url)
            .bearer_auth("admin-test-token")
            .header("content-type", content_type)
            .body(body)
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        assert_eq!(
            response.json::<serde_json::Value>().await.unwrap()["error"]["code"],
            "invalid_test_input"
        );
    }
    let response = client
        .post(&url)
        .bearer_auth("admin-test-token")
        .header("content-type", "audio/wav")
        .body(vec![0; 5 * 1024 * 1024 + 1])
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
    assert_eq!(
        response.json::<serde_json::Value>().await.unwrap()["error"]["code"],
        "request_too_large"
    );
    task.abort();
}

#[tokio::test]
async fn speaker_provider_creation_is_rejected() {
    let (base, task) = server(true).await;
    let client = Client::new();
    let response = client
        .post(format!("{base}/api/admin/providers"))
        .bearer_auth("admin-test-token")
        .json(&serde_json::json!({"type":"speaker","adapter":"campplus_sherpa","name":"Voice","config_json":{}}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let body: serde_json::Value = response.json().await.unwrap();
    assert_eq!(body["error"]["code"], "validation_failed");
    task.abort();
}

#[tokio::test]
async fn speaker_provider_diagnostic_route_is_not_exposed() {
    let (base, task) = server(true).await;
    let response = Client::new()
        .post(format!("{base}/api/admin/providers/missing/test/speaker"))
        .bearer_auth("admin-test-token")
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    task.abort();
}

#[tokio::test]
async fn speaker_waiter_timeout_retains_exact_lease_and_capacity_until_native_completion() {
    use voice_agent_server::{
        database::DesiredProvider,
        providers::speaker::SpeakerError,
        services::provider_runtime::{ProviderRuntimeManager, RuntimeLimits},
    };
    let manager = ProviderRuntimeManager::new(
        RuntimeLimits {
            max_parallel_loads: 1,
            max_pending_loads: 2,
            max_waiters: 8,
            max_resident_bytes: 4,
            max_resources: 4,
            max_version_entries: 8,
            admission_timeout_ms: 1000,
            failure_cooldown_ms: 100,
            idle_ttl_ms: 1000,
        },
        Arc::new(QualificationSpeakerFactory { slow: true }),
        voice_agent_server::lifecycle::AdmissionGate::open(),
    )
    .unwrap();
    let snapshot = DesiredProvider {
        id: 1,
        key: "speaker_test".into(),
        kind: "speaker".into(),
        adapter: "campplus_sherpa".into(),
        config_json: "{}".into(),
        secret_ref: None,
        revision: 1,
    };
    let lease = manager.acquire(snapshot.clone()).await.unwrap();
    let runtime = lease.runtimes().unwrap().speaker(&snapshot.key).unwrap();
    let pcm = PcmF32Mono::new(vec![0.1; 16000], 16000);
    assert!(
        tokio::time::timeout(
            Duration::from_millis(20),
            runtime.extract(pcm.clone(), lease)
        )
        .await
        .is_err()
    );
    assert_eq!(manager.accounting().active_leases, 1);
    let second = manager.acquire(snapshot).await.unwrap();
    assert!(matches!(
        runtime.extract(pcm, second).await,
        Err(SpeakerError::Busy)
    ));
    tokio::time::sleep(Duration::from_millis(250)).await;
    assert_eq!(manager.accounting().active_leases, 0);
    assert_eq!(manager.accounting().physical_inference_usage, 0);
}

#[tokio::test]
async fn draft_tests_reject_invalid_sources_without_creating_resources() {
    let (base, task) = server(true).await;
    let client = Client::new();
    for (route, body) in [
        (
            "provider-tests/llm",
            serde_json::json!({"provider":{"type":"llm","adapter":"openai","config_json":{"api_key":"must-not-leak"}},"input":{"text":"hello"}}),
        ),
        (
            "mcp-tests/connection",
            serde_json::json!({"server":{"key":"test","url":"http://localhost/mcp","auth":{"type":"none"},"api_key":"must-not-leak"}}),
        ),
    ] {
        let response = client
            .post(format!("{base}/api/admin/{route}"))
            .bearer_auth("admin-test-token")
            .json(&body)
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST, "{route}");
        assert!(!response.text().await.unwrap().contains("must-not-leak"));
    }
    for resource in ["providers", "mcp-servers"] {
        let response: serde_json::Value = client
            .get(format!("{base}/api/admin/{resource}"))
            .bearer_auth("admin-test-token")
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        if resource == "providers" {
            assert!(
                response["items"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .all(|item| item["source"] == "config")
            );
        } else {
            assert!(response["items"].as_array().unwrap().is_empty());
        }
    }
    task.abort();
}

#[tokio::test]
async fn draft_asr_rejects_ambiguous_multipart_and_invalid_wav_before_runtime() {
    let (base, task) = server(true).await;
    let client = Client::new();
    let provider = r#"{"type":"asr","adapter":"zipformer_sherpa","config_json":{}}"#;
    let malformed = reqwest::multipart::Form::new()
        .text("provider", provider)
        .text("provider", provider);
    let response = client
        .post(format!("{base}/api/admin/provider-tests/asr"))
        .bearer_auth("admin-test-token")
        .multipart(malformed)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let form = reqwest::multipart::Form::new()
        .text("provider", provider)
        .part(
            "audio",
            reqwest::multipart::Part::bytes(vec![0; 44])
                .mime_str("audio/wav")
                .unwrap(),
        );
    let response = client
        .post(format!("{base}/api/admin/provider-tests/asr"))
        .bearer_auth("admin-test-token")
        .multipart(form)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let form = reqwest::multipart::Form::new()
        .text("provider", provider)
        .part(
            "audio",
            reqwest::multipart::Part::bytes(vec![0; 5 * 1024 * 1024 + 1])
                .mime_str("audio/wav")
                .unwrap(),
        );
    let response = client
        .post(format!("{base}/api/admin/provider-tests/asr"))
        .bearer_auth("admin-test-token")
        .multipart(form)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
    task.abort();
}

#[tokio::test]
async fn system_resources_are_authenticated_uncached_and_independent_of_database() {
    let client = Client::new();
    let (base, task) = resource_server(true, true).await;
    let url = format!("{base}/api/admin/system/resources");
    for token in [None, Some("wrong-token")] {
        let request = client.get(&url);
        let response = match token {
            Some(token) => request.bearer_auth(token),
            None => request,
        }
        .send()
        .await
        .unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }
    let response = client
        .get(&url)
        .bearer_auth("admin-test-token")
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()["cache-control"], "no-store");
    assert!(response.headers().contains_key("x-request-id"));
    let snapshot: serde_json::Value = response.json().await.unwrap();
    assert_eq!(snapshot["sample_interval_ms"], 5000);
    assert_eq!(snapshot["process"]["pid"], std::process::id());
    assert!(chrono::DateTime::parse_from_rfc3339(snapshot["sampled_at"].as_str().unwrap()).is_ok());
    assert_eq!(snapshot.as_object().unwrap().len(), 5);
    assert!(snapshot["system"]["cpu_percent"].is_null());
    assert!(snapshot["process"]["cpu_percent"].is_null());
    assert_eq!(snapshot["storage"].as_object().unwrap().len(), 2);
    assert_eq!(
        client
            .get(format!("{base}/health"))
            .send()
            .await
            .unwrap()
            .text()
            .await
            .unwrap(),
        "ok"
    );
    task.abort();

    let (base, task) = resource_server(true, false).await;
    let response = client
        .get(format!("{base}/api/admin/system/resources"))
        .bearer_auth("admin-test-token")
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    let body: serde_json::Value = response.json().await.unwrap();
    assert_eq!(body["error"]["code"], "system_metrics_unavailable");
    assert!(body["error"]["request_id"].is_string());
    task.abort();

    let (base, task) = resource_server(false, false).await;
    assert_eq!(
        client
            .get(format!("{base}/api/admin/system/resources"))
            .bearer_auth("admin-test-token")
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::NOT_FOUND
    );
    task.abort();
}
