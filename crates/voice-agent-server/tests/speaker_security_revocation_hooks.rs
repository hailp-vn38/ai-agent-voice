//! Public HTTP + real SQLite tests for ticket 16: security revocation hooks.
//!
//! Every Speaker/Agent/Template/grant/policy mutation that revokes authority must cancel the
//! admitted sessions that pinned the changed dependency, while a valid *addition* leaves an old
//! qualified snapshot alone. The registry holds sessions weakly, so a dropped session never leaks.
//!
//! These tests register a stand-in Voice Session token against the shared `ToolSecurity` registry
//! (exactly what a real WebSocket does at admission) and assert which tokens the admin surface
//! cancels.

use std::{fs, sync::Arc};

use reqwest::{Client, StatusCode};
use serde_json::Value;
use sqlx::{Row, SqlitePool};
use tokio_util::sync::CancellationToken;
use voice_agent_server::{
    app::{AppState, router_with_state},
    config::AppConfig,
    database::{Database, tool_security::SpeakerDeps},
    providers::ProviderSet,
};

const TOKEN: &str = "admin-test-token";

fn database_url() -> String {
    format!(
        "sqlite://{}",
        std::env::temp_dir()
            .join(format!(
                "voice-agent-revocation-{}.db",
                uuid::Uuid::new_v4()
            ))
            .display()
    )
}

fn write_config(database_uri: &str) -> AppConfig {
    let path = std::env::temp_dir().join(format!(
        "voice-agent-revocation-{}.toml",
        uuid::Uuid::new_v4()
    ));
    fs::write(
        &path,
        format!(
            r#"
[server]
bind = "127.0.0.1:0"
public_ws_url = "ws://127.0.0.1:0/voice/v1/"
[database]
url = "{database_uri}"
[provider_defaults]
vad = "test"
asr = "test"
llm = "test"
tts = "test"
[api]
enabled = true
admin_token = "{TOKEN}"
[speaker_recognition]
max_candidates_per_agent = 8
"#
        ),
    )
    .unwrap();
    let config = AppConfig::parse_and_resolve(&path).unwrap();
    fs::remove_file(path).unwrap();
    config
}

struct Harness {
    base: String,
    pool: SqlitePool,
    security: Arc<voice_agent_server::database::tool_security::ToolSecurity>,
    task: tokio::task::JoinHandle<()>,
}

