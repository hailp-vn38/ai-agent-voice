use std::sync::Arc;
use std::time::Duration;

use super::harness::*;

#[tokio::test]
async fn external_agent_reviews_exact_contract_and_drift_requires_new_review() {
    let server = start_mcp(McpBehaviour::with_tools(vec![tool("Light")])).await;
    let mut configuration = config(database_url());
    configuration.api.enabled = true;
    configuration.api.jwt_secret =
        vela_server::config::SecretString::new("external-mcp-probes-signing-secret-0123456789");
    let voice = start_with_config(configuration, ConstantSecrets(None)).await;
    let admin = crate::admin_auth::admin_web_token(&reqwest::Client::new(), &voice.base).await;
    let pool = voice.database().await;
    seed(&pool).await;
    bind_server(&pool, "home", &server.url(), "{}", ("none", None, None)).await;
    assert!(names(voice.admit().await.external_mcp()).is_empty());
    let client = reqwest::Client::new();
    let url = format!("{}/api/admin/agents/agent/tool-allowlist", voice.base);
    let items = client
        .get(&url)
        .bearer_auth(&admin)
        .send()
        .await
        .unwrap()
        .json::<serde_json::Value>()
        .await
        .unwrap();
    let observation = &items["items"][0];
    assert_eq!(observation["allowed"], false);
    let mut review = serde_json::json!({"server_key":"home","original_name":"Light","observed_revision":observation["observed_revision"],"fingerprint":observation["fingerprint"],"allowed":true,"sensitive":false});
    let response = client
        .put(&url)
        .bearer_auth(&admin)
        .header("If-Match", "\"1\"")
        .json(&review)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), reqwest::StatusCode::OK);
    assert_eq!(
        names(voice.admit().await.external_mcp()),
        vec!["external.home.light"]
    );
    use futures_util::{SinkExt, StreamExt};
    use tokio_tungstenite::tungstenite::Message;
    let mut request = format!("{}/voice/v1/", voice.base.replacen("http", "ws", 1))
        .into_client_request()
        .unwrap();
    request
        .headers_mut()
        .insert("Protocol-Version", "1".parse().unwrap());
    request
        .headers_mut()
        .insert("Device-Id", "device".parse().unwrap());
    request
        .headers_mut()
        .insert("Client-Id", "review-test".parse().unwrap());
    let (mut socket, _) = connect_async(request).await.unwrap();
    socket.send(Message::Text(serde_json::json!({"type":"hello","version":1,"transport":"websocket","audio_params":{"format":"opus","sample_rate":16000,"channels":1,"frame_duration":60}}).to_string().into())).await.unwrap();
    assert!(matches!(
        socket.next().await.unwrap().unwrap(),
        Message::Text(_)
    ));
    review["sensitive"] = serde_json::json!(true);
    assert_eq!(
        client
            .put(&url)
            .bearer_auth(&admin)
            .header("If-Match", "\"1\"")
            .json(&review)
            .send()
            .await
            .unwrap()
            .status(),
        reqwest::StatusCode::CONFLICT
    );
    assert_eq!(
        client
            .put(&url)
            .bearer_auth(&admin)
            .header("If-Match", "\"2\"")
            .json(&review)
            .send()
            .await
            .unwrap()
            .status(),
        reqwest::StatusCode::OK
    );
    let close = timeout(Duration::from_secs(2), async {
        loop {
            if let Some(Ok(Message::Close(Some(frame)))) = socket.next().await {
                break frame.code;
            }
        }
    })
    .await
    .unwrap();
    assert_eq!(u16::from(close), 1008);
    assert!(names(voice.admit().await.external_mcp()).is_empty());
    server.behaviour.lock().unwrap().tools[0]["description"] =
        serde_json::json!("changed contract");
    assert!(names(voice.admit().await.external_mcp()).is_empty());
    assert_eq!(
        client
            .put(&url)
            .bearer_auth(&admin)
            .header("If-Match", "\"3\"")
            .json(&review)
            .send()
            .await
            .unwrap()
            .status(),
        reqwest::StatusCode::CONFLICT
    );
    voice.task.abort();
    server.task.abort();
}

