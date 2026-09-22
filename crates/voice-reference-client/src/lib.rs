//! Reusable wire checks owned by the independent Voice Protocol Client.

use anyhow::{Context, bail, ensure};
use futures_util::{SinkExt, StreamExt};
use opus2::{Channels, Decoder};
use serde::Deserialize;
use serde_json::json;
use std::{path::PathBuf, time::Duration};
use tokio::time::{Instant, timeout_at};
use tokio_tungstenite::{
    connect_async,
    tungstenite::{Message, client::IntoClientRequest},
};

const MAX_DETECT_TEXT_SCALARS: usize = 4_096;

#[derive(Clone, Debug)]
pub struct TextTurnConfig {
    pub tts_start_timeout: Duration,
    pub turn_timeout: Duration,
    pub post_stop_quiet_period: Duration,
    pub debug_audio_file: Option<PathBuf>,
    pub debug_steps: bool,
}

impl Default for TextTurnConfig {
    fn default() -> Self {
        Self {
            tts_start_timeout: Duration::from_secs(60),
            turn_timeout: Duration::from_secs(120),
            post_stop_quiet_period: Duration::from_millis(250),
            debug_audio_file: None,
            debug_steps: false,
        }
    }
}

impl TextTurnConfig {
    pub fn validate(&self) -> anyhow::Result<()> {
        ensure!(
            !self.tts_start_timeout.is_zero(),
            "tts start timeout must be greater than zero"
        );
        ensure!(
            !self.turn_timeout.is_zero(),
            "turn timeout must be greater than zero"
        );
        ensure!(
            self.turn_timeout >= self.tts_start_timeout,
            "turn timeout must not be shorter than tts start timeout"
        );
        ensure!(
            !self.post_stop_quiet_period.is_zero(),
            "post-stop quiet period must be greater than zero"
        );
        Ok(())
    }
}

