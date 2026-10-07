use std::sync::Arc;

use axum::Router;
use reqwest::{Client, StatusCode};
use sqlx::SqlitePool;
use tokio::task::JoinHandle;
use url::Url;
use voice_agent_server::{
    app::{BootstrapError, bootstrap_with_providers},
    config::{
        AdminApiConfig, AppConfig, AudioConfig, AuthConfig, BargeInConfig, DatabaseConfig,
        DeploymentConfig, LimitsConfig, LlmConfig, McpConfig, ProviderDefaultsConfig,
        ProvidersConfig, RuntimeConfig, ServerConfig, SpeechOutputConfig, TtsConfig, VisionConfig,
        WebsocketConfig, WorkersConfig,
    },
    database::Database,
    providers::ProviderSet,
};

const ADMIN_TOKEN: &str = "admin-test-token";
const VALID_LLM_CONFIG: &str = r#"{"base_url":"https://api.example.test/v1","model":"test-model"}"#;
const INVALID_LLM_CONFIG: &str =
    r#"{"base_url":"https://api.example.test/v1","model":"m","token":"s"}"#;

fn database_url() -> String {
    format!(
        "sqlite://{}",
        std::env::temp_dir()
            .join(format!("voice-agent-load-plan-{}.db", uuid::Uuid::new_v4()))
            .display()
    )
}

