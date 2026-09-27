#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cleans_text_like_the_javascript_reference() {
        assert_eq!(clean_text("  Xin\n chào\t Rust  "), "Xin chào Rust");
    }

    #[test]
    fn start_task_payload_is_a_json_string() {
        let client = ChillAudioClient::new(ChillAudioConfig::default());
        let task = client.build_start_task("Xin chào").unwrap();
        let value = serde_json::to_value(task).unwrap();

        let payload = value["payload"].as_str().expect("payload must be a string");
        let payload: serde_json::Value = serde_json::from_str(payload).unwrap();
        assert_eq!(payload["speaker"], DEFAULT_VOICE);
        assert_eq!(payload["text"], "Xin chào");
        assert_eq!(payload["audio_config"]["format"], DEFAULT_FORMAT);
        assert_eq!(payload["audio_config"]["sample_rate"], DEFAULT_SAMPLE_RATE);
        assert_eq!(payload["audio_config"]["bit_rate"], DEFAULT_BIT_RATE);
    }

    #[test]
    fn parses_terminal_control_events() {
        assert_eq!(
            parse_control_event(r#"{"event":"TaskEnd"}"#).unwrap(),
            ControlEvent::Complete
        );
        assert_eq!(
            parse_control_event(r#"{"event":"TaskFinished"}"#).unwrap(),
            ControlEvent::Complete
        );
        assert_eq!(
            parse_control_event(r#"{"event":"TaskFailed"}"#).unwrap(),
            ControlEvent::Failed
        );
        assert_eq!(
            parse_control_event(r#"{"event":"Progress"}"#).unwrap(),
            ControlEvent::Ignore
        );
    }

    #[test]
    fn rejects_malformed_control_json() {
        assert!(parse_control_event("not json").is_err());
    }
}
// Direct ChillAudio WebSocket TTS protocol client used for provider qualification.

use anyhow::{Context, ensure};
use futures_util::{SinkExt, StreamExt};
use serde::Serialize;
use std::time::Duration;
use tokio::time::{Instant, timeout};
use tokio_tungstenite::{connect_async, tungstenite::Message};

const DEFAULT_WS_URL: &str = "wss://sami-normal-sg.capcutapi.com/internal/api/v1/ws?device_id=7486429558272460289&iid=7486431924195657473&app_id=359289&region=VN&update_version_code=5.7.1.2101&version_code=5.7.1&appKey=ddjeqjLGMn&device_type=macos&device_platform=macos";
const DEFAULT_APP_KEY: &str = "ddjeqjLGMn";
const DEFAULT_TOKEN: &str = "WTV6R2t6V3ZwNUIwQkFETutGxuveRZ9iTmOBC/a3wzMS7zzza86Ky9nIfYhyeoSiWYP1ZO04X7X1+RThg/zczU6u8ga3dTIJpduvWpCqrmr0Kv7BJf6tcGFgevJ/Jaa1slHj/l4NUJ/eCesl1dYBYQ51oKbuFnZjF7qXVWzsoz326XwRdNEmOufSHnuW+kuy+sS7K/sn3gVWsCC4XFi+FYntDxrVTYS/Pv2LtBgpgULmib5+5kMq2ZuJfCDYvq4NthciciB6KUCf1sOsu7VD/27Tquz8Q58NYALFvX85bjvxQJOz0iV3oUiip0RyqR1ltZPNI/LgN2OGCphyCgOJdlUUdgIbSJpaKL+5PMTM4yBuwCU4QPbYYzTs9x2ZA+7zt41ng+i5+EPtePyDjR4VFTz+7zglLw/E+KqN/nscyqLCyrumn4YgfQ3JYnSnz1WLE6q3aD175yweKBj9f9jyqxnLVmEYy9VjmoxuYNRgVmfT6M17bT9iL0PJTlJ6UqKHuNRT6ubv37ZSr961Gw+RJhyLUDBt8AD1B8YDdF4OImS+LgGjfujaY1agc4tfrnk4V4YcAXyTRlYwLMC9ATDp9CbiBrlMBmYm88gwGaTR9pbI2KcQ4Kg86jZYc6CxNM34sbMG/1LlmqvqLe+E3IG6ebOmyVbL+kYK70c1fT5TcmzVwX5O3JGkHHtFoeCmd4Eyyov6QsO1Jewx0gpjp05dqw==";
const DEFAULT_VOICE: &str = "BV421_vivn_streaming";
const DEFAULT_SAMPLE_RATE: u32 = 24_000;
const DEFAULT_BIT_RATE: u32 = 128_000;
const DEFAULT_FORMAT: &str = "mp3";
const DEFAULT_TIMEOUT_MS: u64 = 12_000;

pub const CHILLAUDIO_VOICES: &[&str] = &[
    "BV421_vivn_streaming",
    "vi_female_huong",
    "BV074_streaming",
    "BV075_streaming",
];

#[derive(Debug, Clone)]
pub struct ChillAudioConfig {
    pub ws_url: String,
    pub app_key: String,
    pub token: String,
    pub voice: String,
    pub sample_rate: u32,
    pub bit_rate: u32,
    pub format: String,
    pub timeout: Duration,
}

impl Default for ChillAudioConfig {
    fn default() -> Self {
        Self {
            ws_url: DEFAULT_WS_URL.to_owned(),
            app_key: DEFAULT_APP_KEY.to_owned(),
            token: DEFAULT_TOKEN.to_owned(),
            voice: DEFAULT_VOICE.to_owned(),
            sample_rate: DEFAULT_SAMPLE_RATE,
            bit_rate: DEFAULT_BIT_RATE,
            format: DEFAULT_FORMAT.to_owned(),
            timeout: Duration::from_millis(DEFAULT_TIMEOUT_MS),
        }
    }
}

#[derive(Debug)]
pub struct ChillAudioResult {
    pub audio: Vec<u8>,
    pub binary_chunks: usize,
    pub first_audio_ms: Option<f64>,
    pub total_ms: f64,
}

pub struct ChillAudioClient {
    config: ChillAudioConfig,
}

#[derive(Serialize)]
struct AudioConfig {
    bit_rate: u32,
    format: String,
    sample_rate: u32,
}

#[derive(Serialize)]
struct StartPayload {
    audio_config: AudioConfig,
    speaker: String,
    text: String,
}

#[derive(Serialize)]
struct StartTask {
    appkey: String,
    event: &'static str,
    namespace: &'static str,
    payload: String,
    token: String,
    version: &'static str,
}

#[derive(Debug, PartialEq, Eq)]
enum ControlEvent {
    Complete,
    Failed,
    Ignore,
}

impl ChillAudioClient {
    pub fn new(config: ChillAudioConfig) -> Self {
        Self { config }
    }

    pub async fn synthesize(&self, text: &str) -> anyhow::Result<ChillAudioResult> {
        let text = clean_text(text);
        ensure!(!text.is_empty(), "invalid input: TTS text is empty");

        timeout(self.config.timeout, self.synthesize_inner(&text))
            .await
            .map_err(|_| anyhow::anyhow!("ChillAudio TTS timeout"))?
    }

    fn build_start_task(&self, text: &str) -> anyhow::Result<StartTask> {
        let payload = StartPayload {
            audio_config: AudioConfig {
                bit_rate: self.config.bit_rate,
                format: self.config.format.clone(),
                sample_rate: self.config.sample_rate,
            },
            speaker: self.config.voice.clone(),
            text: text.to_owned(),
        };

        Ok(StartTask {
            appkey: self.config.app_key.clone(),
            event: "StartTask",
            namespace: "TTS",
            payload: serde_json::to_string(&payload)
                .context("ChillAudio request serialization failed")?,
            token: self.config.token.clone(),
            version: "sdk_v1",
        })
    }

    async fn synthesize_inner(&self, text: &str) -> anyhow::Result<ChillAudioResult> {
        let started = Instant::now();
        let (mut websocket, _) = connect_async(self.config.ws_url.as_str())
            .await
            .context("ChillAudio websocket connection failed")?;
        let task = self.build_start_task(text)?;
        websocket
            .send(Message::Text(
                serde_json::to_string(&task)
                    .context("ChillAudio request serialization failed")?
                    .into(),
            ))
            .await
            .context("ChillAudio websocket send failed")?;

        let mut audio = Vec::new();
        let mut binary_chunks = 0;
        let mut first_audio_ms = None;

        loop {
            let message = websocket
                .next()
                .await
                .transpose()
                .context("ChillAudio websocket protocol failure")?
                .ok_or_else(|| {
                    anyhow::anyhow!("ChillAudio websocket closed before terminal event")
                })?;
            match message {
                Message::Binary(bytes) => {
                    if first_audio_ms.is_none() {
                        first_audio_ms = Some(started.elapsed().as_secs_f64() * 1000.0);
                    }
                    binary_chunks += 1;
                    audio.extend_from_slice(&bytes);
                }
                Message::Text(text) => match parse_control_event(text.as_ref())? {
                    ControlEvent::Complete => break,
                    ControlEvent::Failed => anyhow::bail!("ChillAudio TTS task failed"),
                    ControlEvent::Ignore => {}
                },
                Message::Ping(payload) => websocket
                    .send(Message::Pong(payload))
                    .await
                    .context("ChillAudio websocket pong failed")?,
                Message::Close(_) => {
                    anyhow::bail!("ChillAudio websocket closed before terminal event")
                }
                _ => {}
            }
        }

        ensure!(audio.len() > 100, "ChillAudio returned empty audio");
        Ok(ChillAudioResult {
            audio,
            binary_chunks,
            first_audio_ms,
            total_ms: started.elapsed().as_secs_f64() * 1000.0,
        })
    }
}

fn clean_text(input: &str) -> String {
    input.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn parse_control_event(text: &str) -> anyhow::Result<ControlEvent> {
    let value: serde_json::Value = serde_json::from_str(text)
        .context("ChillAudio websocket protocol failure: invalid control JSON")?;
    Ok(
        match value.get("event").and_then(serde_json::Value::as_str) {
            Some("TaskEnd" | "TaskFinished") => ControlEvent::Complete,
            Some("TaskFailed") => ControlEvent::Failed,
            _ => ControlEvent::Ignore,
        },
    )
}
