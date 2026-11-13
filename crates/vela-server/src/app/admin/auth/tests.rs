use super::*;
use crate::{app::router_with_state, providers::LoadedProviders};
use futures_util::StreamExt;
use std::time::Duration;

async fn next_snapshot<S, B, E>(chunks: &mut S) -> serde_json::Value
where
    S: futures_util::Stream<Item = Result<B, E>> + Unpin,
    B: AsRef<[u8]>,
    E: std::fmt::Debug,
{
    loop {
        let chunk = tokio::time::timeout(Duration::from_secs(3), chunks.next())
            .await
            .expect("System Live should send a snapshot promptly")
            .expect("stream should remain open")
            .expect("SSE body should be readable");
        let text = String::from_utf8_lossy(chunk.as_ref());
        if let Some(data) = text.lines().find_map(|line| line.strip_prefix("data: ")) {
            return serde_json::from_str(data).unwrap();
        }
    }
}

async fn settle_system_live<S, B, E>(chunks: &mut S)
where
    S: futures_util::Stream<Item = Result<B, E>> + Unpin,
    B: AsRef<[u8]>,
    E: std::fmt::Debug,
{
    let mut first = true;
    loop {
        let snapshot = next_snapshot(chunks).await;
        if first {
            assert_eq!(snapshot["v"], 1);
            first = false;
        }
        if snapshot["status"]["checked_at"].is_string() {
            break;
        }
    }
}

async fn assert_system_live_quiet<S, B, E>(chunks: &mut S)
where
    S: futures_util::Stream<Item = Result<B, E>> + Unpin,
    B: AsRef<[u8]>,
    E: std::fmt::Debug,
{
    assert!(
        tokio::time::timeout(Duration::from_millis(200), chunks.next())
            .await
            .is_err(),
        "System Live must not repeat an unchanged snapshot"
    );
}

fn test_config() -> AppConfig {
    toml::from_str(
        r#"
        [server]
        bind = "127.0.0.1:0"
        public_ws_url = "ws://127.0.0.1:0/ws"
        [provider_defaults]
        vad = "test"
        asr = "test"
        llm = "test"
        tts = "test"
        [api]
        enabled = true
        jwt_secret = "system-live-session-test-jwt-signing-secret"
        "#,
    )
    .unwrap()
}

fn test_token(config: &AppConfig, revision: i64, expiry: i64) -> String {
    token::sign(
        &hmac::Key::new(
            hmac::HMAC_SHA256,
            config.api.jwt_secret.expose().trim().as_bytes(),
        ),
        &token::Claims {
            iat: unix_now(),
            exp: expiry,
            rev: revision,
        },
    )
}

async fn start_admin_server(state: &crate::app::AppState) -> (String, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let server_state = state.clone();
    let server = tokio::spawn(async move {
        axum::serve(listener, router_with_state(server_state))
            .await
            .unwrap();
    });
    (base, server)
}

async fn open_system_live(base: &str, token: Option<&str>) -> reqwest::Response {
    let request = reqwest::Client::new()
        .get(format!("{base}/api/admin/system/live"))
        .header("accept", "text/event-stream");
    match token {
        Some(token) => request.bearer_auth(token),
        None => request,
    }
    .send()
    .await
    .unwrap()
}

async fn open_admin_sse(base: &str, path: &str, token: &str) -> reqwest::Response {
    reqwest::Client::new()
        .get(format!("{base}{path}"))
        .header("accept", "text/event-stream")
        .bearer_auth(token)
        .send()
        .await
        .unwrap()
}