#[tokio::test]
async fn admin_mcp_credential_is_decrypted_for_requests_after_restart_and_rotation() {
    let server = start_mcp(McpBehaviour::with_tools(vec![tool("Light")])).await;
    let mut configuration = config(database_url());
    configuration.api.enabled = true;
    configuration.api.jwt_secret =
        vela_server::config::SecretString::new("external-mcp-probes-signing-secret-0123456789");
    let cipher = || vela_server::database::credentials::CredentialCipher::new(1, &[7; 32]).unwrap();
    let voice = start_with_config(configuration.clone(), cipher()).await;
    let admin = crate::admin_auth::admin_web_token(&reqwest::Client::new(), &voice.base).await;
    let pool = voice.database().await;
    seed(&pool).await;
    let client = reqwest::Client::new();
    let response = client.post(format!("{}/api/admin/mcp-servers",voice.base)).bearer_auth(&admin).json(&serde_json::json!({"key":"home","name":"Home","url":server.url(),"auth":{"type":"bearer"},"api_key":"mcp-secret-before-1111"})).send().await.unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
    assert_eq!(
        client
            .put(format!(
                "{}/api/admin/agents/agent/mcp-bindings/home",
                voice.base
            ))
            .bearer_auth(&admin)
            .header("if-match", "\"1\"")
            .json(&serde_json::json!({"enabled":true,"required":false}))
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::OK
    );
    voice.task.abort();
    let voice = start_with_config(configuration.clone(), cipher()).await;
    let admin = crate::admin_auth::admin_web_token(&reqwest::Client::new(), &voice.base).await;
    voice.admit().await;
    assert!(!server.seen().is_empty());
    assert!(server.seen().iter().all(
        |(authorization, _)| authorization.as_deref() == Some("Bearer mcp-secret-before-1111")
    ));
    let review_url = format!("{}/api/admin/agents/agent/tool-allowlist", voice.base);
    let observations: serde_json::Value = client
        .get(&review_url)
        .bearer_auth(&admin)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let observation = &observations["items"][0];
    assert!(observation["source"]["auth_reference"].is_null());
    assert!(!observations.to_string().contains("mcp-secret-before-1111"));
    assert_eq!(client.put(&review_url).bearer_auth(&admin).header("if-match",format!("\"{}\"",observation["revision"])).json(&serde_json::json!({"server_key":"home","original_name":"Light","observed_revision":observation["observed_revision"],"fingerprint":observation["fingerprint"],"allowed":true,"sensitive":false})).send().await.unwrap().status(),StatusCode::OK);
    assert_eq!(
        names(voice.admit().await.external_mcp()),
        vec!["external.home.light"]
    );
    server.rewrite(|script| script.seen.lock().unwrap().clear());
    assert_eq!(
        client
            .patch(format!("{}/api/admin/mcp-servers/home", voice.base))
            .bearer_auth(&admin)
            .header("if-match", "\"1\"")
            .json(&serde_json::json!({"api_key":"mcp-secret-after-2222"}))
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::OK
    );
    assert!(
        names(voice.admit().await.external_mcp()).is_empty(),
        "credential rotation requires tool review again"
    );
    assert!(!server.seen().is_empty());
    assert!(
        server
            .seen()
            .iter()
            .all(|(authorization, _)| authorization.as_deref()
                == Some("Bearer mcp-secret-after-2222"))
    );
    voice.task.abort();
    for invalid_cipher in [
        vela_server::database::credentials::CredentialCipher::new(1, &[8; 32]).unwrap(),
        vela_server::database::credentials::CredentialCipher::new(2, &[7; 32]).unwrap(),
    ] {
        server.rewrite(|script| script.seen.lock().unwrap().clear());
        let voice = start_with_config(configuration.clone(), invalid_cipher).await;
        assert!(names(voice.admit().await.external_mcp()).is_empty());
        assert!(
            server.seen().is_empty(),
            "wrong key/tag or unknown version must fail before network access"
        );
        voice.task.abort();
    }
    let voice = start_with_config(configuration.clone(), cipher()).await;
    let admin = crate::admin_auth::admin_web_token(&reqwest::Client::new(), &voice.base).await;
    assert_eq!(client.post(format!("{}/api/admin/mcp-servers",voice.base)).bearer_auth(&admin).json(&serde_json::json!({"key":"donor","name":"Donor","url":server.url(),"auth":{"type":"bearer"},"api_key":"mcp-donor-secret-3333"})).send().await.unwrap().status(),StatusCode::CREATED);
    // Fault injection: copying a valid encrypted record to another owner must fail its AAD check.
    sqlx::query("UPDATE mcp_servers SET credential_json=(SELECT credential_json FROM mcp_servers WHERE key='donor') WHERE key='home'").execute(&pool).await.unwrap();
    voice.task.abort();
    server.rewrite(|script| script.seen.lock().unwrap().clear());
    let voice = start_with_config(configuration, cipher()).await;
    assert!(names(voice.admit().await.external_mcp()).is_empty());
    assert!(
        server.seen().is_empty(),
        "encrypted records cannot be moved between resources"
    );
    voice.task.abort();
    server.task.abort();
}

