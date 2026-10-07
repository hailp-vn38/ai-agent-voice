//! Public HTTP + real SQLite tests for ticket 05: Speaker CRUD and web enrollment drafts.
//!
//! Covers auth, two-tab CAS, bounded quota/TTL, lost-response idempotency, provider in-use
//! references, and a real process-restart runtime repin / startup expiry sweep.

use std::{fs, sync::Arc};

use reqwest::{Client, StatusCode};
use serde_json::Value;
use voice_agent_server::{
    app::{AppState, router_with_state},
    audio::PcmF32Mono,
    config::AppConfig,
    database::Database,
    providers::{ProviderSet, RuntimeCatalog, speaker::SpeakerRuntime},
    services::provider_runtime::{
        PreparedRuntime, ProviderRuntimeManager, RuntimeError, RuntimeLimits, RuntimeMaterializer,
        RuntimeResource,
    },
};

const TOKEN: &str = "admin-test-token";

fn database_url() -> String {
    format!(
        "sqlite://{}",
        std::env::temp_dir()
            .join(format!("voice-agent-speaker-{}.db", uuid::Uuid::new_v4()))
            .display()
    )
}

fn config_toml(database_uri: &str, max_speakers: u32, max_open: u32, ttl_ms: u64) -> String {
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
max_speakers = {max_speakers}
[speaker_recognition.enrollment]
max_open_enrollments = {max_open}
ttl_ms = {ttl_ms}
"#
    )
}

fn write_config(database_uri: &str, max_speakers: u32, max_open: u32, ttl_ms: u64) -> AppConfig {
    let path =
        std::env::temp_dir().join(format!("voice-agent-speaker-{}.toml", uuid::Uuid::new_v4()));
    fs::write(
        &path,
        config_toml(database_uri, max_speakers, max_open, ttl_ms),
    )
    .unwrap();
    let config = AppConfig::parse_and_resolve(&path).unwrap();
    fs::remove_file(path).unwrap();
    config
}

struct TestSpeaker;
impl voice_agent_server::providers::speaker::SpeakerProvider for TestSpeaker {
    fn dimension(&self) -> usize {
        3
    }
    fn extract(
        &mut self,
        _: &PcmF32Mono,
    ) -> Result<Vec<f32>, voice_agent_server::providers::speaker::SpeakerError> {
        Ok(vec![1.0, 2.0, 3.0])
    }
}

struct TestSpeakerResource(Arc<SpeakerRuntime>);
impl RuntimeResource for TestSpeakerResource {
    fn unload(&self) -> bool {
        self.0.shutdown_acknowledged()
    }
    fn runtimes_for(
        &self,
        snapshot: &voice_agent_server::database::DesiredProvider,
        quota: voice_agent_server::workers::ProviderRuntimeAdmission,
    ) -> Option<RuntimeCatalog> {
        Some(RuntimeCatalog::single_speaker(
            snapshot.key.clone(),
            Arc::new(self.0.logical_view(quota)),
        ))
    }
}

struct TestSpeakerFactory;
impl RuntimeMaterializer for TestSpeakerFactory {
    fn estimated_peak_bytes(
        &self,
        _: &voice_agent_server::database::DesiredProvider,
    ) -> Result<u64, RuntimeError> {
        Ok(1)
    }
    fn logical_capacity(
        &self,
        _: &voice_agent_server::database::DesiredProvider,
    ) -> Result<usize, RuntimeError> {
        Ok(1)
    }
    fn build(
        &self,
        _: &voice_agent_server::database::DesiredProvider,
        _: Option<PreparedRuntime>,
        quota: voice_agent_server::workers::ProviderRuntimeAdmission,
    ) -> Result<Arc<dyn RuntimeResource>, RuntimeError> {
        Ok(Arc::new(TestSpeakerResource(Arc::new(
            SpeakerRuntime::new(Box::new(TestSpeaker), quota).unwrap(),
        ))))
    }
}

