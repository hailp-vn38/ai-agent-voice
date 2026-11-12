use std::{sync::Arc, time::Duration};

pub use futures_util::{SinkExt, StreamExt};
pub use sqlx::SqlitePool;
pub use tokio::{net::TcpListener, task::JoinHandle, time::timeout};
pub use tokio_tungstenite::{
    connect_async,
    tungstenite::{Message, client::IntoClientRequest, http::StatusCode},
};
pub use url::Url;
pub use vela_server::{
    app::{AppState, SessionProfileAdmissionError, router_with_state},
    config::{
        AdminApiConfig, AppConfig, AudioConfig, AuthConfig, BargeInConfig, DatabaseConfig,
        DatabaseHistoryConfig, DeploymentConfig, ExternalMcpConfig, ExternalMcpNetworkConfig,
        LimitsConfig, LlmConfig, McpConfig, ProviderDefaultsConfig, ProvidersConfig, RuntimeConfig,
        ServerConfig, ShutdownConfig, SpeechOutputConfig, TtsConfig, VisionConfig, WebsocketConfig,
        WorkersConfig,
    },
    database::Database,
    lifecycle::{AdmissionGate, DrainOutcome, DrainRegistry, RuntimeLifecycle, ShutdownReport},
    providers::ProviderSet,
};

pub fn database_url() -> String {
    format!(
        "sqlite://{}",
        std::env::temp_dir()
            .join(format!("vela-lifecycle-{}.db", uuid::Uuid::new_v4()))
            .display()
    )
}

pub fn config(url: String) -> AppConfig {
    let address: std::net::SocketAddr = ([127, 0, 0, 1], 0).into();
    AppConfig {
        orchestrator: Default::default(),
        wiki: Default::default(),
        memory: Default::default(),
        speaker_recognition: Default::default(),
        server: ServerConfig {
            bind: address,
            public_ws_url: Url::parse(&format!("ws://{address}/voice/v1/")).unwrap(),
            hello_timeout_ms: 500,
            trusted_origins: Vec::new(),
        },
        auth: AuthConfig::default(),
        audio: AudioConfig::default(),
        websocket: WebsocketConfig::default(),
        limits: LimitsConfig::default(),
        provider_defaults: ProviderDefaultsConfig {
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
        mcp: McpConfig::default(),
        system_mcp: Default::default(),
        vision: VisionConfig::default(),
        database: DatabaseConfig {
            url,
            devices: Default::default(),
            ..Default::default()
        },
        api: AdminApiConfig::default(),
        shutdown: ShutdownConfig::default(),
        agent: None,
        effective_agent: Default::default(),
    }
}

pub fn providers() -> Arc<ProviderSet> {
    Arc::new(ProviderSet::unavailable())
}

pub struct Voice {
    pub base: String,
    pub state: AppState,
    pub lifecycle: Arc<RuntimeLifecycle>,
    pub url: String,
    #[allow(dead_code)]
    pub task: JoinHandle<()>,
}

impl Voice {
    pub async fn status(&self, route: &str) -> StatusCode {
        reqwest::get(format!("{}{route}", self.base))
            .await
            .expect("the liveness route answers")
            .status()
    }

    pub async fn body(&self, route: &str) -> String {
        reqwest::get(format!("{}{route}", self.base))
            .await
            .expect("the readiness route answers")
            .text()
            .await
            .expect("the readiness route has a body")
    }

    pub async fn database(&self) -> SqlitePool {
        SqlitePool::connect(&self.url).await.unwrap()
    }
}

pub async fn start(config: AppConfig, grace: Duration) -> Voice {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let url = config.database.url.clone();
    let database = Database::connect(&config.database).await.unwrap();
    crate::admin_auth::seed_default_admin_password(database.pool()).await;
    let lifecycle = RuntimeLifecycle::new(grace);
    let state = AppState::from_provider_set_with_database_and_shutdown(
        config,
        providers(),
        Some(database),
        Arc::clone(&lifecycle),
    );
    let served = state.clone();
    let task = tokio::spawn(async move {
        axum::serve(listener, router_with_state(served))
            .await
            .unwrap()
    });
    Voice {
        base: format!("http://{address}"),
        state,
        lifecycle,
        url,
        task,
    }
}

pub async fn start_without_database(grace: Duration) -> Voice {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let config = config(database_url());
    let lifecycle = RuntimeLifecycle::new(grace);
    let state = AppState::from_provider_set_with_database_and_shutdown(
        config,
        providers(),
        None,
        Arc::clone(&lifecycle),
    );
    let served = state.clone();
    let task = tokio::spawn(async move {
        axum::serve(listener, router_with_state(served))
            .await
            .unwrap()
    });
    Voice {
        base: format!("http://{address}"),
        state,
        lifecycle,
        url: String::new(),
        task,
    }
}

pub fn request(
    base: &str,
    device_id: &str,
) -> tokio_tungstenite::tungstenite::handshake::client::Request {
    let mut request = format!("{}/voice/v1/", base.replacen("http", "ws", 1))
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
        .insert("Client-Id", "lifecycle-test".parse().unwrap());
    request
}

pub fn hello() -> String {
    r#"{"type":"hello","version":1,"transport":"websocket","features":{}}"#.to_owned()
}

pub fn rejected_status(error: tokio_tungstenite::tungstenite::Error) -> StatusCode {
    match error {
        tokio_tungstenite::tungstenite::Error::Http(response) => response.status(),
        other => panic!("expected an HTTP WebSocket rejection, got {other:?}"),
    }
}

pub async fn seed(pool: &SqlitePool) {
    sqlx::query(
        "INSERT INTO agents (key,name,enabled,created_at,updated_at) \
                 VALUES ('agent','Agent',1,1,1)",
    )
    .execute(pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO devices (device_id,agent_id,enabled,created_at,updated_at) \
                 VALUES ('device',1,1,1,1)",
    )
    .execute(pool)
    .await
    .unwrap();
}

pub async fn await_gate_closed(gate: &AdmissionGate) {
    timeout(Duration::from_secs(5), async {
        while gate.is_open() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("the ordered shutdown closes the admission gate");
}

pub async fn await_drain(drain: &DrainRegistry) {
    timeout(Duration::from_secs(5), drain.drained())
        .await
        .expect("the registered session finishes on its own");
}

pub async fn shutdown_within(lifecycle: Arc<RuntimeLifecycle>, limit: Duration) -> ShutdownReport {
    timeout(limit, lifecycle.shutdown())
        .await
        .expect("the ordered shutdown returns within its bound")
}
