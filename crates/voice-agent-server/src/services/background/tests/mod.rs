use super::*;
use crate::{config::AppConfig, database::Database};
use serde_json::{Value, json};
use std::sync::Arc;
mod admission;
mod execution;
mod http;
mod hierarchical;
mod locking;
mod orchestration;
mod providers;
mod recovery;
mod voice_ws;

fn config() -> AppConfig {
    toml::from_str(&format!(
        r#"
[server]
bind="127.0.0.1:0"
public_ws_url="ws://127.0.0.1:0/voice/v1/"
[provider_defaults]
vad="test"
asr="test"
llm="planner"
tts="test"
[providers.llm.instances.planner]
adapter="openai"
base_url="https://example.test/v1"
model="fixture"
api_key="BACKGROUND_SECRET_SENTINEL"
[database]
url="sqlite://{}"
[api]
enabled=true
admin_token="background-test-token"
[background.limits]
max_queued_jobs=2
scheduler_scan_ms=20
"#,
        std::env::temp_dir()
            .join(format!("background-{}.db", uuid::Uuid::new_v4()))
            .display()
    ))
    .unwrap()
}
pub(crate) async fn harness() -> (Arc<Database>, Arc<AppConfig>, Arc<BackgroundRuntime>) {
    let config = Arc::new(config());
    let database = Arc::new(Database::connect(&config.database).await.unwrap());
    let runtime = BackgroundRuntime::foundation_harness(database.clone(), config.clone())
        .await
        .unwrap();
    enable(&runtime.engine).await;
    (database, config, runtime)
}
async fn enable(engine: &JobEngine) {
    let revision = engine.settings().await.unwrap().revision;
    engine
        .update_settings(
            BackgroundSettingsUpdate {
                enabled: true,
                orchestrator_llm_provider: Some("config:llm:planner".into()),
                worker_llm_provider: Some("config:llm:planner".into()),
            },
            revision,
        )
        .await
        .unwrap();
}
fn request(goal: &str) -> JobRequest {
    serde_json::from_value(json!({"goal":goal})).unwrap()
}
pub(crate) async fn voice(database: &Database) -> JobRequester {
    let agent = sqlx::query(
        "INSERT INTO agents(key,name,created_at,updated_at) VALUES('voice-agent','Voice',0,0)",
    )
    .execute(database.pool())
    .await
    .unwrap()
    .last_insert_rowid();
    let device=sqlx::query("INSERT INTO devices(device_id,agent_id,created_at,updated_at) VALUES('voice-device',?,0,0)").bind(agent).execute(database.pool()).await.unwrap().last_insert_rowid();
    JobRequester::Voice {
        device_incarnation: device,
        agent_incarnation: agent,
    }
}
async fn wait_for(engine: &JobEngine, id: &str, predicate: impl Fn(&JobView) -> bool) -> JobView {
    tokio::time::timeout(std::time::Duration::from_secs(3), async {
        loop {
            let job = engine.get(id).await.unwrap();
            if predicate(&job) {
                return job;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap()
}