#[test]
fn stream_access_tracks_the_same_password_login_and_token_rules_as_admin_requests() {
    let config = test_config();
    let auth = AdminAuthState::from_config(&config);
    let expiry = unix_now() + 600;
    let token = test_token(&config, 1, expiry);

    assert!(!auth.stream_access_allowed(None));
    assert!(auth.stream_access_allowed(Some(&token)));
    assert_eq!(
        auth.token_expiry(&token)
            .unwrap()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64,
        expiry
    );

    auth.publish(PasswordLoginStatus {
        password_login_enabled: false,
        must_change_password: false,
        revision: 1,
    });
    assert!(auth.stream_access_allowed(None));
    assert!(auth.stream_access_allowed(Some("invalid-token")));

    auth.publish(PasswordLoginStatus {
        password_login_enabled: true,
        must_change_password: false,
        revision: 1,
    });
    assert!(!auth.stream_access_allowed(None));
    assert!(auth.stream_access_allowed(Some(&token)));

    auth.publish(PasswordLoginStatus {
        password_login_enabled: true,
        must_change_password: true,
        revision: 1,
    });
    assert!(!auth.stream_access_allowed(Some(&token)));

    // `authenticate` answers 403 to an accepted token that must change its password, even with
    // Password Login off; the stream rule matches it.
    auth.publish(PasswordLoginStatus {
        password_login_enabled: false,
        must_change_password: true,
        revision: 1,
    });
    assert!(!auth.stream_access_allowed(Some(&token)));
    assert!(auth.stream_access_allowed(None));
}

#[tokio::test]
async fn system_live_stream_ends_after_logout_everywhere_revision_changes() {
    let config = test_config();
    let state = crate::app::AppState::new(config.clone(), LoadedProviders::default());
    let token = test_token(&config, 1, unix_now() + 600);
    let (base, server) = start_admin_server(&state).await;
    let response = open_system_live(&base, Some(&token)).await;
    assert_eq!(response.status(), reqwest::StatusCode::OK);
    let mut chunks = response.bytes_stream();
    settle_system_live(&mut chunks).await;
    assert_system_live_quiet(&mut chunks).await;
    assert!(
        tokio::time::timeout(Duration::from_millis(200), chunks.next())
            .await
            .is_err(),
        "stream must not repeat its initial snapshot"
    );

    let next_chunk = tokio::spawn(async move { chunks.next().await });
    tokio::task::yield_now().await;
    // This is the in-memory state published after the logout-everywhere transaction commits.
    // The guard must observe the revision without reading the database.
    state.admin_auth.publish(PasswordLoginStatus {
        password_login_enabled: true,
        must_change_password: false,
        revision: 2,
    });
    let end = tokio::time::timeout(Duration::from_secs(6), next_chunk)
        .await
        .expect("revoked stream should end within one auth check interval")
        .unwrap();
    assert!(end.is_none(), "unexpected stream result: {end:?}");
    server.abort();
}

#[tokio::test]
async fn system_live_stream_ends_when_password_login_is_enabled_after_anonymous_open() {
    let state = crate::app::AppState::new(test_config(), LoadedProviders::default());
    state.admin_auth.publish(PasswordLoginStatus {
        password_login_enabled: false,
        must_change_password: false,
        revision: 1,
    });
    let (base, server) = start_admin_server(&state).await;
    let response = open_system_live(&base, None).await;
    assert_eq!(response.status(), reqwest::StatusCode::OK);
    let mut chunks = response.bytes_stream();
    settle_system_live(&mut chunks).await;
    assert_system_live_quiet(&mut chunks).await;
    assert!(
        tokio::time::timeout(Duration::from_millis(200), chunks.next())
            .await
            .is_err(),
        "stream must not repeat its initial snapshot"
    );

    let next_chunk = tokio::spawn(async move { chunks.next().await });
    tokio::task::yield_now().await;
    state.admin_auth.publish(PasswordLoginStatus {
        password_login_enabled: true,
        must_change_password: false,
        revision: 1,
    });
    let end = tokio::time::timeout(Duration::from_secs(6), next_chunk)
        .await
        .expect("anonymous stream should close on the next auth check")
        .unwrap();
    assert!(end.is_none(), "unexpected stream result: {end:?}");
    server.abort();
}

#[tokio::test]
async fn system_live_stream_with_token_survives_disabling_password_login() {
    let config = test_config();
    let state = crate::app::AppState::new(config.clone(), LoadedProviders::default());
    let token = test_token(&config, 1, unix_now() + 600);
    let (base, server) = start_admin_server(&state).await;
    let response = open_system_live(&base, Some(&token)).await;
    assert_eq!(response.status(), reqwest::StatusCode::OK);
    let mut chunks = response.bytes_stream();
    settle_system_live(&mut chunks).await;
    assert_system_live_quiet(&mut chunks).await;
    assert!(
        tokio::time::timeout(Duration::from_millis(200), chunks.next())
            .await
            .is_err(),
        "stream must not repeat its initial snapshot"
    );

    state.admin_auth.publish(PasswordLoginStatus {
        password_login_enabled: false,
        must_change_password: false,
        revision: 1,
    });
    let remains_open = tokio::time::timeout(Duration::from_millis(500), chunks.next()).await;
    assert!(
        remains_open.is_err(),
        "unexpected stream result: {remains_open:?}"
    );
    state.lifecycle.stopping().cancel();
    assert!(
        tokio::time::timeout(Duration::from_secs(2), chunks.next())
            .await
            .unwrap()
            .is_none()
    );
    server.abort();
}

