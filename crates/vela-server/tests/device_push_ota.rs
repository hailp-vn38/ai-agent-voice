use std::sync::Arc;

use axum::Router;
use sha2::Digest;
use sqlx::SqlitePool;
use tokio::{net::TcpListener, task::JoinHandle};
use url::Url;
use vela_server::{
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
            .join(format!("vela-push-{}", uuid::Uuid::new_v4()))
            .join("vela.db")
            .display()
    )
}

fn config(address: std::net::SocketAddr, database_url: String) -> AppConfig {
    AppConfig {
        orchestrator: Default::default(),
        wiki: Default::default(),
        memory: Default::default(),
        speaker_recognition: Default::default(),
        server: ServerConfig {
            bind: address,
            public_ws_url: Url::parse(&format!("ws://{address}/voice/v1/")).unwrap(),
            hello_timeout_ms: 5_000,
            trusted_origins: Vec::new(),
        },
        auth: AuthConfig::default(),
        audio: AudioConfig::default(),
        websocket: WebsocketConfig::default(),
        limits: LimitsConfig::default(),
        provider_defaults: vela_server::config::ProviderDefaultsConfig {
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
        provider_runtime: None,
        llm: LlmConfig::default(),
        tts: TtsConfig::default(),
        speech_output: SpeechOutputConfig::default(),
        barge_in: BargeInConfig::default(),
        mcp: vela_server::config::McpConfig::default(),
        system_mcp: Default::default(),
        vision: vela_server::config::VisionConfig::default(),
        database: DatabaseConfig {
            url: database_url,
            devices: Default::default(),
            ..Default::default()
        },
        api: vela_server::config::AdminApiConfig::default(),
        shutdown: vela_server::config::ShutdownConfig::default(),
        device_push: Default::default(),
        agent: None,
        effective_agent: vela_server::config::EffectiveAgentConfig::default(),
    }
}

fn push_config() -> vela_server::config::DevicePushConfig {
    vela_server::config::DevicePushConfig {
        enabled: true,
        endpoint: "push.example.test:8883".into(),
        broker_url: "mqtt://broker:1883".into(),
        secret: vela_server::config::SecretString::new("push-secret-0123456789"),
        keepalive_seconds: 45,
    }
}

async fn start(
    push: vela_server::config::DevicePushConfig,
    enrollment: bool,
) -> (String, String, JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let database_url = database_url();
    let mut app_config = config(address, database_url.clone());
    app_config.device_push = push;
    if enrollment {
        app_config.database.devices.enrollment = vela_server::config::EnrollmentConfig {
            enabled: true,
            transport: vela_server::config::EnrollmentTransport::Ota,
            ..Default::default()
        };
        app_config.api = vela_server::config::AdminApiConfig {
            enabled: true,
            jwt_secret: vela_server::config::SecretString::new(
                "enrollment-signing-secret-0123456789",
            ),
            ..Default::default()
        };
    }
    let router: Router = bootstrap_with_providers(app_config, Arc::new(ProviderSet::unavailable()))
        .await
        .unwrap();
    let task = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    (format!("http://{address}"), database_url, task)
}

async fn insert_device(database_url: &str, device_id: &str, enabled: bool) {
    let pool = SqlitePool::connect(database_url).await.unwrap();
    sqlx::query("INSERT OR IGNORE INTO agents (key,name,enabled,created_at,updated_at) VALUES ('agent', 'Agent', 1, 1, 1)")
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

async fn ota(base: &str, device_id: &str) -> (u16, serde_json::Value) {
    let response = reqwest::Client::new()
        .post(format!("{base}/voice/ota/"))
        .header("Device-Id", device_id)
        .header("Client-Id", "ota-test")
        .json(&serde_json::json!({}))
        .send()
        .await
        .unwrap();
    let status = response.status().as_u16();
    (status, response.json().await.unwrap())
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

#[tokio::test]
async fn registered_device_receives_push_block_when_enabled() {
    let (base, db, task) = start(push_config(), false).await;
    insert_device(&db, "device-1", true).await;
    let (status, body) = ota(&base, "device-1").await;
    assert_eq!(status, 200);
    let push = &body["push"];
    let key = hex(&sha2::Sha256::digest(b"device-1"))[..32].to_owned();
    assert_eq!(push["endpoint"], "push.example.test:8883");
    assert_eq!(push["username"], "device-1");
    assert_eq!(push["client_id"], format!("vela-{key}"));
    assert_eq!(push["topic"], format!("vela/v1/devices/{key}/signal"));
    assert_eq!(push["keepalive"], 45);
    let expected = ring::hmac::sign(
        &ring::hmac::Key::new(ring::hmac::HMAC_SHA256, b"push-secret-0123456789"),
        b"v1:device-1",
    );
    assert_eq!(push["password"], hex(expected.as_ref()));
    assert!(body.get("mqtt").is_none());
    task.abort();
}

#[tokio::test]
async fn device_named_like_the_service_account_gets_no_push_block() {
    let (base, db, task) = start(push_config(), false).await;
    insert_device(&db, "vela-server", true).await;
    let (status, body) = ota(&base, "vela-server").await;
    assert_eq!(status, 200);
    assert!(body.get("push").is_none());
    task.abort();
}

#[tokio::test]
async fn no_push_block_when_feature_disabled() {
    let (base, db, task) = start(Default::default(), false).await;
    insert_device(&db, "device-1", true).await;
    let (status, body) = ota(&base, "device-1").await;
    assert_eq!(status, 200);
    assert!(body.get("push").is_none());
    assert!(body.get("mqtt").is_none());
    assert!(body.get("websocket").is_some());
    task.abort();
}

#[tokio::test]
async fn no_push_block_for_pending_or_blocked_devices() {
    let (base, db, task) = start(push_config(), true).await;
    let (status, pending) = ota(&base, "new-device").await;
    assert_eq!(status, 200);
    assert!(pending.get("activation").is_some());
    assert!(pending.get("push").is_none());
    assert!(pending.get("mqtt").is_none());
    insert_device(&db, "blocked-device", false).await;
    let (status, blocked) = ota(&base, "blocked-device").await;
    assert_eq!(status, 403);
    assert!(blocked.get("push").is_none());
    task.abort();
}

#[tokio::test]
async fn awkward_device_ids_still_yield_valid_topic_and_client_id() {
    let (base, db, task) = start(push_config(), false).await;
    insert_device(&db, "a/b+c#d", true).await;
    let (_, body) = ota(&base, "a/b+c#d").await;
    let push = &body["push"];
    assert_eq!(push["username"], "a/b+c#d");
    let topic = push["topic"].as_str().unwrap();
    let client_id = push["client_id"].as_str().unwrap();
    assert!(!topic.contains(['+', '#']));
    assert_eq!(topic.matches('/').count(), 4);
    assert!(
        client_id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-')
    );
    task.abort();
}

#[tokio::test]
async fn startup_fails_when_push_enabled_but_broker_url_is_unusable() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let mut app_config = config(listener.local_addr().unwrap(), database_url());
    app_config.device_push = push_config();
    app_config.device_push.broker_url = "not-a-broker-url".into();
    let result = bootstrap_with_providers(app_config, Arc::new(ProviderSet::unavailable())).await;
    assert!(result.is_err());
}
