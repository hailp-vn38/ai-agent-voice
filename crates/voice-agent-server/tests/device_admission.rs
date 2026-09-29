use std::sync::Arc;

use axum::Router;
use futures_util::{SinkExt, StreamExt};
use sqlx::SqlitePool;
use tokio::{
    net::TcpListener,
    task::JoinHandle,
    time::{Duration, timeout},
};
use tokio_tungstenite::{
    connect_async,
    tungstenite::{Message, client::IntoClientRequest, http::StatusCode},
};
use url::Url;
use voice_agent_server::{
    app::bootstrap_with_providers,
    config::{
        AppConfig, AudioConfig, AuthConfig, BargeInConfig, DatabaseConfig, DeploymentConfig,
        LimitsConfig, LlmConfig, ProvidersConfig, RuntimeConfig, ServerConfig, SpeechOutputConfig,
        TtsConfig, WebsocketConfig, WorkersConfig,
    },
    providers::ProviderSet,
};

fn database_url() -> String {
    format!(
        "sqlite://{}",
        std::env::temp_dir()
            .join(format!("voice-agent-admission-{}.db", uuid::Uuid::new_v4()))
            .display()
    )
}

fn config(
    address: std::net::SocketAddr,
    database_url: String,
    admission_enabled: bool,
) -> AppConfig {
    AppConfig {
        server: ServerConfig {
            bind: address,
            public_ws_url: Url::parse(&format!("ws://{address}/voice/v1/")).unwrap(),
            hello_timeout_ms: 500,
        },
        auth: AuthConfig::default(),
        audio: AudioConfig::default(),
        websocket: WebsocketConfig::default(),
        limits: LimitsConfig::default(),
        provider_defaults: voice_agent_server::config::ProviderDefaultsConfig {
            vad: "test".into(),
            asr: "test".into(),
            llm: "test".into(),
            tts: "test".into(),
            vision: None,
        },
        providers: ProvidersConfig::default(),
        workers: WorkersConfig::default(),
        deployment: DeploymentConfig::default(),
        runtime: RuntimeConfig::default(),
        llm: LlmConfig::default(),
        tts: TtsConfig::default(),
        speech_output: SpeechOutputConfig::default(),
        barge_in: BargeInConfig::default(),
        mcp: voice_agent_server::config::McpConfig::default(),
        vision: voice_agent_server::config::VisionConfig::default(),
        database: DatabaseConfig {
            enabled: true,
            url: database_url,
            devices: voice_agent_server::config::DatabaseDevicesConfig {
                admission_enabled,
                ..Default::default()
            },
            ..Default::default()
        },
        api: voice_agent_server::config::AdminApiConfig::default(),
        shutdown: voice_agent_server::config::ShutdownConfig::default(),
        agent: None,
        effective_agent: voice_agent_server::config::EffectiveAgentConfig::default(),
    }
}

async fn start(
    devices: voice_agent_server::config::DatabaseDevicesConfig,
) -> (String, String, JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let database_url = database_url();
    let mut app_config = config(address, database_url.clone(), false);
    app_config.database.devices = devices;
    let router: Router = bootstrap_with_providers(app_config, Arc::new(ProviderSet::unavailable()))
        .await
        .unwrap();
    let task = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    (format!("http://{address}"), database_url, task)
}

fn admission_config() -> voice_agent_server::config::DatabaseDevicesConfig {
    voice_agent_server::config::DatabaseDevicesConfig {
        admission_enabled: true,
        ..Default::default()
    }
}

fn request(
    base: &str,
    device_id: &str,
) -> tokio_tungstenite::tungstenite::handshake::client::Request {
    let mut request = format!("{}/voice/v1/", base.replace("http", "ws"))
        .into_client_request()
        .unwrap();
    request
        .headers_mut()
        .insert("Protocol-Version", "1".parse().unwrap());
    request
        .headers_mut()
        .insert("Device-Id", device_id.parse().unwrap());
    request
        .headers_mut()
        .insert("Client-Id", "admission-test".parse().unwrap());
    request
}

fn rejected_status(error: tokio_tungstenite::tungstenite::Error) -> StatusCode {
    match error {
        tokio_tungstenite::tungstenite::Error::Http(response) => response.status(),
        other => panic!("expected HTTP WebSocket rejection, got {other:?}"),
    }
}

async fn insert_agent_and_device(database_url: &str, device_id: &str, enabled: bool) {
    let pool = SqlitePool::connect(database_url).await.unwrap();
    sqlx::query("INSERT INTO agents (key,name,enabled,created_at,updated_at) VALUES ('agent', 'Agent', 1, 1, 1)")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO devices (device_id,agent_id,enabled,created_at,updated_at) VALUES (?, 1, ?, 1, 1)")
        .bind(device_id)
        .bind(i64::from(enabled))
        .execute(&pool)
        .await
        .unwrap();
}

#[tokio::test]
async fn admission_disabled_preserves_legacy_websocket_upgrade() {
    let (base, _database_url, task) = start(Default::default()).await;
    let (_socket, response) = connect_async(request(&base, "unknown-device"))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::SWITCHING_PROTOCOLS);
    task.abort();
}

#[tokio::test]
async fn enabled_admission_rejects_unknown_and_disabled_devices_before_upgrade() {
    let (base, database_url, task) = start(admission_config()).await;
    let unknown = connect_async(request(&base, "unknown-device"))
        .await
        .unwrap_err();
    assert_eq!(rejected_status(unknown), StatusCode::FORBIDDEN);

    insert_agent_and_device(&database_url, "disabled-device", false).await;
    let disabled = connect_async(request(&base, "disabled-device"))
        .await
        .unwrap_err();
    assert_eq!(rejected_status(disabled), StatusCode::FORBIDDEN);
    task.abort();
}

