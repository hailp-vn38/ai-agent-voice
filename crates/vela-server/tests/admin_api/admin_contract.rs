use super::super::*;

#[tokio::test]
async fn admin_router_auth_crud_revision_and_transport_contract() {
    let (base, task) = server(true).await;
    let client = Client::new();
    let web_token = admin_web_token(&client, &base).await;
    let url = format!("{base}/api/admin/agents");

    let unauthorized = client.get(&url).send().await.unwrap();
    assert_eq!(unauthorized.status(), StatusCode::UNAUTHORIZED);
    assert!(unauthorized.headers().contains_key("x-request-id"));
    assert_eq!(
        client
            .get(&url)
            .header("authorization", &format!("Bearer {web_token}"))
            .header("authorization", &format!("Bearer {web_token}"))
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        client
            .post(&url)
            .bearer_auth(&web_token)
            .header("content-encoding", "gzip")
            .header("content-type", "application/json")
            .body("{}")
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::UNSUPPORTED_MEDIA_TYPE
    );
    assert_eq!(
        client
            .post(&url)
            .bearer_auth(&web_token)
            .header("content-type", "application/json; nonsense")
            .body("{}")
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        client
            .get(format!("{base}/api/admin/agents?page_size=201"))
            .bearer_auth(&web_token)
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::BAD_REQUEST
    );

    let created = client
        .post(&url)
        .bearer_auth(&web_token)
        .json(&serde_json::json!({"key":"kitchen","name":"Kitchen"}))
        .send()
        .await
        .unwrap();
    assert_eq!(created.status(), StatusCode::CREATED);
    let agent: serde_json::Value = created.json().await.unwrap();
    assert_eq!(agent["key"], "kitchen");
    assert_eq!(agent["revision"], 1);

    let device_url = format!("{base}/api/admin/devices");
    let device = client.post(&device_url).bearer_auth(&web_token).json(&serde_json::json!({"device_id":"device-1","agent_key":"kitchen","metadata_json":{"zone":"home"}})).send().await.unwrap();
    assert_eq!(device.status(), StatusCode::CREATED);
    let device: serde_json::Value = device.json().await.unwrap();
    assert_eq!(device["device_id"], "device-1");

    let patch_url = format!("{base}/api/admin/agents/kitchen");
    let missing_match = client
        .patch(&patch_url)
        .bearer_auth(&web_token)
        .json(&serde_json::json!({"description": "primary"}))
        .send()
        .await
        .unwrap();
    assert_eq!(missing_match.status(), StatusCode::BAD_REQUEST);
    let changed = client
        .patch(&patch_url)
        .bearer_auth(&web_token)
        .header("if-match", "\"1\"")
        .json(&serde_json::json!({"description": "primary"}))
        .send()
        .await
        .unwrap();
    assert_eq!(changed.status(), StatusCode::OK);
    assert_eq!(
        changed.json::<serde_json::Value>().await.unwrap()["revision"],
        2
    );
    assert_eq!(
        client
            .patch(&patch_url)
            .bearer_auth(&web_token)
            .header("if-match", "\"1\"")
            .json(&serde_json::json!({"enabled":false}))
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::CONFLICT
    );
    assert_eq!(
        client
            .patch(format!("{base}/api/admin/devices/device-1"))
            .bearer_auth(&web_token)
            .header("if-match", "\"1\"")
            .json(&serde_json::json!({"agent_key": null}))
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        client
            .get(format!("{base}/api/admin/admin_audit_events"))
            .bearer_auth(&web_token)
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::NOT_FOUND
    );
    task.abort();
}

#[tokio::test]
async fn disabled_admin_route_is_not_mounted() {
    let (base, task) = server(false).await;
    assert_eq!(
        reqwest::get(format!("{base}/api/admin/agents"))
            .await
            .unwrap()
            .status(),
        StatusCode::NOT_FOUND
    );
    task.abort();
}

#[tokio::test]
async fn external_tool_reviews_are_admin_authenticated_and_default_empty() {
    let (base, task) = server_with_database(true, &database_url()).await;
    let client = Client::new();
    let web_token = admin_web_token(&client, &base).await;
    let response = client
        .post(format!("{base}/api/admin/agents"))
        .bearer_auth(&web_token)
        .json(&serde_json::json!({"key":"review_agent","name":"Review Agent"}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
    let url = format!("{base}/api/admin/agents/review_agent/tool-allowlist");
    assert_eq!(
        client.get(&url).send().await.unwrap().status(),
        StatusCode::UNAUTHORIZED
    );
    let response = client
        .get(&url)
        .bearer_auth(&web_token)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.json::<serde_json::Value>().await.unwrap()["items"],
        serde_json::json!([])
    );
    task.abort();
}

#[tokio::test]
async fn retired_device_tool_routes_return_not_found() {
    let (base, task) = server_with_database(true, &database_url()).await;
    let client = Client::new();
    let web_token = admin_web_token(&client, &base).await;
    for (method, path) in [
        (
            reqwest::Method::GET,
            "/api/admin/agents/agent/device-tool-allowlist",
        ),
        (
            reqwest::Method::PUT,
            "/api/admin/agents/agent/device-tool-allowlist",
        ),
        (
            reqwest::Method::POST,
            "/api/admin/agents/agent/device-tool-recovery",
        ),
    ] {
        assert_eq!(
            client
                .request(method, format!("{base}{path}"))
                .bearer_auth(&web_token)
                .send()
                .await
                .unwrap()
                .status(),
            StatusCode::NOT_FOUND,
        );
    }
    task.abort();
}
