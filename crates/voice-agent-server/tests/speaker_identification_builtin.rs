//! End-to-end Speaker enrollment with the in-process CAM++ interface and SQLite.
//! No Provider record, Template grant, calibration, or voice authentication required.
use std::{fs, sync::Arc};
use reqwest::{Client, StatusCode};
use serde_json::Value;
use voice_agent_server::{
    app::{AppState, router_with_state},
    audio::PcmF32Mono,
    config::AppConfig,
    database::Database,
    providers::{ProviderSet, speaker::{SpeakerError, SpeakerProvider, SpeakerRuntime}},
    workers::ProviderRuntimeAdmission,
};

const TOKEN: &str = "admin-speaker-test-token";

struct TestEmbedding;
impl SpeakerProvider for TestEmbedding {
    fn dimension(&self) -> usize { 3 }
    fn extract(&mut self, _audio: &PcmF32Mono) -> Result<Vec<f32>, SpeakerError> {
        Ok(vec![1.0, 0.0, 0.0])
    }
}

fn wav() -> Vec<u8> {
    // 5 seconds at 16kHz, mono PCM16 — no raw sample is persisted after capture.
    let len = 16_000 * 5;
    let data_bytes = (len * 2) as u32;
    let mut out = Vec::with_capacity(44 + len * 2);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data_bytes).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&16_000u32.to_le_bytes());
    out.extend_from_slice(&32_000u32.to_le_bytes());
    out.extend_from_slice(&2u16.to_le_bytes());
    out.extend_from_slice(&16u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_bytes.to_le_bytes());
    for i in 0..len {
        let sample = ((i as f32 * 0.07).sin() * 15000.0) as i16;
        out.extend_from_slice(&sample.to_le_bytes());
    }
    out
}

