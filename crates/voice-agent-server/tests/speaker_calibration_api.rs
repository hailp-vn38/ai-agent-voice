//! Public HTTP + real SQLite tests for ticket 14: explicit calibration-catalog reload and
//! exact-candidate-set evidence.
//!
//! Covers the fixed-source contract (no path/URL/field override), all-or-nothing publication that
//! keeps the previous catalog on failure, exact-set evidence that only invalidates the snapshots
//! it covers, and the `required` gate that stays closed until the fresh-turn gate lands.

use std::{fs, sync::Arc};

use reqwest::{Client, StatusCode};
use serde_json::Value;
use voice_agent_server::{
    app::{AppState, router_with_state},
    config::AppConfig,
    database::Database,
    providers::ProviderSet,
};

const TOKEN: &str = "admin-test-token";

fn database_url() -> String {
    format!(
        "sqlite://{}",
        std::env::temp_dir()
            .join(format!(
                "voice-agent-calibration-{}.db",
                uuid::Uuid::new_v4()
            ))
            .display()
    )
}

fn temp_path(extension: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!(
        "voice-agent-calibration-{}.{extension}",
        uuid::Uuid::new_v4()
    ))
}

fn write_config(database_uri: &str, calibration: Option<&std::path::Path>) -> AppConfig {
    let path = temp_path("toml");
    let source = calibration
        .map(|path| format!("\ncalibration_source = \"{}\"", path.display()))
        .unwrap_or_default();
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
max_candidates_per_agent = 8{source}
"#
        ),
    )
    .unwrap();
    let config = AppConfig::parse_and_resolve(&path).unwrap();
    fs::remove_file(path).unwrap();
    config
}

async fn server(
    database_uri: &str,
    calibration: Option<&std::path::Path>,
) -> (String, tokio::task::JoinHandle<()>) {
    let config = write_config(database_uri, calibration);
    let database = Database::connect(&config.database).await.unwrap();
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
    (base, task)
}

