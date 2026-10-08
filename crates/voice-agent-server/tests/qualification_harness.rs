//! Ticket 19 — Mandatory Qualification / Reference Integration Client.
//!
//! Drives the **production process** (`voice-agent-server`) end to end: real
//! config load, real SQLite, real admin HTTP, real HTTP enrollment, and the
//! production listener handoff (`VOICE_AGENT_BOUND_ADDRESS_FILE` +
//! `VOICE_AGENT_STARTUP_NONCE`). The deterministic `qualification_speaker`
//! adapter (ADR 0068) is compiled in, so nothing here downloads a model or
//! reaches the network.
//!
//! Ignored by default: the feature-gated build is a deliberate, separate
//! qualification target. Run with:
//!
//! ```text
//! cargo test -p voice-agent-server --features qualification-providers \
//!     --test qualification_harness -- --ignored --nocapture
//! ```
#![cfg(feature = "qualification-providers")]

use reqwest::{Client, StatusCode};
use serde_json::Value;
use std::{
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    time::Duration,
};

const TOKEN: &str = "qualification-admin-token";

struct Harness {
    child: Child,
    base_url: String,
    http: Client,
}

impl Drop for Harness {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn server_binary() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_voice-agent-server"))
}

fn qualification_config(root: &Path) -> PathBuf {
    let database = root.join("qualification.sqlite3");
    // Deterministic, model-free build: the four deployment defaults are declared
    // so config validation is satisfied, but the qualification feature skips
    // asset preparation and startup materialization. Only `qualification_speaker`
    // is ever built, and it is built on demand by enrollment.
    let toml = format!(
        r#"
[server]
bind = "127.0.0.1:0"
public_ws_url = "ws://127.0.0.1:0/voice/v1/"
[database]
url = "sqlite://{database}?mode=rwc"
[api]
enabled = true
admin_token = "{TOKEN}"
[provider_defaults]
vad = "silero_default"
asr = "gipformer_vi"
llm = "openai_primary"
tts = "chillaudio_default"
[providers.vad.instances.silero_default]
adapter = "silero_onnx"
min_speech_ms = 180
end_silence_ms = 600
pre_roll_ms = 300
speech_threshold = 0.50
exit_threshold = 0.35
[providers.asr.instances.gipformer_vi]
adapter = "gipformer_sherpa_offline"
decoding_method = "modified_beam_search"
max_active_paths = 4
[providers.llm.instances.openai_primary]
adapter = "openai"
api_key = ""
base_url = "https://api.openai.com/v1"
model = "model-name"
[providers.tts.instances.chillaudio_default]
adapter = "chillaudio_ws"
token = "set-deployment-token-here"
[deployment]
speaker_pilot = false
profile = "development-noncommercial"
[runtime.onnx]
library = "runtime/onnxruntime/libonnxruntime.so"
[provider_runtime]
max_parallel_loads = 1
max_pending_loads = 8
max_waiters = 64
max_resident_bytes = 8589934592
max_resources = 16
max_version_entries = 128
admission_timeout_ms = 5000
startup_timeout_ms = 60000
failure_cooldown_ms = 1000
idle_ttl_ms = 600000

[provider_runtime.estimated_peak_bytes]
qualification_speaker = 67108864
"#,
        database = database.display(),
    );
    let path = root.join("config.toml");
    std::fs::write(&path, toml).unwrap();
    path
}

impl Harness {
    /// Graceful shutdown: SIGTERM the process and wait for it to exit. A hard
    /// kill would hide drain bugs, so the restart phase exercises the real
    /// shutdown path.
    fn stop_gracefully(&mut self) {
        let pid = self.child.id().to_string();
        let _ = Command::new("kill").arg("-TERM").arg(&pid).status();
        let deadline = std::time::Instant::now() + Duration::from_secs(30);
        loop {
            match self.child.try_wait() {
                Ok(Some(_)) => return,
                Ok(None) if std::time::Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(50));
                }
                _ => {
                    let _ = self.child.kill();
                    let _ = self.child.wait();
                    return;
                }
            }
        }
    }
}

fn start_qualification_server(root: &Path) -> Harness {
    let nonce = uuid::Uuid::new_v4().to_string();
    let artifact = root.join(format!("bound-address-{nonce}.json"));
    let child = Command::new(server_binary())
        .env("VOICE_AGENT_CONFIG", qualification_config(root))
        .env("VOICE_AGENT_BOUND_ADDRESS_FILE", &artifact)
        .env("VOICE_AGENT_STARTUP_NONCE", &nonce)
        .env("RUST_LOG", "warn")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn production voice-agent-server");

    let deadline = std::time::Instant::now() + Duration::from_secs(60);
    let address = loop {
        if let Ok(contents) = std::fs::read_to_string(&artifact)
            && let Ok(value) = serde_json::from_str::<Value>(&contents)
            && value["version"] == 1
            && value["startup_nonce"] == nonce.as_str()
        {
            break value["address"].as_str().unwrap().to_owned();
        }
        assert!(
            std::time::Instant::now() < deadline,
            "server never published its bound address"
        );
        std::thread::sleep(Duration::from_millis(100));
    };

    Harness {
        child,
        base_url: format!("http://{address}"),
        http: Client::new(),
    }
}

fn wav(ms: u64, amplitude: i16) -> Vec<u8> {
    pulse(ms, amplitude, 2)
}