async fn harness() -> (String, tokio::task::JoinHandle<()>) {
    let db_path = std::env::temp_dir().join(format!("speaker-builtin-{}.db", uuid::Uuid::new_v4()));
    let path = std::env::temp_dir().join(format!("speaker-builtin-{}.toml", uuid::Uuid::new_v4()));
    let config_file = format!(r#"
[server]
bind = "127.0.0.1:0"
public_ws_url = "ws://127.0.0.1:0/voice/v1/"
[database]
url = "sqlite://{}"
[provider_defaults]
vad = "test"
asr = "test"
llm = "test"
tts = "test"
[api]
enabled = true
admin_token = "{TOKEN}"
[speaker_recognition.enrollment]
min_clip_ms = 2000
min_speech_ms = 1800
max_clip_ms = 10000
max_window_ms = 6000
"#, db_path.display());
    fs::write(&path, config_file).unwrap();
    let config = AppConfig::parse_and_resolve(&path).unwrap();
    fs::remove_file(&path).unwrap();
    let db = Database::connect(&config.database).await.unwrap();
    let mut state = AppState::from_provider_set_with_database(
        config, Arc::new(ProviderSet::unavailable()), Some(db),
    );
    state.speaker_runtime = Some(Arc::new(
        SpeakerRuntime::new(Box::new(TestEmbedding), ProviderRuntimeAdmission::new(1, 1)).unwrap(),
    ));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let handle = tokio::spawn(async move {
        axum::serve(listener, router_with_state(state)).await.unwrap();
    });
    (base, handle)
}

#[tokio::test]
async fn single_capture_registers_and_reenrolls_without_a_provider() {
    let (base, task) = harness().await;
    let client = Client::new();
    let summary: Value = client.get(format!("{base}/api/admin/speaker-recognition"))
        .bearer_auth(TOKEN).send().await.unwrap().json().await.unwrap();
    assert_eq!(summary["available"], true);
    assert!(summary.get("providers").is_none());

    let capture_result = client
        .post(format!("{base}/api/admin/speakers/captures"))
        .bearer_auth(TOKEN).header("content-type", "audio/wav")
        .body(wav()).send().await.unwrap();
    assert_eq!(capture_result.status(), StatusCode::CREATED);
    let captured: Value = capture_result.json().await.unwrap();
    let capture_id = captured["capture_id"].as_str().unwrap();

    let created = client
        .post(format!("{base}/api/admin/speakers/from-capture"))
        .bearer_auth(TOKEN)
        .json(&serde_json::json!({"capture_id":capture_id,"name":"Owner"}))
        .send().await.unwrap();
    assert_eq!(created.status(), StatusCode::CREATED);
    let created: Value = created.json().await.unwrap();
    let key = created["speaker"]["key"].as_str().unwrap();
    assert_eq!(created["speaker"]["voiceprints"].as_array().unwrap().len(), 1);

    let duplicate = client
        .post(format!("{base}/api/admin/speakers/from-capture"))
        .bearer_auth(TOKEN)
        .json(&serde_json::json!({"capture_id":capture_id,"name":"Owner"}))
        .send().await.unwrap();
    assert_eq!(duplicate.status(), StatusCode::OK);

    let agent = client.post(format!("{base}/api/admin/agents"))
        .bearer_auth(TOKEN)
        .json(&serde_json::json!({"key":"home","name":"Home"}))
        .send().await.unwrap();
    assert_eq!(agent.status(), StatusCode::CREATED);

    let bound = client.put(format!("{base}/api/admin/agents/home/speakers/{key}"))
        .bearer_auth(TOKEN).header("if-match", "\"1\"")
        .json(&serde_json::json!({}))
        .send().await.unwrap();
    assert_eq!(bound.status(), StatusCode::OK);

    let policy = client.put(format!("{base}/api/admin/agents/home/speaker-policy"))
        .bearer_auth(TOKEN).header("if-match", "\"1\"")
        .json(&serde_json::json!({"mode":"observe"}))
        .send().await.unwrap();
    assert_eq!(policy.status(), StatusCode::OK);

    let agents_speakers: Value = client.get(format!("{base}/api/admin/agents/home/speakers"))
        .bearer_auth(TOKEN).send().await.unwrap().json().await.unwrap();
    assert_eq!(agents_speakers["items"][0]["usable"], true);

    // Re-enroll is an explicit user action, not a repeated verification.
    let second = client.post(format!("{base}/api/admin/speakers/captures"))
        .bearer_auth(TOKEN).header("content-type", "audio/wav")
        .body(wav()).send().await.unwrap();
    assert_eq!(second.status(), StatusCode::CREATED);
    let second_capture: Value = second.json().await.unwrap();
    let revision = created["speaker"]["revision"].as_i64().unwrap();
    let updated = client.put(format!("{base}/api/admin/speakers/{key}/voiceprint"))
        .bearer_auth(TOKEN).header("if-match", format!("\"{revision}\""))
        .json(&serde_json::json!({"capture_id": second_capture["capture_id"]}))
        .send().await.unwrap();
    assert_eq!(updated.status(), StatusCode::OK);
    task.abort();
}

#[tokio::test]
async fn required_authentication_and_template_provider_api_are_not_available() {
    let (base, task) = harness().await;
    let client = Client::new();
    client.post(format!("{base}/api/admin/agents"))
        .bearer_auth(TOKEN).json(&serde_json::json!({"key":"home","name":"Home"}))
        .send().await.unwrap();
    let required = client.put(format!("{base}/api/admin/agents/home/speaker-policy"))
        .bearer_auth(TOKEN).header("if-match", "\"1\"")
        .json(&serde_json::json!({"mode":"required"}))
        .send().await.unwrap();
    assert_eq!(required.status(), StatusCode::BAD_REQUEST);
    let provider = client.post(format!("{base}/api/admin/providers"))
        .bearer_auth(TOKEN)
        .json(&serde_json::json!({
            "name":"Unwanted speaker provider","type":"speaker",
            "adapter":"campplus_sherpa","config_json":{}
        }))
        .send().await.unwrap();
    assert_eq!(provider.status(), StatusCode::BAD_REQUEST);
    let reload = client.post(format!("{base}/api/admin/speaker-recognition/reload"))
        .bearer_auth(TOKEN).send().await.unwrap();
    assert_eq!(reload.status(), StatusCode::NOT_FOUND);
    task.abort();
}
