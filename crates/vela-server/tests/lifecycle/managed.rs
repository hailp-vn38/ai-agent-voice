use std::sync::Arc;
use std::time::Duration;

use super::harness::*;

#[tokio::test]
async fn application_shutdown_uses_its_existing_deadline_for_managed_resources() {
    use std::collections::HashMap;
    use vela_server::{
        database::{DesiredProvider, secrets::EnvSecretResolver},
        services::provider_runtime::{FactoryMaterializer, ProviderRuntimeManager, RuntimeLimits},
        workers::WorkerSupervisor,
    };
    let mut app_config = config("sqlite::memory:".into());
    app_config.shutdown.grace_ms = 30;
    let state =
        AppState::from_provider_set(app_config.clone(), Arc::new(ProviderSet::unavailable()));
    let builder = FactoryMaterializer::new(
        Arc::new(app_config),
        Arc::new(EnvSecretResolver),
        HashMap::from([("openai".into(), 4096)]),
        Arc::new(WorkerSupervisor::start_many(vec![], vec![])),
    )
    .unwrap();
    let manager = ProviderRuntimeManager::new(
        RuntimeLimits {
            max_parallel_loads: 1,
            max_pending_loads: 1,
            max_waiters: 2,
            max_resident_bytes: 8192,
            max_resources: 2,
            max_version_entries: 4,
            admission_timeout_ms: 1000,
            failure_cooldown_ms: 10,
            idle_ttl_ms: 10,
        },
        Arc::new(builder),
        state.admission_gate().clone(),
    )
    .unwrap();
    let state = state.with_runtime_manager(manager.clone());
    let lease = manager
        .acquire(DesiredProvider {
            id: 1,
            key: "remote".into(),
            kind: "llm".into(),
            adapter: "openai".into(),
            revision: 1,
            config_json: r#"{"base_url":"https://example.test/v1","model":"fixture"}"#.into(),
            secret_ref: None,
        })
        .await
        .unwrap();
    let started = tokio::time::Instant::now();
    let report = state.lifecycle.shutdown().await;
    assert_eq!(report.provider_resources_drained, Some(false));
    assert!(started.elapsed() < Duration::from_millis(300));
    assert_eq!(manager.accounting().reserved_bytes, 4096);
    drop(lease);
    assert!(
        manager
            .shutdown_until(tokio::time::Instant::now() + Duration::from_secs(1))
            .await
    );
}

#[tokio::test]
async fn shutdown_rejects_admin_direct_speech_on_an_existing_session() {
    let mut app_config = config(database_url());
    app_config.api.enabled = true;
    app_config.api.admin_token = "direct-admin".into();
    app_config.api.jwt_secret = vela_server::config::SecretString::new("lifecycle-signing-secret-0123456789");
    let voice = start(app_config, Duration::from_secs(15)).await;
    seed(&voice.database().await).await;
    let (mut socket, _) = connect_async(request(&voice.base, "device")).await.unwrap();
    socket.send(Message::Text(serde_json::json!({"type":"hello","version":1,"transport":"websocket","features":{"direct_tts":true}}).to_string().into())).await.unwrap();
    let _ = timeout(Duration::from_secs(2), socket.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let admin = crate::admin_web::admin_web_token(&reqwest::Client::new(), &voice.base).await;
    assert!(voice.lifecycle.begin_shutdown());
    let response = reqwest::Client::new()
        .post(format!("{}/api/admin/devices/device/speak", voice.base))
        .bearer_auth(&admin)
        .json(&serde_json::json!({"request_id":"shutdown-1","text":"Do not start new speech."}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(
        response.json::<serde_json::Value>().await.unwrap()["error"]["code"],
        "shutting_down"
    );
    voice.task.abort();
}

#[tokio::test]
async fn admin_sse_streams_do_not_hold_graceful_shutdown_past_controlled_close_settle() {
    use vela_server::lifecycle::CONTROLLED_CLOSE_SETTLE;

    let temp_root =
        std::env::temp_dir().join(format!("vela-sse-shutdown-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&temp_root).unwrap();

    let mut app_config = config(database_url());
    app_config.api.enabled = true;
    app_config.api.admin_token = "sse-shutdown-admin".into();
    app_config.api.jwt_secret = vela_server::config::SecretString::new("lifecycle-sse-signing-secret-0123456789");
    app_config.orchestrator.enabled = true;
    app_config.orchestrator.store_dir = temp_root.join("orch-store");
    app_config.orchestrator.tasks_store_dir = temp_root.join("tasks-store");
    app_config.orchestrator.reports_dir = temp_root.join("reports");
    app_config.orchestrator.work_dir = temp_root.join("work");
    app_config.orchestrator.prompts_dir = temp_root.join("prompts");
    app_config.orchestrator.skills_dir = Some(temp_root.join("skills"));
    app_config.wiki.vault_dir = temp_root.join("wiki");

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let database = Database::connect(&app_config.database).await.unwrap();
    crate::admin_web::seed_default_admin(database.pool()).await;
    let lifecycle = RuntimeLifecycle::new(Duration::from_secs(15));
    let mut state = AppState::from_provider_set_with_database_and_shutdown(
        app_config.clone(),
        providers(),
        Some(database),
        Arc::clone(&lifecycle),
    );
    let runtime = vela_server::services::task_flow::init_from_app_config(&app_config)
        .await
        .unwrap();
    state.wiki_engine = runtime.wiki_engine;
    state.orchestrator = runtime.orchestrator;
    state.task_manager = runtime.task_manager;
    state.autoflow_notify = Some(runtime.autoflow_notify);

    let listening = lifecycle.listening().clone();
    let mut server = tokio::spawn(async move {
        axum::serve(listener, router_with_state(state))
            .with_graceful_shutdown(listening.cancelled_owned())
            .await
    });

    let base = format!("http://{address}");
    let client = reqwest::Client::new();
    let admin = crate::admin_web::admin_web_token(&client, &base).await;
    let mut streams = Vec::new();
    for path in [
        "/api/admin/system/logs/stream",
        "/api/admin/notifications/stream",
        "/api/admin/jobs/stream",
    ] {
        let resp = client
            .get(format!("{base}{path}"))
            .bearer_auth(&admin)
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK, "SSE stream {path} opened");
        let mut body = resp.bytes_stream();
        let ready = timeout(Duration::from_secs(2), body.next())
            .await
            .expect("ready chunk timeout")
            .expect("ready chunk present")
            .expect("ready chunk bytes");
        assert!(
            String::from_utf8_lossy(&ready).contains("event: ready"),
            "SSE stream {path} emitted ready frame"
        );
        streams.push((path, body));
    }

    let report = lifecycle.shutdown().await;
    assert_eq!(report.outcome, DrainOutcome::Drained);
    assert_eq!(report.controlled_closes, 0);

    let settle = timeout(CONTROLLED_CLOSE_SETTLE, &mut server).await;
    if settle.is_err() {
        server.abort();
        let _ = server.await;
    }
    settle
        .expect("axum::serve with_graceful_shutdown timed out after CONTROLLED_CLOSE_SETTLE while Admin SSE streams were open")
        .expect("server task joined cleanly")
        .expect("axum::serve exited without error");

    for (path, mut body) in streams {
        while let Some(chunk) = timeout(Duration::from_secs(1), body.next())
            .await
            .unwrap_or_else(|_| panic!("SSE stream {path} did not close after shutdown"))
        {
            assert!(chunk.is_ok(), "SSE stream {path} closed cleanly: {chunk:?}");
        }
    }
}
