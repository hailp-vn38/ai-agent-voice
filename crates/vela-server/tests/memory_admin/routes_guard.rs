use super::harness::*;

#[tokio::test]
async fn memory_routes_need_a_device_that_exists_and_create_no_vault_otherwise() {
    let server = start().await;
    for (method, path, body) in device_scoped_routes() {
        let url = format!("{}{path}", server.base);
        let send = |url: String| {
            let request = server
                .client
                .request(method.clone(), url)
                .bearer_auth(&server.token);
            if body.is_null() {
                request.send()
            } else {
                request.json(&body).send()
            }
        };
        let missing = send(url.clone()).await.unwrap();
        assert_eq!(missing.status(), StatusCode::BAD_REQUEST, "{method} {path}");
        let empty = send(format!("{url}?device=")).await.unwrap();
        assert_eq!(empty.status(), StatusCode::BAD_REQUEST, "{method} {path}");
        expect_error(
            send(format!("{url}?device=ghost")).await.unwrap(),
            StatusCode::NOT_FOUND,
            "device_not_found",
        )
        .await;
    }
    assert!(server.device_dirs().is_empty());
    server.task.abort();
}

#[tokio::test]
async fn memory_disabled_answers_503_touches_no_disk_and_leaves_the_server_running() {
    let root = temp_root();
    let server = start_at(&root, "").await;
    server.agent("home").await;
    server.device("kitchen", "home").await;
    expect_error(
        server
            .get("/memory/facts?device=kitchen")
            .send()
            .await
            .unwrap(),
        StatusCode::SERVICE_UNAVAILABLE,
        "memory_disabled",
    )
    .await;
    expect_error(
        server.get("/memory/profile").send().await.unwrap(),
        StatusCode::SERVICE_UNAVAILABLE,
        "memory_disabled",
    )
    .await;
    assert!(!root.join("vault").exists());
    server.task.abort();
}

#[tokio::test]
async fn memory_that_cannot_open_keeps_the_server_up_and_reports_unavailable() {
    let root = temp_root();
    // A file where the vault directory should be.
    fs::write(root.join("vault"), b"not a directory").unwrap();
    let extra = memory_on(&root);
    let server = start_at(&root, &extra).await;
    server.agent("home").await;
    server.device("kitchen", "home").await;
    expect_error(
        server
            .get("/memory/facts?device=kitchen")
            .send()
            .await
            .unwrap(),
        StatusCode::SERVICE_UNAVAILABLE,
        "memory_unavailable",
    )
    .await;
    server.task.abort();
}

#[tokio::test]
async fn memory_rejects_invalid_fact_input_and_maps_core_errors() {
    let server = start().await;
    server.agent("home").await;
    server.device("kitchen", "home").await;
    let create = "/memory/facts?device=kitchen";
    for body in [
        json!({"title": " ", "hook": "h", "body": "b", "kind": "user"}),
        json!({"title": "t", "hook": "h", "body": " ", "kind": "user"}),
        json!({"title": "t", "hook": "h", "body": "b", "kind": "secret"}),
        json!({"title": "t", "hook": "h", "body": "b"}),
    ] {
        assert_eq!(
            server.post(create, body).send().await.unwrap().status(),
            StatusCode::BAD_REQUEST
        );
    }
    expect_error(
        server
            .post(
                create,
                json!({"title": "t", "hook": "h", "kind": "user",
                       "body": "postgres://admin:hunter2@db.example.com/prod"}),
            )
            .send()
            .await
            .unwrap(),
        StatusCode::BAD_REQUEST,
        "memory_rejected_sensitive",
    )
    .await;
    expect_error(
        server
            .post(
                create,
                json!({"title": "t", "hook": "h", "kind": "user", "body": "x".repeat(2_001)}),
            )
            .send()
            .await
            .unwrap(),
        StatusCode::BAD_REQUEST,
        "memory_too_large",
    )
    .await;
    server.task.abort();
}

#[tokio::test]
async fn memory_routes_never_reach_a_wiki_page_and_wiki_search_never_sees_a_fact() {
    let root = temp_root();
    let extra = format!(
        "{}[wiki]\nenabled = true\nvault_dir = \"{}\"\n",
        memory_on(&root),
        root.join("wiki").display()
    );
    let server = start_at(&root, &extra).await;
    server.agent("home").await;
    server.device("kitchen", "home").await;

    let page = ok(server.post(
        "/knowledge/pages",
        json!({"title": "Wiki tri thức", "body": "Trang wiki còn nguyên", "source_ref": "test"}),
    ))
    .await;
    let page_id = page["id"].as_str().unwrap().to_string();
    let encoded = page_id.replace('/', "%2F");

    for suffix in ["forget", "discard"] {
        expect_error(
            server
                .post(
                    &format!("/memory/facts/{encoded}/{suffix}?device=kitchen"),
                    json!({"expected_revision": 1}),
                )
                .send()
                .await
                .unwrap(),
            StatusCode::NOT_FOUND,
            "memory_not_found",
        )
        .await;
    }
    expect_error(
        server
            .get(&format!("/memory/facts/{encoded}?device=kitchen"))
            .send()
            .await
            .unwrap(),
        StatusCode::NOT_FOUND,
        "memory_not_found",
    )
    .await;
    let still_there = ok(server.get(&format!("/knowledge/pages/{encoded}"))).await;
    assert!(still_there.to_string().contains("Trang wiki"));

    server
        .remember("kitchen", "Mật danh", "zephyrquokka là mật danh")
        .await;
    let search = ok(server.get("/knowledge/pages?q=zephyrquokka")).await;
    assert_eq!(search["items"], json!([]));
    // The Memory screen lists only Memory, whatever the Wiki holds.
    assert_eq!(server.facts("kitchen").await.len(), 1);
    server.task.abort();
}