#[tokio::test]
async fn admin_draft_connection_does_not_list_or_call_tools_and_discovery_is_complete() {
    for session_id in [None, Some("probe-session".to_owned())] {
        let behaviour = McpBehaviour {
            session_id,
            page_size: 1,
            ..McpBehaviour::with_tools(vec![tool("Light"), tool("Dim")])
        };
        let observed = behaviour.sessions.clone();
        let calls = behaviour.calls.clone();
        let server = start_mcp(behaviour).await;
        let mut configuration = config(database_url());
        configuration.api.enabled = true;
        configuration.api.jwt_secret =
            vela_server::config::SecretString::new("external-mcp-probes-signing-secret-0123456789");
        let voice = start_with_config(configuration, ConstantSecrets(None)).await;
        let admin = crate::admin_auth::admin_web_token(&reqwest::Client::new(), &voice.base).await;
        let client = reqwest::Client::new();
        let draft = serde_json::json!({"server":{"key":"home","url":server.url(),"auth":{"type":"bearer"},"api_key":"ephemeral-probe-key"}});
        let connection = client
            .post(format!("{}/api/admin/mcp-tests/connection", voice.base))
            .bearer_auth(&admin)
            .json(&draft)
            .send()
            .await
            .unwrap();
        assert_eq!(connection.status(), StatusCode::OK);
        let result: serde_json::Value = connection.json().await.unwrap();
        assert_eq!(result["connected_at_test_time"], true);
        assert!(result.get("tools").is_none());
        assert!(
            !observed
                .lock()
                .unwrap()
                .iter()
                .any(|(method, _)| method == "tools/list")
        );
        let discovery = client
            .post(format!("{}/api/admin/mcp-tests/discover", voice.base))
            .bearer_auth(&admin)
            .json(&draft)
            .send()
            .await
            .unwrap();
        assert_eq!(discovery.status(), StatusCode::OK);
        let result: serde_json::Value = discovery.json().await.unwrap();
        assert_eq!(result["complete"], true);
        assert_eq!(result["tools"].as_array().unwrap().len(), 2);
        assert_eq!(result["tools"][0]["llm_name"], "external.home.light");
        assert!(!result.to_string().contains("ephemeral-probe-key"));
        assert_eq!(calls.lock().unwrap().len(), 0);
        let resources: serde_json::Value = client
            .get(format!("{}/api/admin/mcp-servers", voice.base))
            .bearer_auth(&admin)
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        assert!(resources["items"].as_array().unwrap().is_empty());
        voice.task.abort();
        server.task.abort();
    }
}

#[tokio::test]
async fn manual_saved_probes_keep_disabled_config_and_credentials_and_reject_stale_reuse() {
    let server = start_mcp(McpBehaviour::default()).await;
    let mut configuration = config(database_url());
    configuration.api.enabled = true;
    configuration.api.jwt_secret =
        vela_server::config::SecretString::new("external-mcp-probes-signing-secret-0123456789");
    let cipher = vela_server::database::credentials::CredentialCipher::new(1, &[7; 32]).unwrap();
    let voice = start_with_config(configuration, cipher).await;
    let admin = crate::admin_auth::admin_web_token(&reqwest::Client::new(), &voice.base).await;
    let client = reqwest::Client::new();
    let create = client.post(format!("{}/api/admin/mcp-servers", voice.base)).bearer_auth(&admin).json(&serde_json::json!({"key":"home","name":"Home","url":server.url(),"auth":{"type":"bearer"},"api_key":"saved-probe-secret"})).send().await.unwrap();
    assert_eq!(create.status(), StatusCode::CREATED);
    let disable = client
        .patch(format!("{}/api/admin/mcp-servers/home", voice.base))
        .bearer_auth(&admin)
        .header("if-match", "\"1\"")
        .json(&serde_json::json!({"enabled":false}))
        .send()
        .await
        .unwrap();
    assert_eq!(disable.status(), StatusCode::OK);
    let before: serde_json::Value = disable.json().await.unwrap();
    for route in ["connection", "discover"] {
        let response = client
            .post(format!(
                "{}/api/admin/mcp-servers/home/test/{route}",
                voice.base
            ))
            .bearer_auth(&admin)
            .json(&serde_json::json!({}))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let result: serde_json::Value = response.json().await.unwrap();
        assert_eq!(result["test_source"], "saved");
        if route == "discover" {
            assert_eq!(result["tools"], serde_json::json!([]));
            assert_eq!(result["complete"], true);
        }
        assert!(!result.to_string().contains("saved-probe-secret"));
    }
    assert!(
        server
            .behaviour
            .lock()
            .unwrap()
            .seen()
            .iter()
            .any(|(auth, _)| auth.as_deref() == Some("Bearer saved-probe-secret"))
    );
    let traffic = server.behaviour.lock().unwrap().seen().len();
    for (credential, status) in [
        (
            serde_json::json!({"key":"home","expected_revision":1}),
            StatusCode::CONFLICT,
        ),
        (
            serde_json::json!({"key":"other","expected_revision":2}),
            StatusCode::BAD_REQUEST,
        ),
    ] {
        let response = client.post(format!("{}/api/admin/mcp-tests/connection",voice.base)).bearer_auth(&admin).json(&serde_json::json!({"server":{"key":"home","url":server.url(),"auth":{"type":"bearer"},"saved_credential":credential}})).send().await.unwrap();
        assert_eq!(response.status(), status);
    }
    assert_eq!(server.behaviour.lock().unwrap().seen().len(), traffic);
    let after: serde_json::Value = client
        .get(format!("{}/api/admin/mcp-servers/home", voice.base))
        .bearer_auth(&admin)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(before, after);
    assert_eq!(server.behaviour.lock().unwrap().call_count(), 0);
    voice.task.abort();
    server.task.abort();
}