/// Boots a server against an existing database file. `with_runtime` controls whether a Speaker
/// runtime manager is attached (needed to open drafts).
async fn server_with_runtime(
    database_uri: &str,
    with_runtime: bool,
    max_open: u32,
) -> (String, tokio::task::JoinHandle<()>) {
    let config = write_config(database_uri, 2, max_open, 60_000);
    let database = Database::connect(&config.database).await.unwrap();
    let mut state = AppState::from_provider_set_with_database(
        config,
        Arc::new(ProviderSet::unavailable()),
        Some(database),
    );
    if with_runtime {
        state.provider_runtime_manager = Some(
            ProviderRuntimeManager::new(
                RuntimeLimits {
                    max_parallel_loads: 1,
                    max_pending_loads: 2,
                    max_waiters: 8,
                    max_resident_bytes: 4,
                    max_resources: 4,
                    max_version_entries: 8,
                    admission_timeout_ms: 1000,
                    failure_cooldown_ms: 100,
                    idle_ttl_ms: 1000,
                },
                Arc::new(TestSpeakerFactory),
                voice_agent_server::lifecycle::AdmissionGate::open(),
            )
            .unwrap(),
        );
    }
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let task = tokio::spawn(async move {
        axum::serve(listener, router_with_state(state))
            .await
            .unwrap()
    });
    (base, task)
}

async fn create_speaker(client: &Client, base: &str, key: &str, name: &str) -> reqwest::Response {
    client
        .post(format!("{base}/api/admin/speakers"))
        .bearer_auth(TOKEN)
        .json(&serde_json::json!({"key": key, "name": name}))
        .send()
        .await
        .unwrap()
}