#[derive(Clone, Debug)]
pub struct TextTurnRequest {
    pub ota_url: String,
    pub device_id: String,
    pub client_id: String,
    pub text: String,
    pub config: TextTurnConfig,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TextTurnReport {
    pub binary_packets: usize,
    pub debug_audio_file: Option<PathBuf>,
}

#[derive(Deserialize)]
struct ServerHello {
    #[serde(rename = "type")]
    message_type: String,
    transport: String,
    session_id: String,
    audio_params: ServerAudioParams,
}
#[derive(Deserialize)]
struct ServerAudioParams {
    format: String,
    sample_rate: u32,
    channels: u8,
    frame_duration: u16,
}

pub fn normalize_text_input(input: &str) -> anyhow::Result<String> {
    let text = input.trim();
    if text.is_empty() {
        bail!("text input is empty");
    }
    if text.chars().take(MAX_DETECT_TEXT_SCALARS + 1).count() > MAX_DETECT_TEXT_SCALARS {
        bail!("text input exceeds maximum length");
    }
    Ok(text.to_owned())
}

/// Runs a complete V1 text turn through OTA and WebSocket, validating the downlink lifecycle.
pub async fn run_text_turn(request: TextTurnRequest) -> anyhow::Result<TextTurnReport> {
    request.config.validate()?;
    let text = normalize_text_input(&request.text)?;
    debug_step(request.config.debug_steps, "ota_request");
    let ota: serde_json::Value = reqwest::Client::new()
        .post(&request.ota_url)
        .json(&json!({}))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    let websocket = ota
        .get("websocket")
        .context("OTA response lacks websocket")?;
    let url = websocket
        .get("url")
        .and_then(|value| value.as_str())
        .context("OTA response lacks websocket.url")?;
    let token = websocket
        .get("token")
        .and_then(|value| value.as_str())
        .unwrap_or_default();
    let mut connection = url.into_client_request()?;
    let headers = connection.headers_mut();
    headers.insert("Protocol-Version", "1".parse()?);
    headers.insert("Device-Id", request.device_id.parse()?);
    headers.insert("Client-Id", request.client_id.parse()?);
    if !token.is_empty() {
        headers.insert("Authorization", format!("Bearer {token}").parse()?);
    }
    let (mut socket, _) = connect_async(connection)
        .await
        .context("connect WebSocket")?;
    debug_step(request.config.debug_steps, "ws_connected");
    socket.send(Message::Text(json!({"type":"hello","version":1,"transport":"websocket","audio_params":{"format":"opus","sample_rate":16000,"channels":1,"frame_duration":60}}).to_string().into())).await?;
    debug_step(request.config.debug_steps, "ws_out=client_hello");
    let hello = match socket.next().await {
        Some(Ok(Message::Text(text))) => {
            serde_json::from_str::<ServerHello>(&text).context("invalid ServerHello")?
        }
        Some(Ok(_)) => bail!("invalid ServerHello: expected text frame"),
        Some(Err(error)) => return Err(error.into()),
        None => bail!("WebSocket closed before ServerHello"),
    };
    validate_server_hello(&hello)?;
    debug_step(request.config.debug_steps, "ws_in=server_hello");
    let session_id = hello.session_id;
    socket
        .send(Message::Text(
            json!({"session_id":session_id,"type":"listen","state":"start","mode":"manual"})
                .to_string()
                .into(),
        ))
        .await?;
    debug_step(request.config.debug_steps, "ws_out=listen_start");
    socket
        .send(Message::Text(
            json!({"session_id":session_id,"type":"listen","state":"detect","text":text})
                .to_string()
                .into(),
        ))
        .await?;
    debug_step(request.config.debug_steps, "ws_out=listen_detect");
    let started = Instant::now();
    let start_deadline = started + request.config.tts_start_timeout;
    let turn_deadline = started + request.config.turn_timeout;
    let mut started_tts = false;
    let mut packets = 0;
    let mut decoded_audio = Vec::new();
    let mut decoder =
        Decoder::new(24_000, Channels::Mono).context("create canonical downlink Opus decoder")?;
    loop {
        let deadline = if started_tts {
            turn_deadline
        } else {
            start_deadline
        };
        let message = timeout_at(deadline, socket.next()).await.map_err(|_| {
            anyhow::anyhow!(if started_tts {
                "text turn timed out before tts:stop"
            } else {
                "text turn timed out waiting for tts:start"
            })
        })?;
        let message = match message {
            Some(Ok(message)) => message,
            Some(Err(error)) => return Err(error.into()),
            None => bail!("WebSocket closed during text turn"),
        };
        match message {
            Message::Binary(_) if !started_tts => bail!("audio_before_tts_start"),
            Message::Binary(packet) => {
                let mut pcm = [0_i16; 1_440];
                let decoded = decoder
                    .decode(&packet, &mut pcm, false)
                    .context("decode canonical downlink Opus packet")?;
                if decoded != pcm.len() {
                    bail!("downlink packet has an invalid canonical sample count");
                }
                decoded_audio.extend_from_slice(&pcm);
                packets += 1;
                debug_step(request.config.debug_steps, "ws_in=binary_opus");
            }
            Message::Text(text) => {
                let Ok(value) = serde_json::from_str::<serde_json::Value>(&text) else {
                    continue;
                };
                if value["type"] != "tts" {
                    continue;
                }
                match value["state"].as_str() {
                    Some("start") if !started_tts => {
                        validate_tts_session(&value, &session_id)?;
                        started_tts = true;
                        debug_step(request.config.debug_steps, "ws_in=tts_start");
                    }
                    Some("start") => bail!("duplicate_tts_start"),
                    Some("stop") => {
                        validate_tts_session(&value, &session_id)?;
                        if packets == 0 {
                            bail!("tts:stop received without audio");
                        }
                        debug_step(request.config.debug_steps, "ws_in=tts_stop");
                        break;
                    }
                    _ => {}
                }
            }
            Message::Close(_) => bail!("WebSocket closed during text turn"),
            _ => {}
        }
    }
    let quiet_deadline = Instant::now() + request.config.post_stop_quiet_period;
    loop {
        match timeout_at(quiet_deadline, socket.next()).await {
            Err(_) => break,
            Ok(None) => bail!("WebSocket closed during post-stop quiet period"),
            Ok(Some(Err(error))) => return Err(error.into()),
            Ok(Some(Ok(message))) => match message {
                Message::Binary(_) => bail!("audio_after_tts_stop"),
                Message::Close(_) => bail!("WebSocket closed during post-stop quiet period"),
                Message::Text(text) => {
                    if let Ok(value) = serde_json::from_str::<serde_json::Value>(&text)
                        && value["type"] == "tts"
                    {
                        bail!("lifecycle_after_tts_stop");
                    }
                }
                _ => {}
            },
        }
    }
    let debug_audio_file = request.config.debug_audio_file.clone();
    if let Some(path) = &debug_audio_file {
        write_debug_wav(path, &decoded_audio)?;
        debug_step(request.config.debug_steps, "debug_audio_written");
    }
    Ok(TextTurnReport {
        binary_packets: packets,
        debug_audio_file,
    })
}

fn validate_server_hello(hello: &ServerHello) -> anyhow::Result<()> {
    ensure!(hello.message_type == "hello", "invalid ServerHello: type");
    ensure!(
        hello.transport == "websocket",
        "invalid ServerHello: transport"
    );
    ensure!(
        !hello.session_id.trim().is_empty(),
        "invalid ServerHello: session_id"
    );
    let audio = &hello.audio_params;
    ensure!(audio.format == "opus", "invalid ServerHello: audio format");
    ensure!(
        audio.sample_rate == 24_000,
        "invalid ServerHello: audio sample_rate"
    );
    ensure!(audio.channels == 1, "invalid ServerHello: audio channels");
    ensure!(
        audio.frame_duration == 60,
        "invalid ServerHello: audio frame_duration"
    );
    Ok(())
}

fn validate_tts_session(value: &serde_json::Value, expected: &str) -> anyhow::Result<()> {
    let Some(id) = value.get("session_id").and_then(serde_json::Value::as_str) else {
        bail!("invalid tts lifecycle control: session_id");
    };
    ensure!(
        !id.is_empty() && id == expected,
        "invalid tts lifecycle control: session correlation"
    );
    Ok(())
}

fn debug_step(enabled: bool, step: &str) {
    if enabled {
        eprintln!("send-text {step}");
    }
}

fn write_debug_wav(path: &std::path::Path, samples: &[i16]) -> anyhow::Result<()> {
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: 24_000,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut writer = hound::WavWriter::create(path, spec)
        .with_context(|| format!("create debug WAV {}", path.display()))?;
    for sample in samples {
        writer.write_sample(*sample)?;
    }
    writer.finalize()?;
    Ok(())
}

/// Decodes exactly one canonical downlink Opus packet.
pub fn decode_canonical_downlink_opus_packet(packet: &[u8]) -> anyhow::Result<usize> {
    if packet.is_empty() {
        bail!("canonical downlink packet is empty");
    }
    let mut decoder =
        Decoder::new(24_000, Channels::Mono).context("create canonical downlink Opus decoder")?;
    let mut pcm = [0_i16; 1_440];
    let decoded = decoder
        .decode(packet, &mut pcm, false)
        .context("decode canonical downlink Opus packet")?;
    if decoded != pcm.len() {
        bail!(
            "downlink packet decoded to {decoded} samples; expected one 60 ms 24 kHz frame ({})",
            pcm.len()
        );
    }
    Ok(decoded)
}
