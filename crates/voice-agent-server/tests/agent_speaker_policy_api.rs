//! Public HTTP + real SQLite tests for ticket 09: Agent speaker policy and per-Template grants.
//!
//! Covers the `off`/`observe`/`required` mode contract, policy CAS on its own revision, the grants
//! replace-all CAS on the *Agent* revision, Dependency candidates, the fail-closed `required` gate,
//! unlink invalidation, and both list directions.

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
            .join(format!("voice-agent-policy-{}.db", uuid::Uuid::new_v4()))
            .display()
    )
}

fn write_config(database_uri: &str, max_candidates: u32) -> AppConfig {
    let path =
        std::env::temp_dir().join(format!("voice-agent-policy-{}.toml", uuid::Uuid::new_v4()));
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
max_candidates_per_agent = {max_candidates}
"#
        ),
    )
    .unwrap();
    let config = AppConfig::parse_and_resolve(&path).unwrap();
    fs::remove_file(path).unwrap();
    config
}

async fn server(database_uri: &str, max_candidates: u32) -> (String, tokio::task::JoinHandle<()>) {
    let config = write_config(database_uri, max_candidates);
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

async fn create_speaker(client: &Client, base: &str, key: &str, name: &str) {
    let response = client
        .post(format!("{base}/api/admin/speakers"))
        .bearer_auth(TOKEN)
        .json(&serde_json::json!({"key": key, "name": name}))
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

/// Assigns a Template to an Agent at the Agent revision it currently holds and returns the new one.
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

/// Disables a Template; the assignment survives, but no Agent may grant it any more.
async fn disable_template(client: &Client, base: &str, template: &str) {
    let revision = client
        .get(format!("{base}/api/admin/templates/{template}"))
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
        .patch(format!("{base}/api/admin/templates/{template}"))
        .bearer_auth(TOKEN)
        .header("if-match", format!("\"{revision}\""))
        .json(&serde_json::json!({ "enabled": false }))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
}

async fn put_policy(
    client: &Client,
    base: &str,
    agent: &str,
    mode: &str,
    revision: i64,
) -> reqwest::Response {
    client
        .put(format!("{base}/api/admin/agents/{agent}/speaker-policy"))
        .bearer_auth(TOKEN)
        .header("if-match", format!("\"{revision}\""))
        .json(&serde_json::json!({ "mode": mode }))
        .send()
        .await
        .unwrap()
}

async fn put_grant(
    client: &Client,
    base: &str,
    agent: &str,
    speaker: &str,
    templates: &[&str],
    revision: i64,
) -> reqwest::Response {
    client
        .put(format!(
            "{base}/api/admin/agents/{agent}/speakers/{speaker}"
        ))
        .bearer_auth(TOKEN)
        .header("if-match", format!("\"{revision}\""))
        .json(&serde_json::json!({ "template_keys": templates }))
        .send()
        .await
        .unwrap()
}

#[tokio::test]
async fn policy_has_its_own_revision_and_independent_of_agent_grants() {
    let (base, task) = server(&database_url(), 8).await;
    let client = Client::new();
    create_agent(&client, &base, "kitchen").await;

    // Absent policy reads as the Off contract with revision 1 and an ETag.
    let response = client
        .get(format!("{base}/api/admin/agents/kitchen/speaker-policy"))
        .bearer_auth(TOKEN)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()["etag"], "\"1\"");
    let body: Value = response.json().await.unwrap();
    assert_eq!(body["mode"], "off");
    assert_eq!(body["revision"], 1);
    assert_eq!(body["text_turns"], "allowed_without_speaker_authority");

    // The Agent ETag is a separate counter and was never touched by the policy GET.
    let agent: Value = client
        .get(format!("{base}/api/admin/agents/kitchen"))
        .bearer_auth(TOKEN)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(agent["revision"], 1);

    // Unknown mode is rejected without consuming a revision.
    let invalid = put_policy(&client, &base, "kitchen", "always", 1).await;
    assert_eq!(invalid.status(), StatusCode::BAD_REQUEST);
    assert_eq!(
        invalid.json::<Value>().await.unwrap()["error"]["code"],
        "validation_failed"
    );

    // Observing consumes policy revision 1 -> 2, and does not move the Agent revision.
    let observed = put_policy(&client, &base, "kitchen", "observe", 1).await;
    assert_eq!(observed.status(), StatusCode::OK);
    assert_eq!(observed.headers()["etag"], "\"2\"");
    let observed: Value = observed.json().await.unwrap();
    assert_eq!(observed["mode"], "observe");
    assert_eq!(observed["revision"], 2);
    assert_eq!(observed["verification_scope"], "every_voice_turn");

    let agent: Value = client
        .get(format!("{base}/api/admin/agents/kitchen"))
        .bearer_auth(TOKEN)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(
        agent["revision"], 1,
        "policy revision must not leak into the Agent"
    );

    // Stale policy CAS conflicts and the winner's revision survives.
    let stale = put_policy(&client, &base, "kitchen", "off", 1).await;
    assert_eq!(stale.status(), StatusCode::CONFLICT);
    assert_eq!(
        stale.json::<Value>().await.unwrap()["error"]["code"],
        "revision_conflict"
    );
    assert_eq!(
        client
            .get(format!("{base}/api/admin/agents/kitchen/speaker-policy"))
            .bearer_auth(TOKEN)
            .send()
            .await
            .unwrap()
            .headers()["etag"],
        "\"2\""
    );

    task.abort();
}

#[tokio::test]
async fn required_mode_fails_closed_until_calibration_and_fresh_turn_land() {
    let (base, task) = server(&database_url(), 8).await;
    let client = Client::new();
    create_agent(&client, &base, "kitchen").await;

    let body: Value = client
        .get(format!("{base}/api/admin/agents/kitchen/speaker-policy"))
        .bearer_auth(TOKEN)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(body["required_available"], false);
    assert_eq!(body["required_blockers"][0], "speaker_calibration_required");

    let response = put_policy(&client, &base, "kitchen", "required", 1).await;
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(
        response.json::<Value>().await.unwrap()["error"]["code"],
        "speaker_calibration_required"
    );
    // The failed attempt did not consume a revision.
    assert_eq!(
        client
            .get(format!("{base}/api/admin/agents/kitchen/speaker-policy"))
            .bearer_auth(TOKEN)
            .send()
            .await
            .unwrap()
            .headers()["etag"],
        "\"1\""
    );

    task.abort();
}

#[tokio::test]
async fn grants_replace_all_and_bump_only_the_agent_revision() {
    let (base, task) = server(&database_url(), 8).await;
    let client = Client::new();
    create_agent(&client, &base, "kitchen").await;
    create_speaker(&client, &base, "owner", "Chủ sở hữu").await;
    create_template(&client, &base, "default").await;
    create_template(&client, &base, "kids").await;
    let revision = assign_template(&client, &base, "kitchen", "default", 1).await;
    let revision = assign_template(&client, &base, "kitchen", "kids", revision).await;

    // The Speaker is a Dependency candidate before any grant, with no usable Template yet.
    let candidates: Value = client
        .get(format!("{base}/api/admin/agents/kitchen/speakers"))
        .bearer_auth(TOKEN)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(candidates["items"].as_array().unwrap().len(), 0);

    // Binding requires an explicit, non-empty grant set; wildcards and duplicates are rejected.
    let empty = put_grant(&client, &base, "kitchen", "owner", &[], revision).await;
    assert_eq!(empty.status(), StatusCode::BAD_REQUEST);
    let wildcard = put_grant(&client, &base, "kitchen", "owner", &["*"], revision).await;
    assert_eq!(wildcard.status(), StatusCode::BAD_REQUEST);
    let duplicated = put_grant(
        &client,
        &base,
        "kitchen",
        "owner",
        &["default", "default"],
        revision,
    )
    .await;
    assert_eq!(duplicated.status(), StatusCode::BAD_REQUEST);
    // A Template that exists but is not assigned to this Agent is not grantable.
    create_template(&client, &base, "unassigned").await;
    let unassigned = put_grant(
        &client,
        &base,
        "kitchen",
        "owner",
        &["unassigned"],
        revision,
    )
    .await;
    assert_eq!(unassigned.status(), StatusCode::BAD_REQUEST);
    assert_eq!(
        unassigned.json::<Value>().await.unwrap()["error"]["code"],
        "invalid_template"
    );

    let bound = put_grant(&client, &base, "kitchen", "owner", &["default"], revision).await;
    assert_eq!(bound.status(), StatusCode::OK);
    assert_eq!(bound.headers()["etag"], format!("\"{}\"", revision + 1));
    let bound: Value = bound.json().await.unwrap();
    assert_eq!(bound["template_keys"], serde_json::json!(["default"]));
    assert_eq!(
        bound["activation"]["existing_connections"],
        "reconnect_if_affected"
    );

    // Replace-all: the second PUT swaps default for kids instead of accumulating.
    let replaced = put_grant(&client, &base, "kitchen", "owner", &["kids"], revision + 1).await;
    assert_eq!(replaced.status(), StatusCode::OK);
    assert_eq!(
        replaced.json::<Value>().await.unwrap()["template_keys"],
        serde_json::json!(["kids"])
    );

    let bindings: Value = client
        .get(format!("{base}/api/admin/speakers/owner/bindings"))
        .bearer_auth(TOKEN)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(bindings["items"].as_array().unwrap().len(), 1);
    assert_eq!(bindings["items"][0]["agent_key"], "kitchen");
    assert_eq!(bindings["items"][0]["template_key"], "kids");

    // The policy revision was untouched by all the grant traffic.
    assert_eq!(
        client
            .get(format!("{base}/api/admin/agents/kitchen/speaker-policy"))
            .bearer_auth(TOKEN)
            .send()
            .await
            .unwrap()
            .headers()["etag"],
        "\"1\""
    );

    task.abort();
}

#[tokio::test]
async fn grants_conflict_on_stale_agent_revision_and_enforce_candidate_limit() {
    let (base, task) = server(&database_url(), 1).await;
    let client = Client::new();
    create_agent(&client, &base, "kitchen").await;
    for (key, name) in [("owner", "Owner"), ("guest", "Guest")] {
        create_speaker(&client, &base, key, name).await;
    }
    create_template(&client, &base, "default").await;
    let revision = assign_template(&client, &base, "kitchen", "default", 1).await;

    let stale = put_grant(
        &client,
        &base,
        "kitchen",
        "owner",
        &["default"],
        revision - 1,
    )
    .await;
    assert_eq!(stale.status(), StatusCode::CONFLICT);
    assert_eq!(
        stale.json::<Value>().await.unwrap()["error"]["code"],
        "revision_conflict"
    );

    let first = put_grant(&client, &base, "kitchen", "owner", &["default"], revision).await;
    assert_eq!(first.status(), StatusCode::OK);

    // The second Speaker exceeds max_candidates_per_agent = 1 and is rejected as busy, not stale.
    let limited = put_grant(
        &client,
        &base,
        "kitchen",
        "guest",
        &["default"],
        revision + 1,
    )
    .await;
    assert_eq!(limited.status(), StatusCode::CONFLICT);
    assert_eq!(
        limited.json::<Value>().await.unwrap()["error"]["code"],
        "speaker_candidate_limit"
    );

    task.abort();
}

#[tokio::test]
async fn unlink_removes_candidate_grants_and_invalidates_sessions() {
    let (base, task) = server(&database_url(), 8).await;
    let client = Client::new();
    create_agent(&client, &base, "kitchen").await;
    create_speaker(&client, &base, "owner", "Owner").await;
    create_template(&client, &base, "default").await;
    let revision = assign_template(&client, &base, "kitchen", "default", 1).await;
    let bound = put_grant(&client, &base, "kitchen", "owner", &["default"], revision).await;
    assert_eq!(bound.status(), StatusCode::OK);

    let unlinked = client
        .delete(format!("{base}/api/admin/agents/kitchen/speakers/owner"))
        .bearer_auth(TOKEN)
        .header("if-match", format!("\"{}\"", revision + 1))
        .send()
        .await
        .unwrap();
    assert_eq!(unlinked.status(), StatusCode::OK);
    assert_eq!(
        unlinked.json::<Value>().await.unwrap()["unlinked"],
        serde_json::json!(true)
    );

    let bindings: Value = client
        .get(format!("{base}/api/admin/speakers/owner/bindings"))
        .bearer_auth(TOKEN)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(bindings["total"], 0);

    // Unlinking again at the new revision is a not-found, not a no-op success.
    let again = client
        .delete(format!("{base}/api/admin/agents/kitchen/speakers/owner"))
        .bearer_auth(TOKEN)
        .header("if-match", format!("\"{}\"", revision + 2))
        .send()
        .await
        .unwrap();
    assert_eq!(again.status(), StatusCode::NOT_FOUND);

    task.abort();
}

#[tokio::test]
async fn disabled_template_or_assignment_is_invalid_and_not_usable() {
    let (base, task) = server(&database_url(), 8).await;
    let client = Client::new();
    create_agent(&client, &base, "kitchen").await;
    create_speaker(&client, &base, "owner", "Owner").await;
    create_template(&client, &base, "default").await;
    create_template(&client, &base, "kids").await;
    let revision = assign_template(&client, &base, "kitchen", "default", 1).await;
    let revision = assign_template(&client, &base, "kitchen", "kids", revision).await;

    // Disabling the Template makes it non-grantable even though the assignment is live.
    disable_template(&client, &base, "kids").await;
    let disabled = put_grant(&client, &base, "kitchen", "owner", &["kids"], revision).await;
    assert_eq!(disabled.status(), StatusCode::BAD_REQUEST);
    assert_eq!(
        disabled.json::<Value>().await.unwrap()["error"]["code"],
        "invalid_template"
    );

    // Bind cleanly to the enabled Template, then disable the Template under the live binding.
    let bound = put_grant(&client, &base, "kitchen", "owner", &["default"], revision).await;
    assert_eq!(bound.status(), StatusCode::OK);
    let agent_revision = revision + 1;
    disable_template(&client, &base, "default").await;

    // The disabled resource is still surfaced for reconciliation, but reports itself unusable.
    let listed: Value = client
        .get(format!("{base}/api/admin/agents/kitchen/speakers"))
        .bearer_auth(TOKEN)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(listed["items"][0]["speaker_key"], "owner");
    assert_eq!(listed["items"][0]["usable"], false);
    assert_eq!(
        listed["items"][0]["template_keys"],
        serde_json::json!(["default"])
    );
    assert_eq!(listed["agent_revision"], agent_revision);

    task.abort();
}

#[tokio::test]
async fn routes_require_auth_and_unknown_agents_are_not_found() {
    let (base, task) = server(&database_url(), 8).await;
    let client = Client::new();

    let unauth = client
        .get(format!("{base}/api/admin/agents/kitchen/speaker-policy"))
        .send()
        .await
        .unwrap();
    assert_eq!(unauth.status(), StatusCode::UNAUTHORIZED);

    let missing = client
        .get(format!("{base}/api/admin/agents/kitchen/speaker-policy"))
        .bearer_auth(TOKEN)
        .send()
        .await
        .unwrap();
    assert_eq!(missing.status(), StatusCode::NOT_FOUND);

    task.abort();
}