async fn harness() -> Harness {
    let config = write_config(&database_url());
    let database = Database::connect(&config.database).await.unwrap();
    let pool = database.pool().clone();
    let security = database.tool_security.clone();
    let state = AppState::from_provider_set_with_database(
        config,
        Arc::new(ProviderSet::unavailable()),
        Some(database),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let task = tokio::spawn(async move {
        axum::serve(listener, router_with_state(state))
            .await
            .unwrap()
    });
    Harness {
        base,
        pool,
        security,
        task,
    }
}

impl Harness {
    fn client(&self) -> Client {
        Client::new()
    }

    async fn speaker_id(&self, key: &str) -> i64 {
        sqlx::query("SELECT id FROM speakers WHERE key=?")
            .bind(key)
            .fetch_one(&self.pool)
            .await
            .unwrap()
            .get("id")
    }

    /// Registers a stand-in Voice Session pinned to `speaker_key`, as admission would.
    async fn pin(&self, agent: i64, template: i64, speaker_key: &str) -> Arc<CancellationToken> {
        let speaker = self.speaker_id(speaker_key).await;
        self.security.register_speaker(SpeakerDeps {
            agent,
            template,
            speakers: vec![speaker],
            candidate_set_digest: None,
        })
    }
}

async fn create_agent(client: &Client, base: &str, key: &str) -> i64 {
    let response = client
        .post(format!("{base}/api/admin/agents"))
        .bearer_auth(TOKEN)
        .json(&serde_json::json!({"key": key, "name": key}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
    response.json::<Value>().await.unwrap()["revision"]
        .as_i64()
        .unwrap()
}

async fn create_speaker(client: &Client, base: &str, key: &str) {
    let response = client
        .post(format!("{base}/api/admin/speakers"))
        .bearer_auth(TOKEN)
        .json(&serde_json::json!({"key": key, "name": key}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
}

async fn create_template(client: &Client, base: &str, key: &str) {
    let response = client
        .post(format!("{base}/api/admin/templates"))
        .bearer_auth(TOKEN)
        .json(&serde_json::json!({
            "key": key,
            "name": key,
            "language": "vi-VN",
            "prompt": "A bounded prompt"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
}

async fn assign_template(
    client: &Client,
    base: &str,
    agent: &str,
    template: &str,
    revision: i64,
) -> i64 {
    let response = client
        .put(format!(
            "{base}/api/admin/agents/{agent}/templates/{template}"
        ))
        .bearer_auth(TOKEN)
        .header("if-match", format!("\"{revision}\""))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    response.json::<Value>().await.unwrap()["revision"]
        .as_i64()
        .unwrap()
}

async fn fetch_agent_revision(client: &Client, base: &str, key: &str) -> i64 {
    client
        .get(format!("{base}/api/admin/agents/{key}"))
        .bearer_auth(TOKEN)
        .send()
        .await
        .unwrap()
        .json::<Value>()
        .await
        .unwrap()["revision"]
        .as_i64()
        .unwrap()
}

async fn speaker_revision(client: &Client, base: &str, key: &str) -> i64 {
    client
        .get(format!("{base}/api/admin/speakers/{key}"))
        .bearer_auth(TOKEN)
        .send()
        .await
        .unwrap()
        .json::<Value>()
        .await
        .unwrap()["revision"]
        .as_i64()
        .unwrap()
}

#[tokio::test]
async fn disabling_a_speaker_cancels_only_sessions_that_pinned_it() {
    let h = harness().await;
    let client = h.client();
    create_agent(&client, &h.base, "kitchen").await;
    create_speaker(&client, &h.base, "alice").await;
    create_speaker(&client, &h.base, "bob").await;
    let agent = 1;
    let pinned_alice = h.pin(agent, 1, "alice").await;
    let pinned_bob = h.pin(agent, 1, "bob").await;

    let revision = speaker_revision(&client, &h.base, "alice").await;
    let response = client
        .patch(format!("{}/api/admin/speakers/alice", h.base))
        .bearer_auth(TOKEN)
        .header("if-match", format!("\"{revision}\""))
        .json(&serde_json::json!({"enabled": false}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    assert!(
        pinned_alice.is_cancelled(),
        "disabled speaker session stays live"
    );
    assert!(!pinned_bob.is_cancelled(), "unrelated session must survive");
    h.task.abort();
}

#[tokio::test]
async fn purging_a_voiceprint_cancels_dependent_sessions() {
    let h = harness().await;
    let client = h.client();
    create_speaker(&client, &h.base, "alice").await;
    let pinned = h.pin(1, 1, "alice").await;

    let revision = speaker_revision(&client, &h.base, "alice").await;
    let response = client
        .post(format!(
            "{}/api/admin/speakers/alice/voiceprint/purge",
            h.base
        ))
        .bearer_auth(TOKEN)
        .header("if-match", format!("\"{revision}\""))
        .json(&serde_json::json!({"confirm": "PURGE_SPEAKER_VOICEPRINT"}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert!(pinned.is_cancelled(), "purged speaker session stays live");
    h.task.abort();
}

#[tokio::test]
async fn adding_a_new_speaker_does_not_close_an_existing_snapshot() {
    let h = harness().await;
    let client = h.client();
    create_speaker(&client, &h.base, "alice").await;
    let pinned = h.pin(1, 1, "alice").await;

    // A brand new, qualified speaker is an *addition*: the old qualified snapshot is unchanged.
    create_speaker(&client, &h.base, "carol").await;

    assert!(
        !pinned.is_cancelled(),
        "addition must not close an old snapshot"
    );
    h.task.abort();
}

#[tokio::test]
async fn unlinking_a_grant_cancels_the_agents_sessions() {
    let h = harness().await;
    let client = h.client();
    let agent_revision = create_agent(&client, &h.base, "kitchen").await;
    create_speaker(&client, &h.base, "alice").await;
    create_template(&client, &h.base, "default").await;
    let agent_revision =
        assign_template(&client, &h.base, "kitchen", "default", agent_revision).await;
    // Grant then pin, so the session represents an admitted grant snapshot.
    let response = client
        .put(format!(
            "{}/api/admin/agents/kitchen/speakers/alice",
            h.base
        ))
        .bearer_auth(TOKEN)
        .header("if-match", format!("\"{agent_revision}\""))
        .json(&serde_json::json!({"template_keys": ["default"]}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let pinned = h.pin(1, 1, "alice").await;

    let revision = client
        .get(format!("{}/api/admin/agents/kitchen", h.base))
        .bearer_auth(TOKEN)
        .send()
        .await
        .unwrap()
        .json::<Value>()
        .await
        .unwrap()["revision"]
        .as_i64()
        .unwrap();
    let response = client
        .delete(format!(
            "{}/api/admin/agents/kitchen/speakers/alice",
            h.base
        ))
        .bearer_auth(TOKEN)
        .header("if-match", format!("\"{revision}\""))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert!(pinned.is_cancelled(), "unlinked speaker session stays live");
    h.task.abort();
}

#[tokio::test]
async fn disabling_a_template_cancels_sessions_that_pinned_it() {
    let h = harness().await;
    let client = h.client();
    create_speaker(&client, &h.base, "alice").await;
    create_template(&client, &h.base, "default").await;
    create_template(&client, &h.base, "other").await;
    let template = sqlx::query("SELECT id FROM agent_templates WHERE key='default'")
        .fetch_one(&h.pool)
        .await
        .unwrap()
        .get::<i64, _>("id");
    let other = sqlx::query("SELECT id FROM agent_templates WHERE key='other'")
        .fetch_one(&h.pool)
        .await
        .unwrap()
        .get::<i64, _>("id");
    let pinned_default = h.pin(1, template, "alice").await;
    let pinned_other = h.pin(1, other, "alice").await;

    let revision = client
        .get(format!("{}/api/admin/templates/default", h.base))
        .bearer_auth(TOKEN)
        .send()
        .await
        .unwrap()
        .json::<Value>()
        .await
        .unwrap()["revision"]
        .as_i64()
        .unwrap();
    let response = client
        .patch(format!("{}/api/admin/templates/default", h.base))
        .bearer_auth(TOKEN)
        .header("if-match", format!("\"{revision}\""))
        .json(&serde_json::json!({"enabled": false}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    assert!(
        pinned_default.is_cancelled(),
        "disabled template session stays live"
    );
    assert!(
        !pinned_other.is_cancelled(),
        "unrelated template session must survive"
    );
    h.task.abort();
}

#[tokio::test]
async fn turning_the_policy_off_cancels_agent_sessions() {
    let h = harness().await;
    let client = h.client();
    create_agent(&client, &h.base, "kitchen").await;
    create_speaker(&client, &h.base, "alice").await;
    // Move the Agent to `observe` so the transition to `off` is a real revocation.
    let response = client
        .put(format!(
            "{}/api/admin/agents/kitchen/speaker-policy",
            h.base
        ))
        .bearer_auth(TOKEN)
        .header("if-match", "\"1\"")
        .json(&serde_json::json!({"mode": "observe"}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let pinned = h.pin(1, 1, "alice").await;

    let response = client
        .put(format!(
            "{}/api/admin/agents/kitchen/speaker-policy",
            h.base
        ))
        .bearer_auth(TOKEN)
        .header("if-match", "\"2\"")
        .json(&serde_json::json!({"mode": "off"}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert!(pinned.is_cancelled(), "policy-off session stays live");
    h.task.abort();
}

#[tokio::test]
async fn reducing_a_grant_cancels_sessions_but_adding_one_does_not() {
    let h = harness().await;
    let client = h.client();
    let mut agent_revision = create_agent(&client, &h.base, "kitchen").await;
    create_speaker(&client, &h.base, "alice").await;
    create_template(&client, &h.base, "default").await;
    create_template(&client, &h.base, "extra").await;
    agent_revision = assign_template(&client, &h.base, "kitchen", "default", agent_revision).await;
    agent_revision = assign_template(&client, &h.base, "kitchen", "extra", agent_revision).await;
    let response = client
        .put(format!(
            "{}/api/admin/agents/kitchen/speakers/alice",
            h.base
        ))
        .bearer_auth(TOKEN)
        .header("if-match", format!("\"{agent_revision}\""))
        .json(&serde_json::json!({"template_keys": ["default"]}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let agent_revision = fetch_agent_revision(&client, &h.base, "kitchen").await;

    // Adding a Template is an expansion: it applies to new sessions only, the pinned one survives.
    let pinned = h.pin(1, 1, "alice").await;
    let response = client
        .put(format!(
            "{}/api/admin/agents/kitchen/speakers/alice",
            h.base
        ))
        .bearer_auth(TOKEN)
        .header("if-match", format!("\"{agent_revision}\""))
        .json(&serde_json::json!({"template_keys": ["default", "extra"]}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert!(
        !pinned.is_cancelled(),
        "grant addition must not close a snapshot"
    );
    let agent_revision = fetch_agent_revision(&client, &h.base, "kitchen").await;

    // Removing a Template is a reduction: the pinned session no longer holds that right.
    let response = client
        .put(format!(
            "{}/api/admin/agents/kitchen/speakers/alice",
            h.base
        ))
        .bearer_auth(TOKEN)
        .header("if-match", format!("\"{agent_revision}\""))
        .json(&serde_json::json!({"template_keys": ["default"]}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert!(
        pinned.is_cancelled(),
        "grant reduction must revoke the snapshot"
    );
    h.task.abort();
}

#[tokio::test]
async fn patching_an_agent_cancels_its_sessions() {
    let h = harness().await;
    let client = h.client();
    create_agent(&client, &h.base, "kitchen").await;
    create_speaker(&client, &h.base, "alice").await;
    let pinned = h.pin(1, 1, "alice").await;

    let response = client
        .patch(format!("{}/api/admin/agents/kitchen", h.base))
        .bearer_auth(TOKEN)
        .header("if-match", "\"1\"")
        .json(&serde_json::json!({"name": "Kitchen renamed"}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert!(
        pinned.is_cancelled(),
        "agent update must revoke its sessions"
    );
    h.task.abort();
}

#[tokio::test]
async fn a_dropped_session_is_not_kept_alive_by_the_registry() {
    let h = harness().await;
    let client = h.client();
    create_speaker(&client, &h.base, "alice").await;
    let pinned = h.pin(1, 1, "alice").await;
    let weak = Arc::downgrade(&pinned);
    drop(pinned);
    // Re-registering prunes the dead weak entry rather than holding the token alive.
    let _live = h.pin(1, 1, "alice").await;
    assert!(
        weak.upgrade().is_none(),
        "registry leaked a strong reference"
    );
    h.task.abort();
}
