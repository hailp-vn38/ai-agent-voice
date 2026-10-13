use super::*;
use crate::{
    app::{AppState, router_with_state},
    providers::ProviderSet,
};

#[tokio::test]
async fn plugin_upload_rejects_invalid_zip_at_admin_boundary() {
    let (db, config, runtime) = harness().await;
    let mut state = AppState::from_provider_set_with_database(
        (*config).clone(),
        Arc::new(ProviderSet::unavailable()),
        Some((*db).clone()),
    );
    state.background = Some(runtime.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!(
        "http://{}/api/admin/plugins/install",
        listener.local_addr().unwrap()
    );
    let server = tokio::spawn(async move {
        axum::serve(listener, router_with_state(state))
            .await
            .unwrap()
    });
    let client = reqwest::Client::new();
    let package = reqwest::multipart::Part::bytes(b"invalid ZIP".to_vec())
        .file_name("bad.zip")
        .mime_str("application/zip")
        .unwrap();
    let response = client
        .post(base)
        .bearer_auth("background-test-token")
        .multipart(reqwest::multipart::Form::new().part("package", package))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 400);
    assert_eq!(
        response.json::<Value>().await.unwrap()["error"]["code"],
        "invalid_plugin_package"
    );
    server.abort();
    let _ = server.await;
    runtime.shutdown().await;
}

#[tokio::test]
async fn fixture_zip_reaches_sandbox_preflight_and_fails_closed_on_unqualified_host() {
    let (db, config, runtime) = harness().await;
    let mut state = AppState::from_provider_set_with_database(
        (*config).clone(),
        Arc::new(ProviderSet::unavailable()),
        Some((*db).clone()),
    );
    state.background = Some(runtime.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!(
        "http://{}/api/admin/plugins/install",
        listener.local_addr().unwrap()
    );
    let server = tokio::spawn(async move {
        axum::serve(listener, router_with_state(state))
            .await
            .unwrap()
    });
    let bytes = include_bytes!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../plugins/fixture-research-1.0.0.zip"
    ));
    let package = reqwest::multipart::Part::bytes(bytes.to_vec()).file_name("fixture.zip");
    let response = reqwest::Client::new()
        .post(base)
        .bearer_auth("background-test-token")
        .multipart(reqwest::multipart::Form::new().part("package", package))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 503);
    assert_eq!(
        response.json::<Value>().await.unwrap()["error"]["code"],
        "sandbox_unavailable"
    );
    assert_eq!(
        runtime
            .engine
            .plugins
            .list(1, 30, None, None)
            .await
            .unwrap()
            .1,
        0
    );
    server.abort();
    let _ = server.await;
    runtime.shutdown().await;
}

#[tokio::test]
async fn admin_http_tracer_commits_queued_receipt_and_scheduler_observes_it() {
    let (db, config, runtime) = harness().await;
    let mut state = AppState::from_provider_set_with_database(
        (*config).clone(),
        Arc::new(ProviderSet::unavailable()),
        Some((*db).clone()),
    );
    state.background = Some(runtime.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}/api/admin", listener.local_addr().unwrap());
    let router = router_with_state(state);
    let server = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    let client = reqwest::Client::new();
    let url = format!("{base}/jobs");
    let response = client
        .post(&url)
        .bearer_auth("background-test-token")
        .header("Idempotency-Key", "http-once")
        .json(&json!({"goal":"HTTP foundation tracer"}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 201);
    let receipt: Value = response.json().await.unwrap();
    let id = receipt["id"].as_str().unwrap();
    let persisted = runtime.engine.get(id).await.unwrap();
    assert_eq!(persisted.status, "queued");
    wait_for(&runtime.engine, id, |job| job.scheduler_seen_at.is_some()).await;
    let replay = client
        .post(&url)
        .bearer_auth("background-test-token")
        .header("Idempotency-Key", "http-once")
        .json(&json!({"goal":"HTTP foundation tracer"}))
        .send()
        .await
        .unwrap();
    assert_eq!(replay.status(), 200);
    assert_eq!(replay.json::<Value>().await.unwrap()["id"], id);
    let forged = client
        .post(&url)
        .bearer_auth("background-test-token")
        .json(&json!({"goal":"Forge","agent_id":1}))
        .send()
        .await
        .unwrap();
    assert_eq!(forged.status(), 400);
    let response = client
        .get(&url)
        .bearer_auth("background-test-token")
        .send()
        .await
        .unwrap();
    assert_eq!(response.json::<Value>().await.unwrap()["total"], 1);
    let response = client
        .post(format!("{url}/{id}/cancel"))
        .bearer_auth("background-test-token")
        .json(&json!({}))
        .send()
        .await
        .unwrap();
    assert_eq!(
        response.json::<Value>().await.unwrap()["status"],
        "cancelled"
    );
    let response = client
        .post(format!("{url}/{id}/cancel"))
        .bearer_auth("background-test-token")
        .json(&json!({}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 409);
    server.abort();
    let _ = server.await;
    runtime.shutdown().await;
}