#[tokio::test]
async fn admission_rejects_an_oversized_device_identity_before_database_lookup() {
    let (base, _database_url, task) = start(admission_config()).await;
    let oversized = "d".repeat(129);
    let error = connect_async(request(&base, &oversized)).await.unwrap_err();
    assert_eq!(rejected_status(error), StatusCode::BAD_REQUEST);
    task.abort();
}

#[tokio::test]
async fn enabled_admission_uses_server_defaults_when_agent_has_no_template_assignments() {
    let (base, database_url, task) = start(admission_config()).await;
    insert_agent_and_device(&database_url, "enabled-device", true).await;
    let (_socket, response) = connect_async(request(&base, "enabled-device"))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::SWITCHING_PROTOCOLS);
    task.abort();
}

#[tokio::test]
async fn admitted_connection_keeps_its_baseline_while_reconnect_re_resolves_policy() {
    let (base, database_url, task) = start(admission_config()).await;
    insert_agent_and_device(&database_url, "stable-device", true).await;
    let (mut admitted, response) = connect_async(request(&base, "stable-device"))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::SWITCHING_PROTOCOLS);
    let pool = SqlitePool::connect(&database_url).await.unwrap();
    sqlx::query("UPDATE devices SET enabled = 0 WHERE device_id = 'stable-device'")
        .execute(&pool)
        .await
        .unwrap();
    admitted
        .send(Message::Text(r#"{"type":"hello"}"#.into()))
        .await
        .unwrap();
    assert!(matches!(
        timeout(Duration::from_secs(1), admitted.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap(),
        Message::Text(_)
    ));
    let reconnect = connect_async(request(&base, "stable-device"))
        .await
        .unwrap_err();
    assert_eq!(rejected_status(reconnect), StatusCode::FORBIDDEN);
    task.abort();
}

#[tokio::test]
async fn enabled_admission_returns_coarse_503_when_its_database_is_unavailable() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let mut state = voice_agent_server::app::AppState::from_provider_set(
        config(address, database_url(), true),
        Arc::new(ProviderSet::unavailable()),
    );
    state.database = None;
    let task = tokio::spawn(async move {
        axum::serve(listener, voice_agent_server::app::router_with_state(state))
            .await
            .unwrap()
    });
    let base = format!("http://{address}");
    let error = connect_async(request(&base, "device")).await.unwrap_err();
    assert_eq!(rejected_status(error), StatusCode::SERVICE_UNAVAILABLE);
    task.abort();
}

#[tokio::test]
async fn enabled_admission_returns_coarse_503_when_its_database_pool_is_closed() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let app_config = config(address, database_url(), true);
    let database = voice_agent_server::database::Database::connect(&app_config.database)
        .await
        .unwrap();
    database.pool().close().await;
    let state = voice_agent_server::app::AppState::from_provider_set_with_database(
        app_config,
        Arc::new(ProviderSet::unavailable()),
        Some(database),
    );
    let task = tokio::spawn(async move {
        axum::serve(listener, voice_agent_server::app::router_with_state(state))
            .await
            .unwrap()
    });
    let base = format!("http://{address}");
    let error = connect_async(request(&base, "device")).await.unwrap_err();
    assert_eq!(rejected_status(error), StatusCode::SERVICE_UNAVAILABLE);
    task.abort();
}

#[tokio::test]
async fn enabled_template_assignment_without_a_resolved_profile_returns_coarse_503() {
    let (base, database_url, task) = start(admission_config()).await;
    insert_agent_and_device(&database_url, "profile-device", true).await;
    let pool = SqlitePool::connect(&database_url).await.unwrap();
    sqlx::query("INSERT INTO agent_templates (key,name,language,prompt,created_at,updated_at) VALUES ('template', 'Template', 'vi', 'Prompt', 1, 1)")
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO agent_template_assignments (agent_id,template_id,enabled,created_at) VALUES (1, 1, 1, 1)")
        .execute(&pool)
        .await
        .unwrap();
    let error = connect_async(request(&base, "profile-device"))
        .await
        .unwrap_err();
    assert_eq!(rejected_status(error), StatusCode::SERVICE_UNAVAILABLE);
    task.abort();
}

#[tokio::test]
async fn auto_registration_is_atomic_for_racing_connections_and_stores_only_safe_metadata() {
    let (base, database_url, task) = start(voice_agent_server::config::DatabaseDevicesConfig {
        admission_enabled: true,
        auto_register: true,
        auto_register_agent_key: "agent".into(),
    })
    .await;
    let pool = SqlitePool::connect(&database_url).await.unwrap();
    sqlx::query("INSERT INTO agents (key,name,enabled,created_at,updated_at) VALUES ('agent', 'Agent', 1, 1, 1)")
        .execute(&pool)
        .await
        .unwrap();

    let first = connect_async(request(&base, "racing-device"));
    let second = connect_async(request(&base, "racing-device"));
    let (first, second) = tokio::join!(first, second);
    assert_eq!(first.unwrap().1.status(), StatusCode::SWITCHING_PROTOCOLS);
    assert_eq!(second.unwrap().1.status(), StatusCode::SWITCHING_PROTOCOLS);
    let rows: Vec<(i64, i64, String)> = sqlx::query_as(
        "SELECT agent_id, enabled, metadata_json FROM devices WHERE device_id = 'racing-device'",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].0, 1);
    assert_eq!(rows[0].1, 1);
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&rows[0].2).unwrap()["source"],
        "auto_register"
    );
    task.abort();
}