async fn create_agent(client: &Client, base: &str, key: &str) {
    let response = client
        .post(format!("{base}/api/admin/agents"))
        .bearer_auth(TOKEN)
        .json(&serde_json::json!({"key": key, "name": key}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
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

async fn assign_template(client: &Client, base: &str, agent: &str, template: &str) -> i64 {
    let response = client
        .put(format!(
            "{base}/api/admin/agents/{agent}/templates/{template}"
        ))
        .bearer_auth(TOKEN)
        .header("if-match", "\"1\"")
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    response.json::<Value>().await.unwrap()["revision"]
        .as_i64()
        .unwrap()
}

async fn grant(client: &Client, base: &str, agent: &str, speaker: &str, revision: i64) {
    let response = client
        .put(format!(
            "{base}/api/admin/agents/{agent}/speakers/{speaker}"
        ))
        .bearer_auth(TOKEN)
        .header("if-match", format!("\"{revision}\""))
        .json(&serde_json::json!({ "template_keys": ["default"] }))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
}

async fn reload(client: &Client, base: &str, body: Option<Value>) -> reqwest::Response {
    let mut request = client
        .post(format!("{base}/api/admin/speaker-recognition/reload"))
        .bearer_auth(TOKEN);
    if let Some(body) = body {
        request = request.json(&body);
    }
    request.send().await.unwrap()
}

async fn policy(client: &Client, base: &str, agent: &str) -> Value {
    client
        .get(format!("{base}/api/admin/agents/{agent}/speaker-policy"))
        .bearer_auth(TOKEN)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap()
}

fn qualified_catalog(agents: &[&str]) -> String {
    let evidence: Vec<Value> = agents
        .iter()
        .map(|agent| serde_json::json!({"agent_key": agent, "report_ref": "eval-2026-10-08"}))
        .collect();
    serde_json::json!({
        "calibration_revision": "cal-2026-10-08",
        "profiles": [{
            "space": "sherpa-3dspeaker",
            "status": "qualified",
            "report_ref": "eval-2026-10-08",
            "accept_threshold": 0.45,
            "consistency_threshold": 0.50,
        }],
        "evidence": evidence,
    })
    .to_string()
}

#[tokio::test]
async fn reload_requires_bearer_a_fixed_source_and_no_override() {
    let source = temp_path("json");
    fs::write(&source, qualified_catalog(&[])).unwrap();
    let (base, task) = server(&database_url(), Some(&source)).await;
    let client = Client::new();

    // Scope: the route is behind the Admin bearer, like the rest of the surface.
    let anonymous = client
        .post(format!("{base}/api/admin/speaker-recognition/reload"))
        .send()
        .await
        .unwrap();
    assert_eq!(anonymous.status(), StatusCode::UNAUTHORIZED);

    // An override field, path, or URL is refused even though the body is otherwise valid JSON.
    let override_attempt = reload(
        &client,
        &base,
        Some(serde_json::json!({"path": "/tmp/evil.json"})),
    )
    .await;
    assert_eq!(override_attempt.status(), StatusCode::BAD_REQUEST);
    assert_eq!(
        override_attempt.json::<Value>().await.unwrap()["error"]["code"],
        "calibration_reload_override_not_allowed"
    );

    // A parameterless body (or `{}`) is the only accepted shape.
    let published = reload(&client, &base, Some(serde_json::json!({}))).await;
    assert_eq!(published.status(), StatusCode::OK);
    assert_eq!(
        published.json::<Value>().await.unwrap()["calibration_revision"],
        "cal-2026-10-08"
    );

    task.abort();
}

#[tokio::test]
async fn reload_without_a_configured_source_reports_unavailable() {
    let (base, task) = server(&database_url(), None).await;
    let client = Client::new();

    let response = reload(&client, &base, None).await;
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(
        response.json::<Value>().await.unwrap()["error"]["code"],
        "speaker_catalog_unavailable"
    );

    task.abort();
}

#[tokio::test]
async fn invalid_reload_keeps_the_previous_catalog_published() {
    let source = temp_path("json");
    fs::write(&source, qualified_catalog(&[])).unwrap();
    let (base, task) = server(&database_url(), Some(&source)).await;
    let client = Client::new();

    assert_eq!(reload(&client, &base, None).await.status(), StatusCode::OK);

    // An unknown Agent is a referential failure, so nothing is published.
    fs::write(&source, qualified_catalog(&["missing-agent"])).unwrap();
    let rejected = reload(&client, &base, None).await;
    assert_eq!(rejected.status(), StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(
        rejected.json::<Value>().await.unwrap()["error"]["code"],
        "calibration_invalid"
    );

    // Malformed JSON is rejected the same way.
    fs::write(&source, "{ not json").unwrap();
    assert_eq!(
        reload(&client, &base, None).await.status(),
        StatusCode::UNPROCESSABLE_ENTITY
    );

    // The earlier catalog is still the published one.
    let summary: Value = client
        .get(format!("{base}/api/admin/speaker-recognition"))
        .bearer_auth(TOKEN)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(summary["calibration"]["revision"], "cal-2026-10-08");
    assert_eq!(summary["calibration"]["status"], "qualified");

    task.abort();
}

#[tokio::test]
async fn qualified_reload_makes_required_selectable() {
    let source = temp_path("json");
    fs::write(&source, qualified_catalog(&["kitchen"])).unwrap();
    let (base, task) = server(&database_url(), Some(&source)).await;
    let client = Client::new();
    create_agent(&client, &base, "kitchen").await;
    create_speaker(&client, &base, "owner").await;
    create_template(&client, &base, "default").await;
    let revision = assign_template(&client, &base, "kitchen", "default").await;
    grant(&client, &base, "kitchen", "owner", revision).await;

    // Before any reload the calibration prerequisite is missing.
    assert_eq!(
        policy(&client, &base, "kitchen").await["required_blockers"][0],
        "speaker_calibration_required"
    );

    assert_eq!(reload(&client, &base, None).await.status(), StatusCode::OK);

    // Calibration is now qualified for this exact candidate set, so the fresh-turn gate (ticket 15)
    // makes `required` selectable.
    let body = policy(&client, &base, "kitchen").await;
    assert_eq!(body["required_available"], true);
    assert_eq!(body["required_blockers"], serde_json::json!([]));

    let enable = client
        .put(format!("{base}/api/admin/agents/kitchen/speaker-policy"))
        .bearer_auth(TOKEN)
        .header("if-match", "\"1\"")
        .json(&serde_json::json!({ "mode": "required" }))
        .send()
        .await
        .unwrap();
    assert_eq!(enable.status(), StatusCode::OK);

    task.abort();
}

#[tokio::test]
async fn reload_revokes_only_the_removed_exact_set() {
    let source = temp_path("json");
    let (base, task) = server(&database_url(), Some(&source)).await;
    let client = Client::new();
    create_template(&client, &base, "default").await;
    for agent in ["kitchen", "office"] {
        create_agent(&client, &base, agent).await;
        create_speaker(&client, &base, agent).await;
        let revision = assign_template(&client, &base, agent, "default").await;
        grant(&client, &base, agent, agent, revision).await;
    }

    fs::write(&source, qualified_catalog(&["kitchen", "office"])).unwrap();
    let published: Value = reload(&client, &base, None).await.json().await.unwrap();
    assert_eq!(published["evidence"].as_array().unwrap().len(), 2);
    assert_eq!(
        published["revoked_candidate_sets"]
            .as_array()
            .unwrap()
            .len(),
        0
    );

    // Dropping one entry revokes exactly that snapshot and leaves the other qualified.
    fs::write(&source, qualified_catalog(&["kitchen"])).unwrap();
    let published: Value = reload(&client, &base, None).await.json().await.unwrap();
    assert_eq!(published["evidence"].as_array().unwrap().len(), 1);
    assert_eq!(
        published["revoked_candidate_sets"]
            .as_array()
            .unwrap()
            .len(),
        1
    );

    assert_eq!(
        policy(&client, &base, "kitchen").await["required_blockers"],
        serde_json::json!([])
    );
    assert_eq!(
        policy(&client, &base, "office").await["required_blockers"],
        serde_json::json!(["speaker_calibration_required"])
    );

    task.abort();
}

#[tokio::test]
async fn new_calibration_revision_does_not_qualify_stale_evidence() {
    let source = temp_path("json");
    fs::write(&source, qualified_catalog(&["kitchen"])).unwrap();
    let (base, task) = server(&database_url(), Some(&source)).await;
    let client = Client::new();
    create_agent(&client, &base, "kitchen").await;
    create_speaker(&client, &base, "owner").await;
    create_template(&client, &base, "default").await;
    let revision = assign_template(&client, &base, "kitchen", "default").await;
    grant(&client, &base, "kitchen", "owner", revision).await;

    assert_eq!(reload(&client, &base, None).await.status(), StatusCode::OK);
    assert_eq!(
        policy(&client, &base, "kitchen").await["required_blockers"],
        serde_json::json!([])
    );

    // A contract change is a new calibration revision; evidence qualified under the old revision
    // does not carry over, and this new revision publishes no evidence for the Agent.
    let mut file: Value = serde_json::from_str(&qualified_catalog(&[])).unwrap();
    file["calibration_revision"] = serde_json::json!("cal-2026-11-01");
    fs::write(&source, file.to_string()).unwrap();
    assert_eq!(reload(&client, &base, None).await.status(), StatusCode::OK);
    assert_eq!(
        policy(&client, &base, "kitchen").await["required_blockers"],
        serde_json::json!(["speaker_calibration_required"])
    );

    // A valid domain mutation still saves with no evidence at all; only enabling `required` is gated.
    let observed = client
        .put(format!("{base}/api/admin/agents/kitchen/speaker-policy"))
        .bearer_auth(TOKEN)
        .header("if-match", "\"1\"")
        .json(&serde_json::json!({ "mode": "observe" }))
        .send()
        .await
        .unwrap();
    assert_eq!(observed.status(), StatusCode::OK);

    task.abort();
}
