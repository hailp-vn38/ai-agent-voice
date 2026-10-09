#![cfg(feature = "qualification-providers")]
use std::{collections::HashMap, sync::Arc};
use voice_agent_server::{
    app::{AppState, router_with_state},
    config::AppConfig,
    database::{Database, secrets::EnvSecretResolver},
    lifecycle::RuntimeLifecycle,
    providers::ProviderSet,
    services::provider_runtime::{FactoryMaterializer, ProviderRuntimeManager, RuntimeLimits},
};

#[tokio::test]
async fn draft_inference_uses_real_factories_without_saved_rows_or_readiness_changes() {
    let path = std::env::temp_dir().join(format!("draft-tests-{}.toml", uuid::Uuid::new_v4()));
    std::fs::write(
        &path,
        format!(
            r#"
[server]
bind = "127.0.0.1:0"
public_ws_url = "ws://127.0.0.1:0/voice/v1/"
[provider_defaults]
vad = "test"
asr = "test"
llm = "test"
tts = "test"
[database]
url = "sqlite://{}"
[api]
enabled = true
admin_token = "draft-test-token"
"#,
            path.with_extension("db").display()
        ),
    )
    .unwrap();
    let config = AppConfig::parse_and_resolve(&path).unwrap();
    std::fs::remove_file(&path).unwrap();
    let db = Database::connect(&config.database).await.unwrap();
    let secrets = Arc::new(EnvSecretResolver);
    let lifecycle = RuntimeLifecycle::new(std::time::Duration::from_secs(1));
    let mut state = AppState::from_provider_set_with_database_resolver_and_shutdown(
        config.clone(),
        Arc::new(ProviderSet::unavailable()),
        Some(db),
        secrets.clone(),
        lifecycle.clone(),
    );
    let builder = FactoryMaterializer::new(
        Arc::new(config),
        secrets,
        HashMap::from([
            ("qualification_llm".into(), 1),
            ("qualification_tts".into(), 1),
            ("qualification_asr".into(), 1),
        ]),
        state.worker_supervisor.clone(),
    )
    .unwrap();
    let manager = ProviderRuntimeManager::new(
        RuntimeLimits {
            max_parallel_loads: 1,
            max_pending_loads: 2,
            max_waiters: 8,
            max_resident_bytes: 10,
            max_resources: 8,
            max_version_entries: 16,
            admission_timeout_ms: 5000,
            failure_cooldown_ms: 1,
            idle_ttl_ms: 1,
        },
        Arc::new(builder),
        lifecycle.gate().clone(),
    )
    .unwrap();
    state = state.with_runtime_manager(manager.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}/api/admin", listener.local_addr().unwrap());
    let task = tokio::spawn(async move {
        axum::serve(listener, router_with_state(state))
            .await
            .unwrap()
    });
    let client = reqwest::Client::new();
    let llm = client.post(format!("{base}/provider-tests/llm")).bearer_auth("draft-test-token").json(&serde_json::json!({"provider":{"type":"llm","adapter":"qualification_llm","config_json":{}},"input":{"text":"hello"}})).send().await.unwrap();
    assert_eq!(llm.status(), 200);
    let result: serde_json::Value = llm.json().await.unwrap();
    assert_eq!(result["result"]["text"], "qualification response");
    assert!(result.get("provider_key").is_none());
    assert_eq!(result["runtime"]["persisted_runtime_modified"], false);
    let tts = client.post(format!("{base}/provider-tests/tts")).bearer_auth("draft-test-token").json(&serde_json::json!({"provider":{"type":"tts","adapter":"qualification_tts","config_json":{}},"input":{"text":"hello"}})).send().await.unwrap();
    assert_eq!(tts.status(), 200);
    assert_eq!(tts.headers()["content-type"], "audio/wav");
    let wav = tts.bytes().await.unwrap();
    assert!(hound::WavReader::new(std::io::Cursor::new(&wav)).is_ok());
    let mut audio = std::io::Cursor::new(Vec::new());
    {
        let mut writer = hound::WavWriter::new(
            &mut audio,
            hound::WavSpec {
                channels: 1,
                sample_rate: 16000,
                bits_per_sample: 16,
                sample_format: hound::SampleFormat::Int,
            },
        )
        .unwrap();
        for _ in 0..1600 {
            writer.write_sample(100_i16).unwrap();
        }
        writer.finalize().unwrap();
    }
    let form = reqwest::multipart::Form::new()
        .text(
            "provider",
            r#"{"type":"asr","adapter":"qualification_asr","config_json":{}}"#,
        )
        .part(
            "audio",
            reqwest::multipart::Part::bytes(audio.into_inner())
                .mime_str("audio/wav")
                .unwrap(),
        );
    let asr = client
        .post(format!("{base}/provider-tests/asr"))
        .bearer_auth("draft-test-token")
        .multipart(form)
        .send()
        .await
        .unwrap();
    assert_eq!(asr.status(), 200);
    let result: serde_json::Value = asr.json().await.unwrap();
    assert_eq!(result["result"]["text"], "qualification transcript");
    assert_eq!(result["metrics"]["audio_duration_ms"], 100);
    let providers: serde_json::Value = client
        .get(format!("{base}/providers"))
        .bearer_auth("draft-test-token")
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(providers["items"].as_array().unwrap().is_empty());
    assert_eq!(manager.accounting().active_leases, 0);
    task.abort();
}