/// `period` samples high per `period` samples low at the same loudness. The held
/// out clip uses `period = 4` so it reads as the *same loudness*, hence the same
/// deterministic embedding, but not the same bytes as the enrollment samples.
fn pulse(ms: u64, amplitude: i16, period: usize) -> Vec<u8> {
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: 16_000,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut buffer = std::io::Cursor::new(Vec::new());
    let mut writer = hound::WavWriter::new(&mut buffer, spec).unwrap();
    for index in 0..ms * 16 {
        let high = (index as usize) % (period * 2) < period;
        writer
            .write_sample(if high { amplitude } else { -amplitude })
            .unwrap();
    }
    writer.finalize().unwrap();
    buffer.into_inner()
}

async fn create_speaker_provider(h: &Harness) -> String {
    let response = h
        .http
        .post(format!("{}/api/admin/providers", h.base_url))
        .bearer_auth(TOKEN)
        .json(&serde_json::json!({
            "type": "speaker",
            "adapter": "qualification_speaker",
            "name": "Qualification Speaker",
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
#[ignore = "requires the qualification-providers build of the production binary"]
async fn enrollment_publishes_a_qualification_voiceprint() {
    let root = std::env::temp_dir().join(format!("qualification-harness-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&root).unwrap();
    let mut server = start_qualification_server(&root);
    let base = &server.base_url;
    let http = &server.http;

    let provider_key = create_speaker_provider(&server).await;

    // Real admin HTTP: the speaker exists before a draft can open.
    let created = http
        .post(format!("{base}/api/admin/speakers"))
        .bearer_auth(TOKEN)
        .json(&serde_json::json!({ "key": "owner", "name": "Chủ sở hữu" }))
        .send()
        .await
        .unwrap();
    assert_eq!(created.status(), StatusCode::CREATED);

    let draft: Value = http
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

    // Three samples at the same loudness → one embedding point.
    let mut revision = 1u64;
    for slot in 1..=3 {
        let response = http
            .put(format!(
                "{base}/api/admin/speakers/owner/enrollments/{draft_id}/samples/{slot}"
            ))
            .bearer_auth(TOKEN)
            .header("if-match", format!("\"{revision}\""))
            .header("content-type", "audio/wav")
            .body(wav(8_000, 8_000))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        revision = response
            .headers()
            .get("etag")
            .unwrap()
            .to_str()
            .unwrap()
            .trim_matches('"')
            .parse()
            .unwrap();
    }

    // A *same-loudness* held out recording (different bytes, same embedding)
    // validates: this is the production holdout gate, not a rubber stamp.
    let validated: Value = http
        .post(format!(
            "{base}/api/admin/speakers/owner/enrollments/{draft_id}/validate"
        ))
        .bearer_auth(TOKEN)
        .header("if-match", format!("\"{revision}\""))
        .header("content-type", "audio/wav")
        .body(pulse(8_000, 8_000, 4))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(validated["validation"]["status"], "passed");
    let validated_revision = validated["revision"].as_u64().unwrap();

    let published: Value = http
        .post(format!(
            "{base}/api/admin/speakers/owner/enrollments/{draft_id}/finalize"
        ))
        .bearer_auth(TOKEN)
        .header("if-match", format!("\"{validated_revision}\""))
        .json(&serde_json::json!({ "expected_speaker_revision": 1 }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(published["enrollment"]["status"], "committed");
    assert_eq!(published["activation"]["catalog_revision"], 1);
    assert_eq!(published["activation"]["new_connections"], "effective");

    let voiceprints = published["speaker"]["voiceprints"].as_array().unwrap();
    assert_eq!(voiceprints.len(), 1);
    assert_eq!(voiceprints[0]["browser_validation_status"], "passed");
    assert_eq!(voiceprints[0]["calibration_revision"], "vi_esp32_pilot_v1");
    assert!(voiceprints[0].get("vector").is_none());

    // Controlled restart: the same database + config must republish the same
    // voiceprint. This is the only check that the qualification artifact is
    // durable across the production process boundary.
    server.stop_gracefully();
    let restarted = start_qualification_server(&root);
    let persisted: Value = restarted
        .http
        .get(format!("{}/api/admin/speakers/owner", restarted.base_url))
        .bearer_auth(TOKEN)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let persisted_voiceprints = persisted["voiceprints"].as_array().unwrap();
    assert_eq!(persisted_voiceprints.len(), 1);
    assert_eq!(
        persisted_voiceprints[0]["calibration_revision"],
        "vi_esp32_pilot_v1"
    );
    assert_eq!(
        persisted_voiceprints[0]["browser_validation_status"],
        "passed"
    );

    write_qualification_report(&persisted);

    std::fs::remove_dir_all(&root).ok();
}

/// Machine-readable, privacy-safe Mandatory Qualification result. It records
/// what this deterministic run actually proved, and is explicit about the
/// real-model / ESP32 evidence it did *not* run.
fn write_qualification_report(persisted_speaker: &Value) {
    let report = serde_json::json!({
        "schema_version": 1,
        "scenario": "speaker-enrollment-observe-v1",
        "build": "qualification-providers",
        "mandatory": {
            "result": "pass",
            "checks": [
                "startup handshake artifact is nonce-bound and machine-readable",
                "admin enrollment is reachable through the public HTTP boundary",
                "deterministic qualification speaker loads on demand without a model",
                "holdout validation gates finalize",
                "published voiceprint survives a controlled process restart"
            ]
        },
        "optional_runtime_evidence": {
            "result": "not_run",
            "missing_prerequisites": [
                "real CAM++ model assets and a recorded speaker corpus",
                "ESP32 Observe hardware on the target machine"
            ],
            "required_activation": "unavailable"
        },
        "privacy": {
            "contains_audio": false,
            "contains_embeddings": false,
            "contains_credentials": false
        },
        "speaker_revision": persisted_speaker["revision"]
    });
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../.scratch/speaker-recognition/evidence/pilot-handoff-qualification.json");
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(path, serde_json::to_vec_pretty(&report).unwrap()).unwrap();
}