async fn create_speaker_provider(client: &Client, base: &str) -> String {
    let response = client
        .post(format!("{base}/api/admin/providers"))
        .bearer_auth(TOKEN)
        .json(&serde_json::json!({
            "type": "speaker",
            "adapter": "campplus_sherpa",
            "name": "Speaker runtime",
            "config_json": {}
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
    response.json::<Value>().await.unwrap()["key"]
        .as_str()
        .unwrap()
        .to_owned()
}

#[tokio::test]
async fn speaker_crud_requires_auth_and_uses_cas_revisions() {
    let (base, task) = server_with_runtime(&database_url(), false, 1).await;
    let client = Client::new();

    // Auth is required on every Speaker route.
    let unauth = client
        .get(format!("{base}/api/admin/speakers"))
        .send()
        .await
        .unwrap();
    assert_eq!(unauth.status(), StatusCode::UNAUTHORIZED);

    let created = create_speaker(&client, &base, "owner", "Chủ sở hữu").await;
    assert_eq!(created.status(), StatusCode::CREATED);
    assert_eq!(created.headers()["etag"], "\"1\"");
    let body: Value = created.json().await.unwrap();
    assert_eq!(body["key"], "owner");
    assert_eq!(body["enabled"], true);
    assert_eq!(body["revision"], 1);
    assert_eq!(body["voiceprints"].as_array().unwrap().len(), 0);
    assert_eq!(body["enrollment_drafts"].as_array().unwrap().len(), 0);

    // Lost-response retry: the unique key makes the retry conflict, not duplicate.
    let duplicate = create_speaker(&client, &base, "owner", "Chủ sở hữu").await;
    assert_eq!(duplicate.status(), StatusCode::CONFLICT);
    assert_eq!(
        duplicate.json::<Value>().await.unwrap()["error"]["code"],
        "speaker_key_conflict"
    );

    // Missing If-Match is rejected.
    let no_match = client
        .patch(format!("{base}/api/admin/speakers/owner"))
        .bearer_auth(TOKEN)
        .json(&serde_json::json!({"name": "Khác"}))
        .send()
        .await
        .unwrap();
    assert_eq!(no_match.status(), StatusCode::BAD_REQUEST);

    let patched = client
        .patch(format!("{base}/api/admin/speakers/owner"))
        .bearer_auth(TOKEN)
        .header("if-match", "\"1\"")
        .json(&serde_json::json!({"name": "Khác"}))
        .send()
        .await
        .unwrap();
    assert_eq!(patched.status(), StatusCode::OK);
    assert_eq!(patched.headers()["etag"], "\"2\"");
    assert_eq!(patched.json::<Value>().await.unwrap()["revision"], 2);

    // Two-tab: the second writer holding revision 1 loses.
    let stale = client
        .patch(format!("{base}/api/admin/speakers/owner"))
        .bearer_auth(TOKEN)
        .header("if-match", "\"1\"")
        .json(&serde_json::json!({"name": "Đua"}))
        .send()
        .await
        .unwrap();
    assert_eq!(stale.status(), StatusCode::CONFLICT);
    assert_eq!(
        stale.json::<Value>().await.unwrap()["error"]["code"],
        "revision_conflict"
    );

    // Delete is conditional too.
    let stale_delete = client
        .delete(format!("{base}/api/admin/speakers/owner"))
        .bearer_auth(TOKEN)
        .header("if-match", "\"1\"")
        .send()
        .await
        .unwrap();
    assert_eq!(stale_delete.status(), StatusCode::CONFLICT);

    let deleted = client
        .delete(format!("{base}/api/admin/speakers/owner"))
        .bearer_auth(TOKEN)
        .header("if-match", "\"2\"")
        .send()
        .await
        .unwrap();
    assert_eq!(deleted.status(), StatusCode::NO_CONTENT);

    let missing = client
        .get(format!("{base}/api/admin/speakers/owner"))
        .bearer_auth(TOKEN)
        .send()
        .await
        .unwrap();
    assert_eq!(missing.status(), StatusCode::NOT_FOUND);

    task.abort();
}

#[tokio::test]
async fn enrollment_draft_quota_cancel_and_summary() {
    let (base, task) = server_with_runtime(&database_url(), true, 1).await;
    let client = Client::new();
    let provider_key = create_speaker_provider(&client, &base).await;

    let summary: Value = client
        .get(format!("{base}/api/admin/speaker-recognition"))
        .bearer_auth(TOKEN)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(summary["available"], true);
    assert_eq!(summary["enrollment"]["ttl_ms"], 60_000);
    assert_eq!(summary["limits"]["max_speakers"], 2);

    create_speaker(&client, &base, "owner", "Chủ sở hữu").await;
    create_speaker(&client, &base, "guest", "Khách").await;

    let draft = client
        .post(format!("{base}/api/admin/speakers/owner/enrollments"))
        .bearer_auth(TOKEN)
        .header("if-match", "\"1\"")
        .json(&serde_json::json!({
            "provider_key": provider_key,
            "expected_provider_revision": 1
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(draft.status(), StatusCode::CREATED);
    assert_eq!(draft.headers()["etag"], "\"1\"");
    let draft: Value = draft.json().await.unwrap();
    let draft_id = draft["id"].as_str().unwrap().to_owned();
    assert_eq!(draft["status"], "collecting");
    assert_eq!(draft["desired_provider_revision"], 1);
    assert_eq!(draft["loaded_provider_revision"], 1);
    assert!(
        draft["embedding_space_id"]
            .as_str()
            .unwrap()
            .starts_with("speaker:")
    );
    assert!(
        draft["runtime_id"]
            .as_str()
            .unwrap()
            .contains("voice-agent-server-")
    );

    // Lost response: a second open for the same speaker returns the existing draft id.
    let duplicate = client
        .post(format!("{base}/api/admin/speakers/owner/enrollments"))
        .bearer_auth(TOKEN)
        .header("if-match", "\"1\"")
        .json(&serde_json::json!({
            "provider_key": provider_key,
            "expected_provider_revision": 1
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(duplicate.status(), StatusCode::CONFLICT);
    let duplicate: Value = duplicate.json().await.unwrap();
    assert_eq!(duplicate["error"]["code"], "enrollment_in_progress");
    assert_eq!(duplicate["error"]["enrollment_id"], draft_id);

    // Process-wide quota (max_open_enrollments = 1) rejects a second speaker's draft.
    let quota = client
        .post(format!("{base}/api/admin/speakers/guest/enrollments"))
        .bearer_auth(TOKEN)
        .header("if-match", "\"1\"")
        .json(&serde_json::json!({
            "provider_key": provider_key,
            "expected_provider_revision": 1
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(quota.status(), StatusCode::CONFLICT);
    assert_eq!(
        quota.json::<Value>().await.unwrap()["error"]["code"],
        "enrollment_quota_exceeded"
    );

    // GET resumes the same draft.
    let resumed: Value = client
        .get(format!(
            "{base}/api/admin/speakers/owner/enrollments/{draft_id}"
        ))
        .bearer_auth(TOKEN)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(resumed["id"], draft_id);
    assert_eq!(resumed["revision"], 1);

    // Cancel releases the slot and removes the draft.
    let cancelled = client
        .delete(format!(
            "{base}/api/admin/speakers/owner/enrollments/{draft_id}"
        ))
        .bearer_auth(TOKEN)
        .header("if-match", "\"1\"")
        .send()
        .await
        .unwrap();
    assert_eq!(cancelled.status(), StatusCode::NO_CONTENT);
    let gone = client
        .get(format!(
            "{base}/api/admin/speakers/owner/enrollments/{draft_id}"
        ))
        .bearer_auth(TOKEN)
        .send()
        .await
        .unwrap();
    assert_eq!(gone.status(), StatusCode::NOT_FOUND);

    // Speaker quota: two is the cap.
    let over = create_speaker(&client, &base, "third", "Ba").await;
    assert_eq!(over.status(), StatusCode::CONFLICT);
    assert_eq!(
        over.json::<Value>().await.unwrap()["error"]["code"],
        "speaker_quota_exceeded"
    );

    task.abort();
}

#[tokio::test]
async fn provider_delete_is_blocked_while_a_draft_references_it() {
    let (base, task) = server_with_runtime(&database_url(), true, 1).await;
    let client = Client::new();
    let provider_key = create_speaker_provider(&client, &base).await;
    create_speaker(&client, &base, "owner", "Chủ sở hữu").await;

    let draft: Value = client
        .post(format!("{base}/api/admin/speakers/owner/enrollments"))
        .bearer_auth(TOKEN)
        .header("if-match", "\"1\"")
        .json(&serde_json::json!({
            "provider_key": provider_key,
            "expected_provider_revision": 1
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let draft_id = draft["id"].as_str().unwrap().to_owned();

    let blocked = client
        .delete(format!("{base}/api/admin/providers/{provider_key}"))
        .bearer_auth(TOKEN)
        .header("if-match", "\"1\"")
        .send()
        .await
        .unwrap();
    assert_eq!(blocked.status(), StatusCode::CONFLICT);
    assert_eq!(
        blocked.json::<Value>().await.unwrap()["error"]["code"],
        "provider_in_use"
    );

    client
        .delete(format!(
            "{base}/api/admin/speakers/owner/enrollments/{draft_id}"
        ))
        .bearer_auth(TOKEN)
        .header("if-match", "\"1\"")
        .send()
        .await
        .unwrap();

    let deleted = client
        .delete(format!("{base}/api/admin/providers/{provider_key}"))
        .bearer_auth(TOKEN)
        .header("if-match", "\"1\"")
        .send()
        .await
        .unwrap();
    assert_eq!(deleted.status(), StatusCode::NO_CONTENT);

    task.abort();
}

#[tokio::test]
async fn restart_repins_a_compatible_draft_and_sweeps_expired_ones() {
    let database_uri = database_url();
    let (base, task) = server_with_runtime(&database_uri, true, 2).await;
    let client = Client::new();
    let provider_key = create_speaker_provider(&client, &base).await;
    create_speaker(&client, &base, "owner", "Chủ sở hữu").await;
    create_speaker(&client, &base, "guest", "Khách").await;

    let draft: Value = client
        .post(format!("{base}/api/admin/speakers/owner/enrollments"))
        .bearer_auth(TOKEN)
        .header("if-match", "\"1\"")
        .json(&serde_json::json!({
            "provider_key": provider_key,
            "expected_provider_revision": 1
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let first_draft = draft["id"].as_str().unwrap().to_owned();
    let first_runtime = draft["runtime_id"].as_str().unwrap().to_owned();
    let space = draft["embedding_space_id"].as_str().unwrap().to_owned();

    let expired: Value = client
        .post(format!("{base}/api/admin/speakers/guest/enrollments"))
        .bearer_auth(TOKEN)
        .header("if-match", "\"1\"")
        .json(&serde_json::json!({
            "provider_key": provider_key,
            "expected_provider_revision": 1
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let expired_draft = expired["id"].as_str().unwrap().to_owned();
    task.abort();

    // Force the second draft past its TTL by hand; the startup sweep must expire it.
    let pool = sqlx::SqlitePool::connect(&database_uri).await.unwrap();
    sqlx::query("UPDATE speaker_enrollment_drafts SET created_at = 1, expires_at = 2 WHERE id = ?")
        .bind(&expired_draft)
        .execute(&pool)
        .await
        .unwrap();
    pool.close().await;

    // Fresh process incarnation over the same database file.
    let (base, task) = server_with_runtime(&database_uri, true, 2).await;
    let client = Client::new();

    let repinned: Value = client
        .get(format!(
            "{base}/api/admin/speakers/owner/enrollments/{first_draft}"
        ))
        .bearer_auth(TOKEN)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(repinned["status"], "collecting");
    assert_ne!(repinned["runtime_id"], first_runtime);
    assert_eq!(repinned["embedding_space_id"], space);
    assert_eq!(repinned["revision"], 2);

    let expired = client
        .get(format!(
            "{base}/api/admin/speakers/guest/enrollments/{expired_draft}"
        ))
        .bearer_auth(TOKEN)
        .send()
        .await
        .unwrap();
    assert_eq!(expired.status(), StatusCode::GONE);
    assert_eq!(
        expired.json::<Value>().await.unwrap()["error"]["code"],
        "enrollment_expired"
    );

    task.abort();
}