#[tokio::test]
async fn manual_discovery_rejects_an_invalid_later_page_without_partial_catalog() {
    let server = start_mcp(McpBehaviour {
        page_size: 1,
        ..McpBehaviour::with_tools(vec![tool("Light"), recursive_tool()])
    })
    .await;
    let mut configuration = config(database_url());
    configuration.api.enabled = true;
    configuration.api.jwt_secret =
        vela_server::config::SecretString::new("external-mcp-probes-signing-secret-0123456789");
    let voice = start_with_config(configuration, ConstantSecrets(None)).await;
    let admin = crate::admin_auth::admin_web_token(&reqwest::Client::new(), &voice.base).await;
    let response = reqwest::Client::new()
        .post(format!("{}/api/admin/mcp-tests/discover", voice.base))
        .bearer_auth(&admin)
        .json(
            &serde_json::json!({"server":{"key":"home","url":server.url(),"auth":{"type":"none"}}}),
        )
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
    let result: serde_json::Value = response.json().await.unwrap();
    assert!(result.get("tools").is_none());
    assert_eq!(server.behaviour.lock().unwrap().call_count(), 0);
    voice.task.abort();
    server.task.abort();
}

#[tokio::test]
async fn manual_probe_admission_stays_bounded_after_waiter_cancellation_and_shutdown() {
    let gate = Arc::new(tokio::sync::Notify::new());
    let behaviour = McpBehaviour {
        initialize_gate: Some(gate.clone()),
        ..Default::default()
    };
    let observed = behaviour.sessions.clone();
    let server = start_mcp(behaviour).await;
    let mut configuration = config(database_url());
    configuration.api.enabled = true;
    configuration.api.jwt_secret =
        vela_server::config::SecretString::new("external-mcp-probes-signing-secret-0123456789");
    configuration.api.mcp_tests.max_concurrency = 1;
    let voice = start_with_config(configuration, ConstantSecrets(None)).await;
    let admin = crate::admin_auth::admin_web_token(&reqwest::Client::new(), &voice.base).await;
    let url = format!("{}/api/admin/mcp-tests/connection", voice.base);
    let draft =
        serde_json::json!({"server":{"key":"home","url":server.url(),"auth":{"type":"none"}}});
    let client = reqwest::Client::new();
    let request = client.post(&url).bearer_auth(&admin).json(&draft);
    let pending = tokio::spawn(async move { request.send().await });
    timeout(Duration::from_secs(1), async {
        while !observed
            .lock()
            .unwrap()
            .iter()
            .any(|(method, _)| method == "initialize")
        {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    pending.abort();
    let overloaded = client
        .post(&url)
        .bearer_auth(&admin)
        .json(&draft)
        .send()
        .await
        .unwrap();
    assert_eq!(overloaded.status(), StatusCode::TOO_MANY_REQUESTS);
    voice.state.lifecycle.gate().close();
    let stopping = client
        .post(&url)
        .bearer_auth(&admin)
        .json(&draft)
        .send()
        .await
        .unwrap();
    assert_eq!(stopping.status(), StatusCode::SERVICE_UNAVAILABLE);
    gate.notify_one();
    voice.task.abort();
    server.task.abort();
}