#[tokio::test]
async fn system_live_stream_ends_at_signed_token_expiry() {
    let config = test_config();
    let state = crate::app::AppState::new(config.clone(), LoadedProviders::default());
    let token = test_token(&config, 1, unix_now() + 2);
    let (base, server) = start_admin_server(&state).await;
    let response = open_system_live(&base, Some(&token)).await;
    assert_eq!(response.status(), reqwest::StatusCode::OK);
    let mut chunks = response.bytes_stream();
    settle_system_live(&mut chunks).await;
    assert_system_live_quiet(&mut chunks).await;
    let next_chunk = tokio::spawn(async move { chunks.next().await });
    let end = tokio::time::timeout(Duration::from_secs(4), next_chunk)
        .await
        .expect("SSE must stop at exp without waiting for the 5 second poll")
        .unwrap();
    assert!(end.is_none(), "unexpected stream result: {end:?}");
    server.abort();
}

#[tokio::test]
async fn other_admin_sse_routes_end_when_the_token_revision_changes() {
    use crate::{
        config::DatabaseConfig, database::Database, services::notification_hub::NotificationHub,
    };
    use async_trait::async_trait;
    use orchestrator_agent_rs::{
        AgentDefinition, AgentMode, CustomModelExecutor, GatewayError, LlmBackend, LlmConfig,
        LlmProviderConfig, ModelRequest, ModelResponse, Orchestrator, OrchestratorConfig,
        ProviderAuth, RequestLimits, StreamSink,
    };
    use orchestrator_flow_rs::{TaskConfig, TaskManager};
    use tokio_util::sync::CancellationToken;

    #[derive(Debug)]
    struct NoopExecutor;

    #[async_trait]
    impl CustomModelExecutor for NoopExecutor {
        fn routing_fingerprint(&self) -> String {
            "admin-stream-guard-test".into()
        }

        async fn complete(
            &self,
            _request: &ModelRequest,
            _cancel: &CancellationToken,
            _sink: StreamSink<'_>,
        ) -> Result<ModelResponse, GatewayError> {
            Ok(ModelResponse {
                text: String::new(),
                finish_reason: "stop".into(),
                tool_calls: Vec::new(),
                usage: None,
            })
        }
    }

    let temp = std::env::temp_dir().join(format!("admin-stream-guard-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&temp).unwrap();
    let config = test_config();
    let mut state = crate::app::AppState::new(config.clone(), LoadedProviders::default());
    let database = Database::connect(&DatabaseConfig {
        url: "sqlite::memory:".into(),
        max_connections: 1,
        ..Default::default()
    })
    .await
    .unwrap();
    state.database = Some(std::sync::Arc::new(database.clone()));
    state.notification_hub = Some(NotificationHub::new(
        database,
        state.secret_resolver.clone(),
        vec![],
    ));
    let prompt = temp.join("root.md");
    std::fs::write(&prompt, "test root").unwrap();
    let orchestrator = std::sync::Arc::new(
        Orchestrator::builder(OrchestratorConfig::new(
            LlmConfig {
                providers: vec![LlmProviderConfig {
                    id: "test".into(),
                    backend: LlmBackend::OpenAi,
                    endpoint: None,
                    auth: ProviderAuth::CustomExecutor(std::sync::Arc::new(NoopExecutor)),
                    limits: RequestLimits::uniform(Duration::from_secs(30)),
                }],
            },
            vec![
                AgentDefinition::new("stream_guard_root", "test", "test-model")
                    .with_mode(AgentMode::Primary)
                    .with_prompt_path(&prompt),
            ],
            temp.join("orchestrator"),
        ))
        .build()
        .await
        .unwrap(),
    );
    state.task_manager = Some(
        TaskManager::prepare(
            TaskConfig::new(temp.join("tasks")).with_default_target("default", "stream_guard_root"),
        )
        .await
        .unwrap()
        .register_flow("default", orchestrator)
        .start()
        .await
        .unwrap(),
    );
    state.wiki_engine = Some(std::sync::Arc::new(
        llm_wiki::WikiEngine::open(temp.join("vault")).unwrap(),
    ));

    let (base, server) = start_admin_server(&state).await;
    let paths = [
        "/api/admin/jobs/stream",
        "/api/admin/notifications/stream",
        "/api/admin/system/logs/stream",
        "/api/admin/knowledge/events",
    ];
    for (revision, path) in (1..).zip(paths) {
        let token = test_token(&config, revision, unix_now() + 600);
        let response = open_admin_sse(&base, path, &token).await;
        assert_eq!(response.status(), reqwest::StatusCode::OK, "{path}");
        let mut chunks = response.bytes_stream();
        let first = tokio::time::timeout(Duration::from_secs(3), chunks.next())
            .await
            .expect("SSE stream should emit its initial event")
            .expect("SSE response should remain open")
            .expect("SSE response body should be readable");
        assert!(String::from_utf8_lossy(&first).contains("event:"), "{path}");

        let next_chunk = tokio::spawn(async move { chunks.next().await });
        tokio::task::yield_now().await;
        let next_revision = revision + 1;
        state.admin_auth.publish(PasswordLoginStatus {
            password_login_enabled: true,
            must_change_password: false,
            revision: next_revision,
        });
        let end = tokio::time::timeout(Duration::from_secs(6), next_chunk)
            .await
            .unwrap_or_else(|_| panic!("{path} should end within one auth check interval"))
            .unwrap();
        assert!(
            end.is_none(),
            "{path} unexpectedly emitted data after revocation"
        );
    }

    state.lifecycle.stopping().cancel();
    server.abort();
    if let Some(task_manager) = state.task_manager.as_ref() {
        task_manager.shutdown().await.unwrap();
    }
    let _ = std::fs::remove_dir_all(temp);
}

#[tokio::test(start_paused = true)]
async fn admin_stream_guard_ends_at_the_token_expiry_deadline() {
    let config = test_config();
    let state = crate::app::AppState::new(config.clone(), LoadedProviders::default());
    // Signed token verification reads the wall clock, which paused tokio time does not move, so
    // revoking at the deadline stands in for the token turning invalid exactly at `exp`.
    let token = test_token(&config, 1, unix_now() + 600);
    let expires_at = tokio::time::Instant::now() + Duration::from_secs(2);
    let guard = tokio::spawn(
        crate::app::admin::middleware::wait_for_admin_stream_revocation(
            state.clone(),
            Some(token),
            Some(expires_at),
            Duration::from_secs(5),
        ),
    );
    tokio::time::advance(Duration::from_secs(1)).await;
    assert!(!guard.is_finished());
    state.admin_auth.publish(PasswordLoginStatus {
        password_login_enabled: true,
        must_change_password: false,
        revision: 2,
    });
    tokio::time::advance(Duration::from_secs(1)).await;
    tokio::time::timeout(Duration::from_millis(1), guard)
        .await
        .expect("guard must re-check at exp instead of waiting for the next 5 second poll")
        .unwrap();
}

#[tokio::test(start_paused = true)]
async fn admin_stream_guard_keeps_an_open_stream_past_exp_while_password_login_is_off() {
    let state = crate::app::AppState::new(test_config(), LoadedProviders::default());
    state.admin_auth.publish(PasswordLoginStatus {
        password_login_enabled: false,
        must_change_password: false,
        revision: 1,
    });
    let expires_at = tokio::time::Instant::now() + Duration::from_secs(2);
    let guard = tokio::spawn(
        crate::app::admin::middleware::wait_for_admin_stream_revocation(
            state,
            Some("expired-or-invalid".into()),
            Some(expires_at),
            Duration::from_secs(5),
        ),
    );
    tokio::time::advance(Duration::from_secs(12)).await;
    assert!(!guard.is_finished());
    guard.abort();
}
