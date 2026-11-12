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
            .join(format!("vela-pushauth-{}", uuid::Uuid::new_v4()))
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

async fn post(base: &str, path: &str, body: serde_json::Value) -> u16 {
    reqwest::Client::new()
        .post(format!("{base}/mqtt/auth/{path}"))
        .json(&body)
        .send()
        .await
        .unwrap()
        .status()
        .as_u16()
}

async fn check_user(base: &str, username: &str, password: &str) -> u16 {
    post(
        base,
        "user",
        serde_json::json!({"username": username, "password": password, "clientid": "c"}),
    )
    .await
}

async fn check_acl(base: &str, username: &str, topic: &str, acc: u8) -> u16 {
    post(
        base,
        "acl",
        serde_json::json!({"username": username, "clientid": "c", "topic": topic, "acc": acc}),
    )
    .await
}

fn service_password() -> String {
    let tag = ring::hmac::sign(
        &ring::hmac::Key::new(ring::hmac::HMAC_SHA256, b"push-secret-0123456789"),
        b"v1-service:vela-server",
    );
    hex(tag.as_ref())
}

#[tokio::test]
async fn credentials_from_ota_pass_user_and_topic_checks_then_block_denies() {
    let (base, db, task) = start(push_config(), false).await;
    insert_device(&db, "device-1", true).await;
    let (_, body) = ota(&base, "device-1").await;
    let push = &body["push"];
    let (user, pass, topic) = (
        push["username"].as_str().unwrap(),
        push["password"].as_str().unwrap(),
        push["topic"].as_str().unwrap(),
    );
    assert_eq!(check_user(&base, user, pass).await, 200);
    assert_eq!(check_acl(&base, user, topic, 4).await, 200);
    assert_eq!(check_acl(&base, user, topic, 1).await, 200);
    let pool = SqlitePool::connect(&db).await.unwrap();
    sqlx::query("UPDATE devices SET enabled=0 WHERE device_id='device-1'")
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(check_user(&base, user, pass).await, 403);
    assert_eq!(check_acl(&base, user, topic, 4).await, 403);
    task.abort();
}

#[tokio::test]
async fn wrong_password_unknown_device_are_denied() {
    let (base, db, task) = start(push_config(), false).await;
    insert_device(&db, "device-1", true).await;
    assert_eq!(check_user(&base, "device-1", "nope").await, 403);
    let (_, body) = ota(&base, "device-1").await;
    let pass = body["push"]["password"].as_str().unwrap().to_owned();
    assert_eq!(check_user(&base, "stranger", &pass).await, 403);
    assert_eq!(check_user(&base, "device-1", "").await, 403);
    task.abort();
}

#[tokio::test]
async fn device_acl_is_read_only_on_own_topic() {
    let (base, db, task) = start(push_config(), false).await;
    insert_device(&db, "device-1", true).await;
    insert_device(&db, "device-2", true).await;
    let own = vela_server::services::device_push::DevicePush::signal_topic("device-1");
    let other = vela_server::services::device_push::DevicePush::signal_topic("device-2");
    assert_eq!(check_acl(&base, "device-1", &own, 4).await, 200);
    assert_eq!(check_acl(&base, "device-1", &own, 2).await, 403);
    assert_eq!(check_acl(&base, "device-1", &own, 3).await, 403);
    assert_eq!(check_acl(&base, "device-1", &other, 4).await, 403);
    assert_eq!(check_acl(&base, "device-1", "#", 4).await, 403);
    task.abort();
}

#[tokio::test]
async fn service_account_publishes_signal_topics_only() {
    let (base, _db, task) = start(push_config(), false).await;
    let topic = vela_server::services::device_push::DevicePush::signal_topic("device-9");
    assert_eq!(
        check_user(&base, "vela-server", &service_password()).await,
        200
    );
    assert_eq!(check_user(&base, "vela-server", "bad").await, 403);
    assert_eq!(check_acl(&base, "vela-server", &topic, 2).await, 200);
    assert_eq!(check_acl(&base, "vela-server", &topic, 4).await, 403);
    assert_eq!(
        check_acl(&base, "vela-server", "vela/v1/devices/x/other", 2).await,
        403
    );
    assert_eq!(check_acl(&base, "vela-server", "#", 2).await, 403);
    assert_eq!(
        post(
            &base,
            "superuser",
            serde_json::json!({"username": "vela-server"})
        )
        .await,
        403
    );
    assert_eq!(
        post(
            &base,
            "superuser",
            serde_json::json!({"username": "device-1"})
        )
        .await,
        403
    );
    task.abort();
}

#[tokio::test]
async fn endpoint_absent_when_feature_disabled() {
    let (base, _db, task) = start(Default::default(), false).await;
    assert_eq!(
        check_user(&base, "vela-server", &service_password()).await,
        404
    );
    task.abort();
}

#[tokio::test]
async fn database_error_denies() {
    let (base, db, task) = start(push_config(), false).await;
    insert_device(&db, "device-1", true).await;
    let (_, body) = ota(&base, "device-1").await;
    let pass = body["push"]["password"].as_str().unwrap().to_owned();
    let pool = SqlitePool::connect(&db).await.unwrap();
    sqlx::query("ALTER TABLE devices RENAME TO devices_gone")
        .execute(&pool)
        .await
        .unwrap();
    assert_eq!(check_user(&base, "device-1", &pass).await, 403);
    task.abort();
}
