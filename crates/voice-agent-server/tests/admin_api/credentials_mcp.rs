use super::harness::*;

#[tokio::test]
async fn external_mcp_http_can_be_created_and_updated_without_network_configuration() {
    let (base, task) = server(true).await;
    let client = Client::new();
    let servers = format!("{base}/api/admin/mcp-servers");
    let created = client
        .post(&servers)
        .bearer_auth("admin-test-token")
        .json(&serde_json::json!({
            "key": "http_mcp", "name": "HTTP MCP",
            "url": "http://192.168.1.157:8080/mcp", "auth": {"type": "none"}
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(created.status(), StatusCode::CREATED);
    let updated = client
        .patch(format!("{servers}/http_mcp"))
        .bearer_auth("admin-test-token")
        .header("if-match", "\"1\"")
        .json(&serde_json::json!({"url": "http://127.0.0.1:8080/mcp"}))
        .send()
        .await
        .unwrap();
    assert_eq!(updated.status(), StatusCode::OK);
    assert_eq!(
        updated.json::<serde_json::Value>().await.unwrap()["url"],
        "http://127.0.0.1:8080/mcp"
    );
    task.abort();
}

#[tokio::test]
async fn external_mcp_configuration_is_redacted_validated_and_revisioned() {
    let (base, task) = server(true).await;
    let client = Client::new();
    let auth = "admin-test-token";
    assert_eq!(
        client
            .post(format!("{base}/api/admin/agents"))
            .bearer_auth(auth)
            .json(&serde_json::json!({"key":"kitchen","name":"Kitchen"}))
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::CREATED
    );
    let servers = format!("{base}/api/admin/mcp-servers");
    assert_eq!(
        client
            .post(&servers)
            .bearer_auth(auth)
            .json(&serde_json::json!({"key":"outside","name":"Outside","url":"ftp://outside.example.test/","auth":{"type":"none"}}))
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::BAD_REQUEST
    );
    assert_eq!(client.post(&servers).bearer_auth(auth).json(&serde_json::json!({"key":"bad","name":"Bad","url":"https://mcp.example.test/?token=secret","headers":{"authorization":"nope"},"auth":{"type":"none"}})).send().await.unwrap().status(), StatusCode::BAD_REQUEST);
    let created = client.post(&servers).bearer_auth(auth).json(&serde_json::json!({"key":"weather","name":"Weather","url":"https://mcp.example.test/tools","auth":{"type":"header","header_name":"x-api-key"}})).send().await.unwrap();
    assert_eq!(created.status(), StatusCode::CREATED);
    let server: serde_json::Value = created.json().await.unwrap();
    assert_eq!(
        server["auth"],
        serde_json::json!({"type":"header","header_name":"x-api-key"})
    );
    assert!(server.get("secret_ref").is_none());
    assert_eq!(server["credential_env"], "VOICE_MCP_WEATHER_TOKEN");
    // Legacy secret references and arbitrary headers remain rejected.
    for rejected in [
        serde_json::json!({"key":"legacy","name":"Legacy MCP","url":"https://mcp.example.test/mcp","auth":{"type":"bearer","secret_ref":"LEGACY_TOKEN"}}),
        serde_json::json!({"key":"legacy_none","name":"Legacy MCP","url":"https://mcp.example.test/mcp","auth":{"type":"none","secret_ref":"LEGACY_TOKEN"}}),
        serde_json::json!({"key":"legacy_header","name":"Legacy MCP","url":"https://mcp.example.test/mcp","auth":{"type":"header","header_name":"x-api-key","secret_ref":"LEGACY_TOKEN"}}),
        serde_json::json!({"key":"headers","name":"Headers MCP","url":"https://mcp.example.test/mcp","headers":{"x-api-key":"plaintext"},"auth":{"type":"none"}}),
    ] {
        assert_eq!(
            client
                .post(&servers)
                .bearer_auth(auth)
                .json(&rejected)
                .send()
                .await
                .unwrap()
                .status(),
            StatusCode::BAD_REQUEST
        );
    }
    assert_eq!(
        client
            .post(&servers)
            .bearer_auth(auth)
            .json(&serde_json::json!({"key":"unsafe_auth","name":"Unsafe auth","url":"https://mcp.example.test/","auth":{"type":"header","header_name":"authorization"}}))
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::BAD_REQUEST
    );
    for legacy_auth in [
        serde_json::json!({"type":"none","secret_ref":"LEGACY_TOKEN"}),
        serde_json::json!({"type":"bearer","secret_ref":"LEGACY_TOKEN"}),
        serde_json::json!({"type":"header","header_name":"x-api-key","secret_ref":"LEGACY_TOKEN"}),
    ] {
        assert_eq!(
            client
                .patch(format!("{servers}/weather"))
                .bearer_auth(auth)
                .header("if-match", "\"1\"")
                .json(&serde_json::json!({"auth":legacy_auth}))
                .send()
                .await
                .unwrap()
                .status(),
            StatusCode::BAD_REQUEST
        );
    }
    for header in ["proxy-authenticate", "www-authenticate", "keep-alive"] {
        assert_eq!(
            client
                .post(&servers)
                .bearer_auth(auth)
                .json(&serde_json::json!({"key":format!("blocked_{}", header.replace('-', "_")),"name":"Blocked","url":"https://mcp.example.test/","headers":{header:"value"},"auth":{"type":"none"}}))
                .send()
                .await
                .unwrap()
                .status(),
            StatusCode::BAD_REQUEST,
            "{header} must be protected"
        );
    }
    let binding = client
        .put(format!(
            "{base}/api/admin/agents/kitchen/mcp-bindings/weather"
        ))
        .bearer_auth(auth)
        .header("if-match", "\"1\"")
        .json(&serde_json::json!({"enabled":true,"required":false}))
        .send()
        .await
        .unwrap();
    assert_eq!(binding.status(), StatusCode::OK);
    assert_eq!(
        binding.json::<serde_json::Value>().await.unwrap()["revision"],
        2
    );
    assert_eq!(
        client
            .put(format!(
                "{base}/api/admin/agents/kitchen/mcp-bindings/weather"
            ))
            .bearer_auth(auth)
            .header("if-match", "\"1\"")
            .json(&serde_json::json!({"enabled":false,"required":false}))
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::CONFLICT
    );
    assert_eq!(
        client
            .put(format!(
                "{base}/api/admin/agents/kitchen/mcp-bindings/weather"
            ))
            .bearer_auth(auth)
            .header("if-match", "\"2\"")
            .json(&serde_json::json!({"enabled":true,"required":true}))
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::BAD_REQUEST
    );

    let mcp_in_use = client
        .delete(format!("{base}/api/admin/mcp-servers/weather"))
        .bearer_auth(auth)
        .header("if-match", "\"1\"")
        .send()
        .await
        .unwrap();
    assert_eq!(mcp_in_use.status(), StatusCode::CONFLICT);
    assert_eq!(
        mcp_in_use.json::<serde_json::Value>().await.unwrap()["error"]["code"],
        "mcp_server_in_use"
    );
    let missing_unlink_match = client
        .delete(format!(
            "{base}/api/admin/agents/kitchen/mcp-bindings/weather"
        ))
        .bearer_auth(auth)
        .send()
        .await
        .unwrap();
    assert_eq!(missing_unlink_match.status(), StatusCode::BAD_REQUEST);
    assert_eq!(
        client
            .delete(format!(
                "{base}/api/admin/agents/kitchen/mcp-bindings/weather"
            ))
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
            .delete(format!("{base}/api/admin/mcp-servers/weather"))
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
async fn provider_credentials_require_encryption_and_never_create_a_partial_resource() {
    let (base, task) = server(true).await;
    let client = Client::new();
    let response = client.post(format!("{base}/api/admin/providers"))
        .bearer_auth("admin-test-token")
        .json(&serde_json::json!({"name":"Encrypted OpenAI","type":"llm","adapter":"openai","config_json":{"base_url":"https://example.test/v1","model":"test-model","max_tokens":8},"api_key":"sk-test-never-plaintext-7A91"}))
        .send().await.unwrap();
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    let body = response.text().await.unwrap();
    assert!(!body.contains("sk-test-never-plaintext-7A91"));
    let page: serde_json::Value = client
        .get(format!("{base}/api/admin/providers"))
        .bearer_auth("admin-test-token")
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(
        page["items"]
            .as_array()
            .unwrap()
            .iter()
            .all(|item| item["source"] == "config")
    );
    task.abort();
}

#[tokio::test]
async fn provider_credentials_are_write_only_masked_and_persist_after_restart() {
    let uri = database_url();
    let (base, task) = server_with_resolver(
        true,
        &uri,
        Arc::new(
            voice_agent_server::database::credentials::CredentialCipher::new(1, &[7; 32]).unwrap(),
        ),
    )
    .await;
    let client = Client::new();
    let response = client.post(format!("{base}/api/admin/providers"))
        .bearer_auth("admin-test-token")
        .json(&serde_json::json!({"name":"Encrypted OpenAI","type":"llm","adapter":"openai","config_json":{"base_url":"https://example.test/v1","model":"test-model","max_tokens":8},"api_key":"sk-test-never-plaintext-7A91"}))
        .send().await.unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
    let body = response.text().await.unwrap();
    assert!(!body.contains("sk-test-never-plaintext-7A91"));
    let created: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(created["credential"]["masked_key"], "sk-...7A91");
    let key = created["key"].as_str().unwrap();
    for (path, payload) in [
        (
            "agents",
            serde_json::json!({"key":"cipher_agent","name":"Cipher Agent"}),
        ),
        (
            "templates",
            serde_json::json!({"key":"cipher_template","name":"Cipher Template","language":"vi-VN","prompt":"Short"}),
        ),
    ] {
        assert_eq!(
            client
                .post(format!("{base}/api/admin/{path}"))
                .bearer_auth("admin-test-token")
                .json(&payload)
                .send()
                .await
                .unwrap()
                .status(),
            StatusCode::CREATED
        );
    }
    assert_eq!(
        client
            .put(format!(
                "{base}/api/admin/templates/cipher_template/providers/llm"
            ))
            .bearer_auth("admin-test-token")
            .header("if-match", "\"1\"")
            .json(&serde_json::json!({"provider_key":key}))
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::OK
    );
    assert_eq!(
        client
            .put(format!(
                "{base}/api/admin/agents/cipher_agent/default-template/cipher_template"
            ))
            .bearer_auth("admin-test-token")
            .header("if-match", "\"1\"")
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::OK
    );
    task.abort();
    let (base, task) = server_with_resolver(
        true,
        &uri,
        Arc::new(
            voice_agent_server::database::credentials::CredentialCipher::new(1, &[7; 32]).unwrap(),
        ),
    )
    .await;
    let body = client
        .get(format!("{base}/api/admin/providers/{key}"))
        .bearer_auth("admin-test-token")
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    assert!(!body.contains("sk-test-never-plaintext-7A91"));
    let fetched: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(fetched["credential"], created["credential"]);
    assert!(fetched.get("api_key").is_none());
    assert_eq!(fetched["runtime_status"], "loaded");
    task.abort();
}

#[tokio::test]
async fn provider_credentials_replace_only_when_supplied() {
    let (base, task) = server_with_resolver(
        true,
        &database_url(),
        Arc::new(
            voice_agent_server::database::credentials::CredentialCipher::new(1, &[7; 32]).unwrap(),
        ),
    )
    .await;
    let client = Client::new();
    let key = create_provider(&client, &format!("{base}/api/admin/providers"), serde_json::json!({"name":"LLM","type":"llm","adapter":"openai","config_json":{"base_url":"https://example.test/v1","model":"test","max_tokens":8},"api_key":"sk-first-secret-1111"})).await;
    let url = format!("{base}/api/admin/providers/{key}");
    for (revision, patch, hint) in [
        (
            1,
            serde_json::json!({"api_key":"sk-second-secret-2222"}),
            "sk-...2222",
        ),
        (2, serde_json::json!({"name":"Renamed"}), "sk-...2222"),
    ] {
        let response = client
            .patch(&url)
            .bearer_auth("admin-test-token")
            .header("if-match", format!("\"{revision}\""))
            .json(&patch)
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let value: serde_json::Value = response.json().await.unwrap();
        assert_eq!(value["credential"]["masked_key"], hint);
        assert!(value.get("api_key").is_none());
    }
    for value in [
        serde_json::Value::Null,
        serde_json::json!(""),
        serde_json::json!("has whitespace"),
    ] {
        assert_eq!(
            client
                .patch(&url)
                .bearer_auth("admin-test-token")
                .header("if-match", "\"3\"")
                .json(&serde_json::json!({"api_key":value}))
                .send()
                .await
                .unwrap()
                .status(),
            StatusCode::BAD_REQUEST
        );
    }
    let value: serde_json::Value = client
        .get(&url)
        .bearer_auth("admin-test-token")
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(value["revision"], 3);
    assert_eq!(value["credential"]["masked_key"], "sk-...2222");
    task.abort();
}

#[tokio::test]
async fn mcp_credentials_require_encryption_and_never_create_a_partial_resource() {
    use voice_agent_server::database::secrets::{
        SecretRef, SecretResolveError, SecretResolver, SecretValue,
    };
    struct UnavailableSecrets;
    impl SecretResolver for UnavailableSecrets {
        fn resolve(&self, _: &SecretRef) -> Result<SecretValue, SecretResolveError> {
            Err(SecretResolveError::Unavailable)
        }
    }
    let (base, task) =
        server_with_resolver(true, &database_url(), Arc::new(UnavailableSecrets)).await;
    let client = Client::new();
    let servers = format!("{base}/api/admin/mcp-servers");
    let response = client
        .post(&servers)
        .bearer_auth("admin-test-token")
        .json(&serde_json::json!({
            "key":"gateway", "name":"Gateway", "url":"http://192.168.1.157:8080/mcp",
            "auth":{"type":"bearer"}, "api_key":"mcp-test-never-plaintext-7A91"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    let body = response.text().await.unwrap();
    assert!(!body.contains("mcp-test-never-plaintext-7A91"));
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&body).unwrap()["error"]["code"],
        "credential_storage_unavailable"
    );
    let page: serde_json::Value = client
        .get(&servers)
        .bearer_auth("admin-test-token")
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(page["items"].as_array().unwrap().is_empty());
    task.abort();
}

#[tokio::test]
async fn mcp_credentials_are_write_only_replaceable_and_persist_after_restart() {
    let uri = database_url();
    let cipher = || {
        Arc::new(
            voice_agent_server::database::credentials::CredentialCipher::new(1, &[7; 32]).unwrap(),
        ) as Arc<dyn voice_agent_server::database::secrets::SecretResolver>
    };
    let (base, task) = server_with_resolver(true, &uri, cipher()).await;
    let client = Client::new();
    let response = client.post(format!("{base}/api/admin/mcp-servers")).bearer_auth("admin-test-token")
        .json(&serde_json::json!({"key":"weather","name":"Weather","url":"https://mcp.example.test/mcp","auth":{"type":"bearer"},"api_key":"mcp-test-secret-7A91"})).send().await.unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
    let body = response.text().await.unwrap();
    assert!(!body.contains("mcp-test-secret-7A91"));
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&body).unwrap()["credential"]["masked_key"],
        "…7A91"
    );
    task.abort();
    let (base, task) = server_with_resolver(true, &uri, cipher()).await;
    let url = format!("{base}/api/admin/mcp-servers/weather");
    let value: serde_json::Value = client
        .get(&url)
        .bearer_auth("admin-test-token")
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(value["credential"]["masked_key"], "…7A91");
    for (revision, patch, hint) in [
        (
            1,
            serde_json::json!({"api_key":"mcp-replaced-secret-2222"}),
            serde_json::json!("…2222"),
        ),
        (
            2,
            serde_json::json!({"name":"Renamed"}),
            serde_json::json!("…2222"),
        ),
        (
            3,
            serde_json::json!({"auth":{"type":"none"}}),
            serde_json::Value::Null,
        ),
    ] {
        let response = client
            .patch(&url)
            .bearer_auth("admin-test-token")
            .header("if-match", format!("\"{revision}\""))
            .json(&patch)
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = response.text().await.unwrap();
        assert!(!body.contains("mcp-replaced-secret-2222"));
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&body).unwrap()["credential"]["masked_key"],
            hint
        );
    }
    assert_eq!(
        client
            .patch(&url)
            .bearer_auth("admin-test-token")
            .header("if-match", "\"4\"")
            .json(&serde_json::json!({"api_key":"mcp-rejected-secret-3333"}))
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::BAD_REQUEST
    );
    task.abort();
}
