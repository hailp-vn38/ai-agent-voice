use super::harness::*;

#[tokio::test]
async fn memory_fact_lifecycle_reports_real_state_on_a_device_that_is_off() {
    let server = start().await;
    server.agent("home").await;
    let device = server.device("kitchen", "home").await;
    assert_eq!(device["memory_mode"], "off");

    let created = server
        .post(
            "/memory/facts?device=kitchen",
            json!({"title": "Lịch họp", "hook": "họp thứ hai", "body": "Đặt lịch họp thứ hai", "kind": "business"}),
        )
        .send()
        .await
        .unwrap();
    assert_eq!(created.status(), StatusCode::CREATED);
    let receipt: Value = created.json().await.unwrap();
    assert_eq!(receipt["action"], "remember");
    assert_eq!(receipt["revision"], 1);
    assert_eq!(receipt["cleanup_pending"], false);
    let id = receipt["fact_id"].as_str().unwrap().to_string();
    assert!(id.starts_with("fact_"));

    // The same body again returns the fact it already has.
    let again = server
        .remember("kitchen", "Lịch khác", "Đặt lịch họp thứ hai")
        .await;
    assert_eq!(again, id);

    let items = server.facts("kitchen").await;
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["id"], id.as_str());
    assert_eq!(items[0]["revision"], 1);
    assert_eq!(items[0]["status"], "confirmed");
    assert_eq!(items[0]["kind"], "business");
    assert_eq!(items[0]["pinned"], false);
    assert_eq!(items[0]["hook"], "họp thứ hai");

    // Search folds Vietnamese, as ASR text arrives without diacritics.
    let found = ok(server.get("/memory/facts?device=kitchen&q=dat%20lich")).await;
    assert_eq!(found["items"][0]["id"], id.as_str());
    let nothing = ok(server.get("/memory/facts?device=kitchen&q=zzzqqq")).await;
    assert_eq!(nothing["items"], json!([]));

    let fact_path = format!("/memory/facts/{id}?device=kitchen");
    let read = ok(server.get(&fact_path)).await;
    assert_eq!(read["body"], "Đặt lịch họp thứ hai");

    // `expected_revision` is mandatory, and an old one changes nothing.
    assert_eq!(
        server
            .put(&fact_path, json!({"body": "x"}))
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::BAD_REQUEST
    );
    expect_error(
        server
            .put(&fact_path, json!({"body": "x", "expected_revision": 9}))
            .send()
            .await
            .unwrap(),
        StatusCode::CONFLICT,
        "memory_revision_conflict",
    )
    .await;
    assert_eq!(
        ok(server.get(&fact_path)).await["body"],
        "Đặt lịch họp thứ hai"
    );
    let corrected = ok(server.put(
        &fact_path,
        json!({"title": "Lịch mới", "hook": "thứ ba", "body": "Đặt lịch họp thứ ba", "expected_revision": 1}),
    ))
    .await;
    assert_eq!(corrected["action"], "correct");
    assert_eq!(corrected["revision"], 2);
    let read = ok(server.get(&fact_path)).await;
    assert_eq!(read["title"], "Lịch mới");
    assert_eq!(read["hook"], "thứ ba");

    // Pinning keeps the revision.
    let pin_path = format!("/memory/facts/{id}/pin?device=kitchen");
    ok(server.post(&pin_path, json!({"pinned": true}))).await;
    let read = ok(server.get(&fact_path)).await;
    assert_eq!(read["pinned"], true);
    assert_eq!(read["revision"], 2);

    let summary = ok(server.get("/memory/summary?device=kitchen")).await;
    assert!(summary["summary"].as_str().unwrap().contains("Lịch mới"));
    assert_eq!(summary["hidden"], 0);

    let index = ok(server.get("/memory/index?device=kitchen")).await;
    assert_eq!(index["facts"], 1);
    assert_eq!(index["quarantined"], 0);
    assert_eq!(index["memory_mode"], "off");
    assert_eq!(
        ok(server.post("/memory/index/rebuild?device=kitchen", json!({}))).await["facts"],
        1
    );

    // Forget needs the current revision and leaves a tombstone.
    let forget_path = format!("/memory/facts/{id}/forget?device=kitchen");
    assert_eq!(
        server
            .post(&forget_path, json!({}))
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::BAD_REQUEST
    );
    expect_error(
        server
            .post(&forget_path, json!({"expected_revision": 1}))
            .send()
            .await
            .unwrap(),
        StatusCode::CONFLICT,
        "memory_revision_conflict",
    )
    .await;
    let forgotten = ok(server.post(&forget_path, json!({"expected_revision": 2}))).await;
    assert_eq!(forgotten["action"], "forget");
    assert!(server.facts("kitchen").await.is_empty());
    expect_error(
        server.get(&fact_path).send().await.unwrap(),
        StatusCode::NOT_FOUND,
        "memory_not_found",
    )
    .await;
    let summary = ok(server.get("/memory/summary?device=kitchen")).await;
    assert_eq!(summary["summary"], "");
    server.task.abort();
}

#[tokio::test]
async fn memory_discard_leaves_no_tombstone_so_the_same_content_saves_again() {
    let server = start().await;
    server.agent("home").await;
    server.device("kitchen", "home").await;
    let id = server
        .remember("kitchen", "Ghi nhầm", "Nội dung nhầm")
        .await;
    let discarded = ok(server.post(
        &format!("/memory/facts/{id}/discard?device=kitchen"),
        json!({"expected_revision": 1}),
    ))
    .await;
    assert_eq!(discarded["action"], "discard");
    assert!(server.facts("kitchen").await.is_empty());
    for dir in server.device_dirs() {
        let path = server
            .root
            .join("vault/devices")
            .join(dir)
            .join("tombstones.json");
        assert!(!path.exists(), "Discard wrote a Forget Tombstone");
    }
    let again = server
        .remember("kitchen", "Ghi nhầm", "Nội dung nhầm")
        .await;
    assert_ne!(again, id);
    server.task.abort();
}

#[tokio::test]
async fn memory_profile_needs_no_device_and_checks_its_revision() {
    let server = start().await;
    let initial = ok(server.get("/memory/profile")).await;
    assert_eq!(initial["body"], "");
    let revision = initial["revision"].as_u64().unwrap();

    let saved = ok(server.post(
        "/memory/profile",
        json!({"body": "Công ty ở Đà Nẵng", "expected_revision": revision}),
    ))
    .await;
    assert_eq!(saved["action"], "set_profile");
    let read = ok(server.get("/memory/profile")).await;
    assert_eq!(read["body"], "Công ty ở Đà Nẵng");
    assert_eq!(read["revision"], revision + 1);

    expect_error(
        server
            .post(
                "/memory/profile",
                json!({"body": "cũ", "expected_revision": revision}),
            )
            .send()
            .await
            .unwrap(),
        StatusCode::CONFLICT,
        "memory_revision_conflict",
    )
    .await;

    // A Device's summary carries the shared profile.
    server.agent("home").await;
    server.device("kitchen", "home").await;
    let summary = ok(server.get("/memory/summary?device=kitchen")).await;
    assert!(summary["summary"].as_str().unwrap().contains("Đà Nẵng"));
    server.task.abort();
}