fn config(url: String) -> AppConfig {
    AppConfig {
        speaker_recognition: Default::default(),
        server: ServerConfig {
            bind: "127.0.0.1:0".parse().unwrap(),
            public_ws_url: Url::parse("ws://127.0.0.1:0/voice/v1/").unwrap(),
            hello_timeout_ms: 500,
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
        vision: VisionConfig::default(),
        database: DatabaseConfig {
            url,
            devices: Default::default(),
            ..Default::default()
        },
        api: AdminApiConfig {
            enabled: true,
            admin_token: ADMIN_TOKEN.into(),
            ..Default::default()
        },
        shutdown: Default::default(),
        agent: None,
        effective_agent: Default::default(),
    }
}

/// Opens and migrates the control plane first, so a test can seed the graph that startup reads.
async fn seeded(url: String) -> SqlitePool {
    let database = Database::connect(&config(url.clone()).database)
        .await
        .unwrap();
    database.pool().clone()
}

async fn insert_agent(pool: &SqlitePool, key: &str, enabled: bool) -> i64 {
    sqlx::query(
        "INSERT INTO agents (key,name,enabled,created_at,updated_at) VALUES (?, ?, ?, 1, 1)",
    )
    .bind(key)
    .bind(key)
    .bind(i64::from(enabled))
    .execute(pool)
    .await
    .unwrap();
    sqlx::query_scalar("SELECT id FROM agents WHERE key = ?")
        .bind(key)
        .fetch_one(pool)
        .await
        .unwrap()
}

async fn insert_provider(
    pool: &SqlitePool,
    key: &str,
    config_json: &str,
    secret_ref: Option<&str>,
    enabled: bool,
) -> i64 {
    sqlx::query(
        "INSERT INTO providers (key,name,type,adapter,config_json,secret_ref,enabled,created_at,updated_at) \
         VALUES (?, ?, 'llm', 'openai', ?, ?, ?, 1, 1)",
    )
    .bind(key)
    .bind(key)
    .bind(config_json)
    .bind(secret_ref)
    .bind(i64::from(enabled))
    .execute(pool)
    .await
    .unwrap();
    sqlx::query_scalar("SELECT id FROM providers WHERE key = ?")
        .bind(key)
        .fetch_one(pool)
        .await
        .unwrap()
}

async fn insert_template(pool: &SqlitePool, key: &str) -> i64 {
    sqlx::query(
        "INSERT INTO agent_templates (key,name,language,prompt,enabled,created_at,updated_at) \
         VALUES (?, ?, 'vi-VN', 'stored prompt', 1, 1, 1)",
    )
    .bind(key)
    .bind(key)
    .execute(pool)
    .await
    .unwrap();
    sqlx::query_scalar("SELECT id FROM agent_templates WHERE key = ?")
        .bind(key)
        .fetch_one(pool)
        .await
        .unwrap()
}

async fn assign(
    pool: &SqlitePool,
    agent_id: i64,
    template_id: i64,
    is_default: bool,
    enabled: bool,
) {
    sqlx::query(
        "INSERT INTO agent_template_assignments (agent_id,template_id,is_default,enabled,created_at) \
         VALUES (?, ?, ?, ?, 1)",
    )
    .bind(agent_id)
    .bind(template_id)
    .bind(i64::from(is_default))
    .bind(i64::from(enabled))
    .execute(pool)
    .await
    .unwrap();
}

async fn bind_llm(pool: &SqlitePool, template_id: i64, provider_id: i64) {
    sqlx::query(
        "INSERT INTO template_provider_bindings (template_id,provider_type,provider_id,created_at,updated_at) \
         VALUES (?, 'llm', ?, 1, 1)",
    )
    .bind(template_id)
    .bind(provider_id)
    .execute(pool)
    .await
    .unwrap();
}

fn boot(config: AppConfig) -> impl std::future::Future<Output = Result<Router, BootstrapError>> {
    bootstrap_with_providers(config, Arc::new(ProviderSet::unavailable()))
}

async fn serve(url: String) -> (String, JoinHandle<()>) {
    let router = bootstrap_with_providers(config(url), Arc::new(ProviderSet::unavailable()))
        .await
        .unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let task = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    (format!("http://{address}"), task)
}

async fn provider_view(base: &str, key: &str) -> serde_json::Value {
    Client::new()
        .get(format!("{base}/api/admin/providers/{key}"))
        .bearer_auth(ADMIN_TOKEN)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap()
}

#[tokio::test]
async fn a_required_default_template_provider_failure_refuses_to_start() {
    let url = database_url();
    let pool = seeded(url.clone()).await;
    let agent = insert_agent(&pool, "agent", true).await;
    let template = insert_template(&pool, "primary").await;
    let provider = insert_provider(
        &pool,
        "db-llm",
        VALID_LLM_CONFIG,
        Some("VOICE_AGENT_TEST_ABSENT_SECRET"),
        true,
    )
    .await;
    assign(&pool, agent, template, true, true).await;
    bind_llm(&pool, template, provider).await;

    let Err(error) = boot(config(url)).await else {
        panic!("a required provider failure must block startup before the listener binds");
    };
    assert!(matches!(error, BootstrapError::Provider));
}

#[tokio::test]
async fn an_optional_non_default_provider_failure_starts_and_reports_unavailable() {
    let url = database_url();
    let pool = seeded(url.clone()).await;
    let agent = insert_agent(&pool, "agent", true).await;
    let template = insert_template(&pool, "secondary").await;
    let provider = insert_provider(
        &pool,
        "db-llm",
        VALID_LLM_CONFIG,
        Some("VOICE_AGENT_TEST_ABSENT_SECRET"),
        true,
    )
    .await;
    assign(&pool, agent, template, false, true).await;
    bind_llm(&pool, template, provider).await;

    let (base, task) = serve(url).await;
    let response = Client::new()
        .get(format!("{base}/api/admin/providers/db-llm"))
        .bearer_auth(ADMIN_TOKEN)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let view: serde_json::Value = response.json().await.unwrap();
    assert_eq!(view["runtime_status"], "unavailable");
    assert_eq!(view["runtime_matches_desired"], false);
    assert_eq!(view["requires_restart"], true);
    assert!(
        view.get("has_secret_value").is_none(),
        "runtime reporting must never expose a credential: {view}"
    );
    task.abort();
}

#[tokio::test]
async fn an_unbound_provider_is_validated_without_resolving_its_secret_or_loading() {
    let url = database_url();
    let pool = seeded(url.clone()).await;
    insert_provider(
        &pool,
        "db-llm",
        VALID_LLM_CONFIG,
        Some("VOICE_AGENT_TEST_ABSENT_SECRET"),
        true,
    )
    .await;

    let (base, task) = serve(url).await;
    let view = provider_view(&base, "db-llm").await;
    assert_eq!(
        view["runtime_status"], "not_loaded",
        "an unbound provider must skip secret resolution and runtime build: {view}"
    );
    task.abort();
}

#[tokio::test]
async fn an_unbound_provider_with_invalid_config_does_not_block_boot() {
    let url = database_url();
    let pool = seeded(url.clone()).await;
    insert_provider(&pool, "db-llm", INVALID_LLM_CONFIG, None, true).await;

    let (base, task) = serve(url).await;
    let view = provider_view(&base, "db-llm").await;
    assert_eq!(view["runtime_status"], "unavailable");
    assert_eq!(view["runtime_matches_desired"], false);
    task.abort();
}

#[tokio::test]
async fn a_provider_bound_only_by_a_disabled_agent_stays_out_of_the_load_plan() {
    let url = database_url();
    let pool = seeded(url.clone()).await;
    let agent = insert_agent(&pool, "agent", false).await;
    let template = insert_template(&pool, "primary").await;
    let provider = insert_provider(
        &pool,
        "db-llm",
        VALID_LLM_CONFIG,
        Some("VOICE_AGENT_TEST_ABSENT_SECRET"),
        true,
    )
    .await;
    assign(&pool, agent, template, true, true).await;
    bind_llm(&pool, template, provider).await;

    let (base, task) = serve(url).await;
    let view = provider_view(&base, "db-llm").await;
    assert_eq!(
        view["runtime_status"], "not_loaded",
        "a disabled agent cannot make a provider required: {view}"
    );
    task.abort();
}

#[tokio::test]
async fn a_required_provider_that_collides_with_a_deployment_instance_refuses_to_start() {
    let url = database_url();
    let pool = seeded(url.clone()).await;
    let agent = insert_agent(&pool, "agent", true).await;
    let template = insert_template(&pool, "primary").await;
    // The row is otherwise perfectly loadable, so startup can only refuse because the key the
    // binding names is already owned by the deployment's own instance.
    let provider = insert_provider(&pool, "test", VALID_LLM_CONFIG, None, true).await;
    assign(&pool, agent, template, true, true).await;
    bind_llm(&pool, template, provider).await;

    let Err(error) = boot(config(url)).await else {
        panic!("a required provider key collision must be a startup validation error");
    };
    assert!(matches!(error, BootstrapError::Provider));
}

#[tokio::test]
async fn an_optional_non_default_provider_that_loads_becomes_a_usable_candidate() {
    let url = database_url();
    let pool = seeded(url.clone()).await;
    let agent = insert_agent(&pool, "agent", true).await;
    let template = insert_template(&pool, "secondary").await;
    let provider = insert_provider(&pool, "db-llm", VALID_LLM_CONFIG, None, true).await;
    assign(&pool, agent, template, false, true).await;
    bind_llm(&pool, template, provider).await;

    let (base, task) = serve(url).await;
    let view = provider_view(&base, "db-llm").await;
    assert_eq!(
        view["runtime_status"], "loaded",
        "an optional provider is still attempted at startup: {view}"
    );
    assert_eq!(view["runtime_matches_desired"], true);
    task.abort();
}

#[tokio::test]
async fn a_loaded_database_provider_reports_desired_versus_loaded_revision() {
    let url = database_url();
    let pool = seeded(url.clone()).await;
    let agent = insert_agent(&pool, "agent", true).await;
    let template = insert_template(&pool, "primary").await;
    let provider = insert_provider(&pool, "db-llm", VALID_LLM_CONFIG, None, true).await;
    assign(&pool, agent, template, true, true).await;
    bind_llm(&pool, template, provider).await;

    let (base, task) = serve(url).await;
    let loaded = provider_view(&base, "db-llm").await;
    assert_eq!(loaded["runtime_status"], "loaded");
    assert_eq!(loaded["runtime_matches_desired"], true);
    assert_eq!(loaded["requires_restart"], false);

    // A desired revision written after startup never reaches the running process: the loaded
    // runtime keeps serving while Admin is told a restart is required.
    sqlx::query("UPDATE providers SET revision = revision + 1 WHERE key = 'db-llm'")
        .execute(&pool)
        .await
        .unwrap();
    let stale = provider_view(&base, "db-llm").await;
    assert_eq!(stale["runtime_status"], "loaded");
    assert_eq!(stale["runtime_matches_desired"], false);
    assert_eq!(stale["requires_restart"], true);
    task.abort();
}
