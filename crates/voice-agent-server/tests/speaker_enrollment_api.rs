//! Public HTTP + real SQLite tests for tickets 05–06: Speaker CRUD, web enrollment drafts, and
//! bounded browser WAV sample upload.
//!
//! Covers auth, two-tab CAS, bounded quota/TTL, lost-response idempotency, provider in-use
//! references, and a real process-restart runtime repin / startup expiry sweep.

use std::{fs, sync::Arc};

use reqwest::header::HeaderValue;
use reqwest::{Client, StatusCode};
use serde_json::Value;
use voice_agent_server::{
    app::{AppState, router_with_state},
    audio::PcmF32Mono,
    config::AppConfig,
    database::Database,
    providers::{ProviderSet, RuntimeCatalog, speaker::SpeakerRuntime},
    services::provider_diagnostic::{ProviderDiagnosticLimiter, ProviderDiagnosticService},
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
        pcm: &PcmF32Mono,
    ) -> Result<Vec<f32>, voice_agent_server::providers::speaker::SpeakerError> {
        // Map clip loudness to an angle so that identical recordings collapse to one point on the
        // unit circle and a deliberately different speaker lands far away. Lets the holdout and
        // consistency paths be exercised without a real embedding model.
        let samples = pcm.samples();
        let rms = (samples.iter().map(|sample| sample * sample).sum::<f32>()
            / samples.len().max(1) as f32)
            .sqrt();
        let angle = (rms * std::f32::consts::PI * 4.0).rem_euclid(std::f32::consts::TAU);
        Ok(vec![angle.cos(), angle.sin(), 0.0])
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
        let manager = ProviderRuntimeManager::new(
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
        .unwrap();
        state.provider_diagnostics = Arc::new(
            ProviderDiagnosticService::new(
                Arc::new(Default::default()),
                None,
                state.database.clone(),
                ProviderDiagnosticLimiter::new(1),
                std::time::Duration::from_secs(5),
            )
            .with_runtime_manager(manager.clone()),
        );
        state.provider_runtime_manager = Some(manager);
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

// ---------------------------------------------------------------------------
// Ticket 06: browser WAV sample upload
// ---------------------------------------------------------------------------

/// A PCM16 mono 16 kHz WAV of `ms` milliseconds alternating between `amplitude` and its inverse.
fn enrollment_wav(ms: u64, amplitude: i16) -> Vec<u8> {
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: 16_000,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut buffer = std::io::Cursor::new(Vec::new());
    let mut writer = hound::WavWriter::new(&mut buffer, spec).unwrap();
    for index in 0..ms * 16 {
        let sample = if index % 2 == 0 {
            amplitude
        } else {
            -amplitude
        };
        writer.write_sample(sample).unwrap();
    }
    writer.finalize().unwrap();
    buffer.into_inner()
}

/// A WAV differing from [`enrollment_wav`] only by amplitude, i.e. a different speaker for the
/// test provider's loudness-mapped embedding.
fn other_speaker_wav(ms: u64) -> Vec<u8> {
    enrollment_wav(ms, 1_000)
}

/// Same loudness (so the same embedding) as [`enrollment_wav`] but a different exact PCM, i.e. a
/// distinct recording of the same speaker.
fn same_speaker_holdout(ms: u64) -> Vec<u8> {
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: 16_000,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut buffer = std::io::Cursor::new(Vec::new());
    let mut writer = hound::WavWriter::new(&mut buffer, spec).unwrap();
    for index in 0..ms * 16 {
        let sample = if index % 4 < 2 { 8_000 } else { -8_000 };
        writer.write_sample(sample).unwrap();
    }
    writer.finalize().unwrap();
    buffer.into_inner()
}

async fn open_owner_draft(client: &Client, base: &str) -> String {
    let provider_key = create_speaker_provider(client, base).await;
    create_speaker(client, base, "owner", "Chủ sở hữu").await;
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
    draft["id"].as_str().unwrap().to_owned()
}

fn sample_url(base: &str, draft_id: &str, slot: u32) -> String {
    format!("{base}/api/admin/speakers/owner/enrollments/{draft_id}/samples/{slot}")
}

async fn put_sample(
    client: &Client,
    base: &str,
    draft_id: &str,
    slot: u32,
    revision: &str,
    content_type: &str,
    body: Vec<u8>,
) -> reqwest::Response {
    client
        .put(sample_url(base, draft_id, slot))
        .bearer_auth(TOKEN)
        .header("if-match", format!("\"{revision}\""))
        .header("content-type", content_type)
        .body(body)
        .send()
        .await
        .unwrap()
}

#[tokio::test]
async fn sample_upload_stores_bounded_metadata_and_returns_draft() {
    let (base, task) = server_with_runtime(&database_url(), true, 2).await;
    let client = Client::new();
    let draft_id = open_owner_draft(&client, &base).await;

    let accepted = put_sample(
        &client,
        &base,
        &draft_id,
        1,
        "1",
        "audio/wav",
        enrollment_wav(8_000, 8_000),
    )
    .await;
    let status = accepted.status();
    let etag = accepted.headers().get("etag").cloned();
    let text = accepted.text().await.unwrap();
    assert_eq!(status, StatusCode::OK, "{text}");
    assert_eq!(etag, Some(HeaderValue::from_static("\"2\"")), "{text}");
    let body: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(body["revision"], 2);
    let sample = &body["samples"][0];
    assert_eq!(sample["slot"], 1);
    assert_eq!(sample["status"], "accepted");
    assert_eq!(sample["quality"], "good");
    assert_eq!(sample["duration_ms"], 8_000);
    assert!(sample["speech_ms"].as_u64().unwrap() >= 3_000);
    // Only bounded metadata leaves the server; no audio, no vector.
    assert!(sample.get("vector").is_none());
    assert!(sample.get("audio").is_none());

    // The stored sample is visible on resume and holds no raw audio bytes.
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
    assert_eq!(resumed["samples"].as_array().unwrap().len(), 1);
    assert_eq!(resumed["samples"][0]["slot"], 1);

    task.abort();
}

#[tokio::test]
async fn sample_upload_rejects_bad_transport_without_mutation() {
    let (base, task) = server_with_runtime(&database_url(), true, 2).await;
    let client = Client::new();
    let draft_id = open_owner_draft(&client, &base).await;

    let wrong_type = put_sample(
        &client,
        &base,
        &draft_id,
        1,
        "1",
        "application/octet-stream",
        enrollment_wav(8_000, 8_000),
    )
    .await;
    assert_eq!(wrong_type.status(), StatusCode::UNSUPPORTED_MEDIA_TYPE);
    assert_eq!(
        wrong_type.json::<Value>().await.unwrap()["error"]["code"],
        "unsupported_audio_format"
    );

    let encoded = client
        .put(sample_url(&base, &draft_id, 1))
        .bearer_auth(TOKEN)
        .header("if-match", "\"1\"")
        .header("content-type", "audio/wav")
        .header("content-encoding", "gzip")
        .body(enrollment_wav(8_000, 8_000))
        .send()
        .await
        .unwrap();
    assert_eq!(encoded.status(), StatusCode::UNSUPPORTED_MEDIA_TYPE);
    assert_eq!(
        encoded.json::<Value>().await.unwrap()["error"]["code"],
        "unsupported_content_encoding"
    );

    let malformed = put_sample(
        &client,
        &base,
        &draft_id,
        1,
        "1",
        "audio/wav",
        b"definitely not a wav".to_vec(),
    )
    .await;
    assert_eq!(malformed.status(), StatusCode::UNSUPPORTED_MEDIA_TYPE);

    let oversized = put_sample(
        &client,
        &base,
        &draft_id,
        1,
        "1",
        "audio/wav",
        vec![0u8; 600_000],
    )
    .await;
    assert_eq!(oversized.status(), StatusCode::PAYLOAD_TOO_LARGE);
    assert_eq!(
        oversized.json::<Value>().await.unwrap()["error"]["code"],
        "request_too_large"
    );

    let bad_slot = put_sample(
        &client,
        &base,
        &draft_id,
        6,
        "1",
        "audio/wav",
        enrollment_wav(8_000, 8_000),
    )
    .await;
    assert_eq!(bad_slot.status(), StatusCode::BAD_REQUEST);

    // Every rejected request left revision 1 untouched.
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
    assert_eq!(resumed["revision"], 1);
    assert_eq!(resumed["samples"].as_array().unwrap().len(), 0);

    task.abort();
}

#[tokio::test]
async fn sample_upload_rejects_low_quality_without_mutation() {
    let (base, task) = server_with_runtime(&database_url(), true, 2).await;
    let client = Client::new();
    let draft_id = open_owner_draft(&client, &base).await;

    let short = put_sample(
        &client,
        &base,
        &draft_id,
        1,
        "1",
        "audio/wav",
        enrollment_wav(2_000, 8_000),
    )
    .await;
    assert_eq!(short.status(), StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(
        short.json::<Value>().await.unwrap()["error"]["code"],
        "speaker_insufficient_audio"
    );

    let silent = put_sample(
        &client,
        &base,
        &draft_id,
        1,
        "1",
        "audio/wav",
        enrollment_wav(6_000, 0),
    )
    .await;
    assert_eq!(silent.status(), StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(
        silent.json::<Value>().await.unwrap()["error"]["code"],
        "speaker_insufficient_audio"
    );

    let clipped = put_sample(
        &client,
        &base,
        &draft_id,
        1,
        "1",
        "audio/wav",
        enrollment_wav(6_000, i16::MAX),
    )
    .await;
    assert_eq!(clipped.status(), StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(
        clipped.json::<Value>().await.unwrap()["error"]["code"],
        "speaker_audio_clipped"
    );

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
    assert_eq!(resumed["revision"], 1);
    assert_eq!(resumed["samples"].as_array().unwrap().len(), 0);

    task.abort();
}

#[tokio::test]
async fn sample_upload_requires_current_revision() {
    let (base, task) = server_with_runtime(&database_url(), true, 2).await;
    let client = Client::new();
    let draft_id = open_owner_draft(&client, &base).await;

    let missing = client
        .put(sample_url(&base, &draft_id, 1))
        .bearer_auth(TOKEN)
        .header("content-type", "audio/wav")
        .body(enrollment_wav(8_000, 8_000))
        .send()
        .await
        .unwrap();
    assert_eq!(missing.status(), StatusCode::BAD_REQUEST);
    assert_eq!(
        missing.json::<Value>().await.unwrap()["error"]["code"],
        "invalid_if_match"
    );

    let stale = put_sample(
        &client,
        &base,
        &draft_id,
        1,
        "9",
        "audio/wav",
        enrollment_wav(8_000, 8_000),
    )
    .await;
    assert_eq!(stale.status(), StatusCode::CONFLICT);
    assert_eq!(
        stale.json::<Value>().await.unwrap()["error"]["code"],
        "revision_conflict"
    );

    task.abort();
}

#[tokio::test]
async fn sample_replace_and_delete_are_revisioned() {
    let (base, task) = server_with_runtime(&database_url(), true, 2).await;
    let client = Client::new();
    let draft_id = open_owner_draft(&client, &base).await;

    let first = put_sample(
        &client,
        &base,
        &draft_id,
        1,
        "1",
        "audio/wav",
        enrollment_wav(8_000, 8_000),
    )
    .await;
    assert_eq!(first.status(), StatusCode::OK);
    assert_eq!(first.headers()["etag"], "\"2\"");

    // Re-recording the same slot replaces it rather than adding a second sample.
    let replaced = put_sample(
        &client,
        &base,
        &draft_id,
        1,
        "2",
        "audio/wav",
        enrollment_wav(6_000, 8_000),
    )
    .await;
    assert_eq!(replaced.status(), StatusCode::OK);
    assert_eq!(replaced.headers()["etag"], "\"3\"");
    let body: Value = replaced.json().await.unwrap();
    assert_eq!(body["samples"].as_array().unwrap().len(), 1);
    assert_eq!(body["samples"][0]["duration_ms"], 6_000);

    let removed = client
        .delete(sample_url(&base, &draft_id, 1))
        .bearer_auth(TOKEN)
        .header("if-match", "\"3\"")
        .send()
        .await
        .unwrap();
    assert_eq!(removed.status(), StatusCode::OK);
    assert_eq!(removed.headers()["etag"], "\"4\"");
    let body: Value = removed.json().await.unwrap();
    assert_eq!(body["samples"].as_array().unwrap().len(), 0);

    let gone = client
        .delete(sample_url(&base, &draft_id, 1))
        .bearer_auth(TOKEN)
        .header("if-match", "\"4\"")
        .send()
        .await
        .unwrap();
    assert_eq!(gone.status(), StatusCode::NOT_FOUND);

    task.abort();
}

#[tokio::test]
async fn sample_upload_needs_a_collecting_draft() {
    let (base, task) = server_with_runtime(&database_url(), true, 2).await;
    let client = Client::new();
    let draft_id = open_owner_draft(&client, &base).await;

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

    let after = put_sample(
        &client,
        &base,
        &draft_id,
        1,
        "1",
        "audio/wav",
        enrollment_wav(8_000, 8_000),
    )
    .await;
    assert_eq!(after.status(), StatusCode::NOT_FOUND);
    assert_eq!(
        after.json::<Value>().await.unwrap()["error"]["code"],
        "enrollment_not_found"
    );

    task.abort();
}

async fn get_draft(client: &Client, base: &str, draft_id: &str) -> Value {
    client
        .get(format!(
            "{base}/api/admin/speakers/owner/enrollments/{draft_id}"
        ))
        .bearer_auth(TOKEN)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap()
}

/// Registers three identical-loudness samples and returns the draft id and its revision.
async fn collecting_draft_with_samples(client: &Client, base: &str) -> (String, u64) {
    let draft_id = open_owner_draft(client, base).await;
    let mut revision = "1".to_owned();
    for slot in 1..=3u32 {
        let response = put_sample(
            client,
            base,
            &draft_id,
            slot,
            &revision,
            "audio/wav",
            enrollment_wav(8_000, 8_000),
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        revision = response.headers()["etag"]
            .to_str()
            .unwrap()
            .trim_matches('"')
            .to_owned();
    }
    (draft_id, revision.parse().unwrap())
}

async fn validate_holdout(
    client: &Client,
    base: &str,
    draft_id: &str,
    revision: u64,
    body: Vec<u8>,
) -> reqwest::Response {
    client
        .post(format!(
            "{base}/api/admin/speakers/owner/enrollments/{draft_id}/validate"
        ))
        .bearer_auth(TOKEN)
        .header("if-match", format!("\"{revision}\""))
        .header("content-type", "audio/wav")
        .body(body)
        .send()
        .await
        .unwrap()
}

async fn finalize_draft(
    client: &Client,
    base: &str,
    draft_id: &str,
    revision: u64,
    expected_speaker_revision: i64,
) -> reqwest::Response {
    client
        .post(format!(
            "{base}/api/admin/speakers/owner/enrollments/{draft_id}/finalize"
        ))
        .bearer_auth(TOKEN)
        .header("if-match", format!("\"{revision}\""))
        .json(&serde_json::json!({"expected_speaker_revision": expected_speaker_revision}))
        .send()
        .await
        .unwrap()
}

#[tokio::test]
async fn holdout_validation_gates_finalize_and_publishes_the_space() {
    let (base, task) = server_with_runtime(&database_url(), true, 2).await;
    let client = Client::new();
    let (draft_id, revision) = collecting_draft_with_samples(&client, &base).await;

    // Finalize before any validation is refused.
    let unvalidated = finalize_draft(&client, &base, &draft_id, revision, 1).await;
    assert_eq!(unvalidated.status(), StatusCode::CONFLICT);
    assert_eq!(
        unvalidated.json::<Value>().await.unwrap()["error"]["code"],
        "speaker_validation_required"
    );

    // A holdout from a different speaker fails and stays HTTP 200 with a decision.
    let failed = validate_holdout(
        &client,
        &base,
        &draft_id,
        revision,
        other_speaker_wav(8_000),
    )
    .await;
    assert_eq!(failed.status(), StatusCode::OK);
    let failed: Value = failed.json().await.unwrap();
    assert_eq!(failed["validation"]["status"], "failed");
    assert_eq!(failed["validation"]["valid_for_current_revision"], false);
    assert_eq!(failed["revision"], revision + 1);

    // A same-speaker holdout that is byte-identical to a registered sample is rejected.
    let duplicate = validate_holdout(
        &client,
        &base,
        &draft_id,
        revision + 1,
        enrollment_wav(8_000, 8_000),
    )
    .await;
    assert_eq!(duplicate.status(), StatusCode::CONFLICT);
    assert_eq!(
        duplicate.json::<Value>().await.unwrap()["error"]["code"],
        "speaker_holdout_duplicate"
    );

    // A distinct same-speaker recording passes.
    let passed = validate_holdout(
        &client,
        &base,
        &draft_id,
        revision + 1,
        same_speaker_holdout(8_000),
    )
    .await;
    assert_eq!(passed.status(), StatusCode::OK);
    let passed: Value = passed.json().await.unwrap();
    assert_eq!(passed["validation"]["status"], "passed");
    assert_eq!(passed["validation"]["valid_for_current_revision"], true);
    let validated_revision = passed["revision"].as_u64().unwrap();

    // Any sample mutation invalidates the stored decision.
    let replaced = put_sample(
        &client,
        &base,
        &draft_id,
        1,
        &validated_revision.to_string(),
        "audio/wav",
        enrollment_wav(7_000, 8_000),
    )
    .await;
    let replaced_status = replaced.status();
    let replaced_etag = replaced
        .headers()
        .get("etag")
        .map(|v| v.to_str().unwrap().to_owned());
    let replaced_body = replaced.text().await.unwrap();
    assert_eq!(replaced_status, StatusCode::OK, "{replaced_body}");
    let bumped = replaced_etag
        .unwrap()
        .trim_matches('"')
        .parse::<u64>()
        .unwrap();
    let invalidated = get_draft(&client, &base, &draft_id).await;
    assert_eq!(invalidated["validation"]["status"], "none");
    assert_eq!(
        invalidated["validation"]["valid_for_current_revision"],
        false
    );

    let blocked = finalize_draft(&client, &base, &draft_id, bumped, 1).await;
    assert_eq!(blocked.status(), StatusCode::CONFLICT);

    // Re-validate, then finalize bumps the catalog and publishes exactly one space.
    let revalidated = validate_holdout(
        &client,
        &base,
        &draft_id,
        bumped,
        same_speaker_holdout(8_000),
    )
    .await;
    assert_eq!(revalidated.status(), StatusCode::OK);
    let revalidated: Value = revalidated.json().await.unwrap();
    let final_revision = revalidated["revision"].as_u64().unwrap();

    let published = finalize_draft(&client, &base, &draft_id, final_revision, 1).await;
    assert_eq!(published.status(), StatusCode::OK);
    let published: Value = published.json().await.unwrap();
    assert_eq!(published["enrollment"]["status"], "committed");
    assert_eq!(published["activation"]["catalog_revision"], 1);
    assert_eq!(published["activation"]["new_connections"], "effective");
    assert_eq!(
        published["activation"]["existing_connections"],
        "reconnect_if_affected"
    );
    let voiceprints = published["speaker"]["voiceprints"].as_array().unwrap();
    assert_eq!(voiceprints.len(), 1);
    assert_eq!(voiceprints[0]["browser_validation_status"], "passed");
    assert_eq!(voiceprints[0]["calibration_revision"], "vi_esp32_pilot_v1");
    assert!(voiceprints[0].get("vector").is_none());

    // The draft is terminal: a second finalize conflicts.
    let again = finalize_draft(&client, &base, &draft_id, final_revision, 2).await;
    assert_eq!(again.status(), StatusCode::CONFLICT);

    // A stale If-Match is refused before anything is written.
    let (fresh_draft, fresh_revision) = collecting_draft_with_samples(&client, &base).await;
    let stale = finalize_draft(&client, &base, &fresh_draft, fresh_revision - 1, 1).await;
    assert_eq!(stale.status(), StatusCode::CONFLICT);
    assert_eq!(
        stale.json::<Value>().await.unwrap()["error"]["code"],
        "revision_conflict"
    );

    // The failed finalize did not advance the published catalog: the next success is revision 2.
    let fresh_validated = validate_holdout(
        &client,
        &base,
        &fresh_draft,
        fresh_revision,
        same_speaker_holdout(8_000),
    )
    .await;
    assert_eq!(fresh_validated.status(), StatusCode::OK);
    let fresh_validated_revision = fresh_validated.json::<Value>().await.unwrap()["revision"]
        .as_u64()
        .unwrap();
    let second = finalize_draft(&client, &base, &fresh_draft, fresh_validated_revision, 1).await;
    assert_eq!(second.status(), StatusCode::OK);
    assert_eq!(
        second.json::<Value>().await.unwrap()["activation"]["catalog_revision"],
        2
    );

    task.abort();
}

#[tokio::test]
async fn validate_requires_the_configured_minimum_sample_count() {
    let (base, task) = server_with_runtime(&database_url(), true, 2).await;
    let client = Client::new();
    let draft_id = open_owner_draft(&client, &base).await;
    let mut revision = "1".to_owned();
    for slot in 1..=2u32 {
        let response = put_sample(
            &client,
            &base,
            &draft_id,
            slot,
            &revision,
            "audio/wav",
            enrollment_wav(8_000, 8_000),
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        revision = response.headers()["etag"]
            .to_str()
            .unwrap()
            .trim_matches('"')
            .to_owned();
    }
    let revision = revision.parse().unwrap();
    let refused = validate_holdout(
        &client,
        &base,
        &draft_id,
        revision,
        same_speaker_holdout(8_000),
    )
    .await;
    assert_eq!(refused.status(), StatusCode::CONFLICT);
    assert_eq!(
        refused.json::<Value>().await.unwrap()["error"]["code"],
        "speaker_insufficient_samples"
    );
    task.abort();
}

#[tokio::test]
async fn finalized_voiceprint_survives_restart_and_catalog_is_monotonic() {
    let database = database_url();
    let (base, task) = server_with_runtime(&database, true, 2).await;
    let client = Client::new();
    let (draft_id, revision) = collecting_draft_with_samples(&client, &base).await;
    let passed = validate_holdout(
        &client,
        &base,
        &draft_id,
        revision,
        same_speaker_holdout(8_000),
    )
    .await;
    assert_eq!(passed.status(), StatusCode::OK);
    let validated_revision = passed.json::<Value>().await.unwrap()["revision"]
        .as_u64()
        .unwrap();
    let published = finalize_draft(&client, &base, &draft_id, validated_revision, 1).await;
    assert_eq!(published.status(), StatusCode::OK);
    assert_eq!(
        published.json::<Value>().await.unwrap()["activation"]["catalog_revision"],
        1
    );
    task.abort();

    // The published voiceprint is durable: a fresh process on the same database sees it.
    let (restarted, task) = server_with_runtime(&database, true, 2).await;
    let speaker: Value = client
        .get(format!("{restarted}/api/admin/speakers/owner"))
        .bearer_auth(TOKEN)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let voiceprints = speaker["voiceprints"].as_array().unwrap();
    assert_eq!(voiceprints.len(), 1);
    assert_eq!(voiceprints[0]["browser_validation_status"], "passed");
    task.abort();
}
