//! Reusable wire checks owned by the independent Voice Protocol Client.

use anyhow::{Context, bail, ensure};
use futures_util::{SinkExt, StreamExt};
use opus2::{Application, Channels, Decoder, Encoder};
use serde::Deserialize;
use serde_json::json;
use std::{path::PathBuf, time::Duration};
use tokio::{
    net::TcpStream,
    time::{Instant, timeout_at},
};
use tokio_tungstenite::{
    MaybeTlsStream, WebSocketStream, connect_async,
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AudioListenMode {
    Manual,
    Auto,
}

impl AudioListenMode {
    fn wire_value(self) -> &'static str {
        match self {
            Self::Manual => "manual",
            Self::Auto => "auto",
        }
    }
}

#[derive(Clone, Debug)]
pub struct AudioTurnRequest {
    pub ota_url: String,
    pub device_id: String,
    pub client_id: String,
    pub wav_file: PathBuf,
    pub mode: AudioListenMode,
    pub expected_stt_suffix: String,
    pub trailing_silence_frames: usize,
    pub timeout: Duration,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AudioTurnReport {
    pub uplink_packets: usize,
    pub stt_text: String,
}

#[derive(Clone, Debug)]
pub struct McpTextTurnOptions {
    pub ota_url: String,
    pub device_id: String,
    pub client_id: String,
    pub text: String,
    pub initial_value: i32,
    pub turn_timeout: Duration,
}

#[derive(Clone, Debug, PartialEq)]
pub struct McpToolCallRecord {
    pub request_id: u64,
    pub name: String,
    pub arguments: serde_json::Value,
}
#[derive(Clone, Debug, PartialEq)]
pub struct McpToolResultRecord {
    pub request_id: u64,
    pub is_error: bool,
    pub content: String,
}
#[derive(Clone, Debug, PartialEq)]
pub struct McpTextTurnReport {
    pub initial_value: i32,
    pub final_value: i32,
    pub discovered_tools: Vec<String>,
    pub received_calls: Vec<McpToolCallRecord>,
    pub tool_results: Vec<McpToolResultRecord>,
    pub final_assistant_text: Option<String>,
    pub server_hello_received: bool,
    pub initialize_received: bool,
    pub tools_list_requests: usize,
    pub tts_started: bool,
    pub tts_finished: bool,
}

/// Connection-wide report for the deterministic Device MCP server.
pub type McpSessionReport = McpTextTurnReport;

#[derive(Clone, Debug)]
pub struct McpSessionOptions {
    pub ota_url: String,
    pub device_id: String,
    pub client_id: String,
    pub initial_value: i32,
    pub turn_timeout: Duration,
}

/// A stateful Voice Protocol Client which serves deterministic Device MCP on one WebSocket.
pub struct ReferenceClient {
    socket: WebSocketStream<MaybeTlsStream<TcpStream>>,
    session_id: Option<String>,
    turn_timeout: Duration,
    report: McpSessionReport,
    discovery_complete: bool,
}

impl McpTextTurnOptions {
    pub fn new(ota_url: String, text: String) -> Self {
        Self {
            ota_url,
            device_id: "reference-client-mcp-01".into(),
            client_id: "reference-client-mcp".into(),
            text,
            initial_value: 10,
            turn_timeout: Duration::from_secs(60),
        }
    }
}

impl ReferenceClient {
    pub async fn connect(options: McpSessionOptions) -> anyhow::Result<Self> {
        ensure!(
            (0..=100).contains(&options.initial_value),
            "MCP initial value must be 0..=100"
        );
        let ota: serde_json::Value = reqwest::Client::new()
            .post(&options.ota_url)
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
        let mut request = url.into_client_request()?;
        request
            .headers_mut()
            .insert("Protocol-Version", "1".parse()?);
        request
            .headers_mut()
            .insert("Device-Id", options.device_id.parse()?);
        request
            .headers_mut()
            .insert("Client-Id", options.client_id.parse()?);
        if !token.is_empty() {
            request
                .headers_mut()
                .insert("Authorization", format!("Bearer {token}").parse()?);
        }
        let (mut socket, _) = connect_async(request).await?;
        socket.send(Message::Text(json!({"type":"hello", "version":1, "transport":"websocket", "features":{"mcp":true}, "audio_params":{"format":"opus","sample_rate":16000,"channels":1,"frame_duration":60}}).to_string().into())).await?;
        let mut client = Self {
            socket,
            session_id: None,
            turn_timeout: options.turn_timeout,
            report: McpSessionReport {
                initial_value: options.initial_value,
                final_value: options.initial_value,
                discovered_tools: Vec::new(),
                received_calls: Vec::new(),
                tool_results: Vec::new(),
                final_assistant_text: None,
                server_hello_received: false,
                initialize_received: false,
                tools_list_requests: 0,
                tts_started: false,
                tts_finished: false,
            },
            discovery_complete: false,
        };
        client.wait_for_discovery().await?;
        Ok(client)
    }

    pub async fn run_text_turn(&mut self, text: &str) -> anyhow::Result<()> {
        let text = normalize_text_input(text)?;
        ensure!(self.discovery_complete, "MCP discovery is incomplete");
        let session = self
            .session_id
            .as_deref()
            .context("MCP before ServerHello")?;
        self.socket
            .send(Message::Text(
                json!({"type":"listen", "session_id":session, "state":"start", "mode":"manual"})
                    .to_string()
                    .into(),
            ))
            .await?;
        self.socket
            .send(Message::Text(
                json!({"type":"listen", "session_id":session, "state":"detect", "text":text})
                    .to_string()
                    .into(),
            ))
            .await?;
        let deadline = Instant::now() + self.turn_timeout;
        loop {
            if self.process_next(deadline).await? {
                break;
            }
        }
        // The writer acknowledges the terminal TTS control asynchronously.  Give the
        // server a bounded handoff before arming the next turn on this same session.
        tokio::time::sleep(Duration::from_millis(250)).await;
        Ok(())
    }

    pub fn finish(self) -> McpSessionReport {
        self.report
    }

    async fn wait_for_discovery(&mut self) -> anyhow::Result<()> {
        let deadline = Instant::now() + self.turn_timeout;
        while !self.discovery_complete {
            self.process_next(deadline).await?;
        }
        ensure!(
            self.report.server_hello_received
                && self.report.initialize_received
                && self.report.tools_list_requests == 2,
            "incomplete MCP discovery"
        );
        Ok(())
    }

    async fn process_next(&mut self, deadline: Instant) -> anyhow::Result<bool> {
        let message = timeout_at(deadline, self.socket.next())
            .await
            .map_err(|_| anyhow::anyhow!("MCP session timed out"))?
            .context("WebSocket closed during MCP session")??;
        let Message::Text(text_frame) = message else {
            if matches!(message, Message::Close(_)) {
                bail!("WebSocket closed during MCP session");
            }
            return Ok(false);
        };
        let value: serde_json::Value = serde_json::from_str(&text_frame)?;
        match value.get("type").and_then(serde_json::Value::as_str) {
            Some("hello") => {
                let hello: ServerHello = serde_json::from_value(value)?;
                validate_server_hello(&hello)?;
                self.session_id = Some(hello.session_id);
                self.report.server_hello_received = true;
            }
            Some("mcp") => self.handle_mcp_request(value).await?,
            Some("llm") => {
                self.report.final_assistant_text = value["text"].as_str().map(str::to_owned)
            }
            Some("tts") if value["state"].as_str() == Some("start") => {
                self.report.tts_started = true
            }
            Some("tts") if value["state"].as_str() == Some("stop") => {
                self.report.tts_finished = true;
                return Ok(true);
            }
            _ => {}
        }
        Ok(false)
    }

    async fn handle_mcp_request(&mut self, value: serde_json::Value) -> anyhow::Result<()> {
        let id = value["payload"]["id"]
            .as_u64()
            .context("MCP request lacks numeric id")?;
        let method = value["payload"]["method"]
            .as_str()
            .context("MCP request lacks method")?;
        let session = self
            .session_id
            .as_deref()
            .context("MCP before ServerHello")?;
        ensure!(
            value["session_id"].as_str() == Some(session),
            "MCP session_id does not match"
        );
        let response = match method {
            "initialize" => {
                self.report.initialize_received = true;
                json!({"protocolVersion":"2024-11-05", "capabilities":{}, "serverInfo":{"name":"voice-reference-client","version":"0.1.0"}})
            }
            "tools/list" => {
                self.report.tools_list_requests += 1;
                let first_page = value["payload"]["params"]["cursor"].as_str().is_none();
                let (tools, next) = if first_page {
                    (vec![mcp_echo_tool(), mcp_get_value_tool()], Some("page-2"))
                } else {
                    self.discovery_complete = true;
                    (vec![mcp_set_value_tool()], None)
                };
                self.report.discovered_tools.extend(
                    tools
                        .iter()
                        .filter_map(|tool| tool["name"].as_str().map(str::to_owned)),
                );
                let mut result = json!({"tools": tools});
                if let Some(next) = next {
                    result["nextCursor"] = json!(next);
                };
                result
            }
            "tools/call" => {
                let name = value["payload"]["params"]["name"]
                    .as_str()
                    .unwrap_or_default()
                    .to_owned();
                let arguments = value["payload"]["params"]["arguments"].clone();
                self.report.received_calls.push(McpToolCallRecord {
                    request_id: id,
                    name: name.clone(),
                    arguments: arguments.clone(),
                });
                let (content, is_error) =
                    execute_mcp_test_tool(&name, &arguments, &mut self.report.final_value);
                self.report.tool_results.push(McpToolResultRecord {
                    request_id: id,
                    is_error,
                    content: content.clone(),
                });
                json!({"content":[{"type":"text", "text":content}], "isError":is_error})
            }
            _ => {
                self.socket.send(Message::Text(json!({"type":"mcp","session_id":session,"payload":{"jsonrpc":"2.0","id":id,"error":{"code":-32601,"message":"method not found"}}}).to_string().into())).await?;
                return Ok(());
            }
        };
        self.socket.send(Message::Text(json!({"type":"mcp","session_id":session,"payload":{"jsonrpc":"2.0","id":id,"result":response}}).to_string().into())).await?;
        Ok(())
    }
}

/// Executes one text turn while serving deterministic Device MCP over the same WebSocket.
pub async fn run_mcp_text_turn(options: McpTextTurnOptions) -> anyhow::Result<McpTextTurnReport> {
    let mut client = ReferenceClient::connect(McpSessionOptions {
        ota_url: options.ota_url,
        device_id: options.device_id,
        client_id: options.client_id,
        initial_value: options.initial_value,
        turn_timeout: options.turn_timeout,
    })
    .await?;
    client.run_text_turn(&options.text).await?;
    Ok(client.finish())
}

fn mcp_echo_tool() -> serde_json::Value {
    json!({"name":"test.echo", "description":"Echo deterministic test text.", "inputSchema":{"type":"object", "properties":{"text":{"type":"string"}}, "required":["text"], "additionalProperties":false}})
}
fn mcp_get_value_tool() -> serde_json::Value {
    json!({"name":"test.get_value", "description":"Get the deterministic test value.", "inputSchema":{"type":"object", "properties":{}, "additionalProperties":false}})
}
fn mcp_set_value_tool() -> serde_json::Value {
    json!({"name":"test.set_value", "description":"Set the deterministic test value.", "inputSchema":{"type":"object", "properties":{"value":{"type":"integer", "minimum":0, "maximum":100}}, "required":["value"], "additionalProperties":false}})
}

fn execute_mcp_test_tool(
    name: &str,
    arguments: &serde_json::Value,
    value: &mut i32,
) -> (String, bool) {
    match name {
        "test.echo" => arguments
            .get("text")
            .and_then(serde_json::Value::as_str)
            .map(|text| (json!({"text":text}).to_string(), false))
            .unwrap_or_else(|| (json!({"error":"invalid_arguments"}).to_string(), true)),
        "test.get_value" if arguments.as_object().is_some_and(|args| args.is_empty()) => {
            (json!({"value":*value}).to_string(), false)
        }
        "test.set_value" => match arguments.get("value").and_then(serde_json::Value::as_i64) {
            Some(next) if (0..=100).contains(&next) => {
                *value = next as i32;
                (json!({"value":*value}).to_string(), false)
            }
            Some(_) => (json!({"error":"value_out_of_range"}).to_string(), true),
            None => (json!({"error":"invalid_arguments"}).to_string(), true),
        },
        _ => (json!({"error":"unknown_tool"}).to_string(), true),
    }
}

/// Listening mode used by the strict acoustic barge-in qualification scenario.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BargeInMode {
    Auto,
    Realtime,
}

impl BargeInMode {
    fn wire_value(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Realtime => "realtime",
        }
    }
}

#[derive(Clone, Debug)]
pub struct BargeInConfig {
    pub tts_start_timeout: Duration,
    pub turn_timeout: Duration,
    pub post_stop_quiet_period: Duration,
}

impl Default for BargeInConfig {
    fn default() -> Self {
        Self {
            tts_start_timeout: Duration::from_secs(60),
            turn_timeout: Duration::from_secs(120),
            post_stop_quiet_period: Duration::from_millis(250),
        }
    }
}

impl BargeInConfig {
    fn validate(&self) -> anyhow::Result<()> {
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

/// Canonical Opus inputs and connection data for one same-socket barge-in flow.
#[derive(Clone, Debug)]
pub struct BargeInRequest {
    pub websocket_url: String,
    pub device_id: String,
    pub client_id: String,
    pub mode: BargeInMode,
    pub uplink_a: Vec<Vec<u8>>,
    pub uplink_b: Vec<Vec<u8>>,
    pub expected_b_stt: String,
    pub expected_b_llm: String,
    pub config: BargeInConfig,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BargeInReport {
    pub session_id: String,
    pub interruption_stops: usize,
    pub b_binary_packets: usize,
    pub b_stt_messages: usize,
    pub b_llm_messages: usize,
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

/// Replays a PCM16 mono WAV at microphone cadence and requires one matching STT final.
pub async fn run_audio_turn(request: AudioTurnRequest) -> anyhow::Result<AudioTurnReport> {
    ensure!(
        !request.timeout.is_zero(),
        "audio turn timeout must be greater than zero"
    );
    let expected_suffix = request.expected_stt_suffix.trim();
    ensure!(
        !expected_suffix.is_empty(),
        "expected STT suffix must not be empty"
    );
    let pcm = read_wav_as_uplink_pcm(&request.wav_file)?;
    ensure!(!pcm.is_empty(), "WAV input has no PCM samples");

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
        .and_then(serde_json::Value::as_str)
        .context("OTA response lacks websocket.url")?;
    let token = websocket
        .get("token")
        .and_then(serde_json::Value::as_str)
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
    socket.send(Message::Text(json!({"type":"hello","version":1,"transport":"websocket","audio_params":{"format":"opus","sample_rate":16000,"channels":1,"frame_duration":60}}).to_string().into())).await?;
    let hello = match timeout_at(Instant::now() + request.timeout, socket.next()).await? {
        Some(Ok(Message::Text(text))) => {
            serde_json::from_str::<ServerHello>(&text).context("invalid ServerHello")?
        }
        Some(Ok(_)) => bail!("invalid ServerHello: expected text frame"),
        Some(Err(error)) => return Err(error.into()),
        None => bail!("WebSocket closed before ServerHello"),
    };
    validate_server_hello(&hello)?;
    let session_id = hello.session_id;
    socket
        .send(Message::Text(
            json!({"session_id":session_id,"type":"listen","state":"start","mode":request.mode.wire_value()})
                .to_string()
                .into(),
        ))
        .await?;

    let mut encoder = Encoder::new(16_000, Channels::Mono, Application::Voip)
        .context("initialize uplink Opus encoder")?;
    let mut uplink_packets = 0;
    for frame in pcm.chunks(960) {
        let mut canonical = [0_i16; 960];
        canonical[..frame.len()].copy_from_slice(frame);
        send_uplink_frame(&mut socket, &mut encoder, &canonical).await?;
        uplink_packets += 1;
        tokio::time::sleep(Duration::from_millis(60)).await;
    }
    for _ in 0..request.trailing_silence_frames {
        send_uplink_frame(&mut socket, &mut encoder, &[0_i16; 960]).await?;
        tokio::time::sleep(Duration::from_millis(60)).await;
    }
    match request.mode {
        AudioListenMode::Manual => {
            socket
                .send(Message::Text(
                    json!({"session_id":session_id,"type":"listen","state":"stop"})
                        .to_string()
                        .into(),
                ))
                .await?;
        }
        AudioListenMode::Auto => {}
    }

    let deadline = Instant::now() + request.timeout;
    loop {
        let message = timeout_at(deadline, socket.next())
            .await
            .map_err(|_| anyhow::anyhow!("audio turn timed out waiting for STT"))?;
        match message {
            Some(Ok(Message::Text(text))) => {
                let Ok(value) = serde_json::from_str::<serde_json::Value>(&text) else {
                    continue;
                };
                if value["type"] != "stt" {
                    continue;
                }
                let stt_text = value["text"]
                    .as_str()
                    .context("STT message has no text")?
                    .to_owned();
                ensure!(
                    transcript_has_suffix(&stt_text, expected_suffix),
                    "STT final is missing the expected suffix"
                );
                return Ok(AudioTurnReport {
                    uplink_packets,
                    stt_text,
                });
            }
            Some(Ok(Message::Close(_))) => bail!("WebSocket closed before STT"),
            Some(Ok(_)) => {}
            Some(Err(error)) => return Err(error.into()),
            None => bail!("WebSocket closed before STT"),
        }
    }
}

async fn send_uplink_frame<S>(
    socket: &mut WebSocketStream<S>,
    encoder: &mut Encoder,
    pcm: &[i16; 960],
) -> anyhow::Result<()>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    let mut packet = [0_u8; 4_000];
    let encoded = encoder
        .encode(pcm, &mut packet)
        .context("encode canonical uplink Opus frame")?;
    ensure!(encoded > 0, "Opus encoder produced an empty uplink packet");
    socket
        .send(Message::Binary(packet[..encoded].to_vec().into()))
        .await?;
    Ok(())
}

fn transcript_has_suffix(actual: &str, expected: &str) -> bool {
    let normalize = |text: &str| {
        text.trim()
            .trim_end_matches(|character: char| character.is_ascii_punctuation())
            .to_lowercase()
    };
    normalize(actual).ends_with(&normalize(expected))
}

fn read_wav_as_uplink_pcm(path: &std::path::Path) -> anyhow::Result<Vec<i16>> {
    let mut reader = hound::WavReader::open(path)
        .with_context(|| format!("open WAV input {}", path.display()))?;
    let spec = reader.spec();
    ensure!(
        spec.sample_format == hound::SampleFormat::Int
            && spec.bits_per_sample == 16
            && spec.channels == 1,
        "WAV must be PCM16 mono"
    );
    let samples = reader
        .samples::<i16>()
        .collect::<Result<Vec<_>, _>>()
        .context("read PCM16 WAV samples")?;
    match spec.sample_rate {
        16_000 => Ok(samples),
        24_000 => Ok(resample_24khz_to_16khz(&samples)),
        rate => bail!("WAV sample rate must be 16000 or 24000 Hz, got {rate}"),
    }
}

fn resample_24khz_to_16khz(input: &[i16]) -> Vec<i16> {
    let output_len = input.len().saturating_mul(2) / 3;
    (0..output_len)
        .map(|index| {
            let position = index * 3;
            let lower = position / 2;
            let upper = (lower + 1).min(input.len().saturating_sub(1));
            if position.is_multiple_of(2) {
                input.get(lower).copied().unwrap_or_default()
            } else {
                ((i32::from(input[lower]) + i32::from(input[upper])) / 2) as i16
            }
        })
        .collect()
}

/// Runs the public Phase 5 acoustic barge-in contract over one WebSocket.
///
/// The client asserts AEC, starts Auto or Realtime capture, waits until A has
/// protocol-visible TTS, then sends B. It proves the interruption stop and the
/// next TTS lifecycle are correlated to the same Voice Session, and rejects
/// stale audio after the stop boundary.
pub async fn run_barge_in(request: BargeInRequest) -> anyhow::Result<BargeInReport> {
    request.config.validate()?;
    ensure!(!request.uplink_a.is_empty(), "uplink A must not be empty");
    ensure!(!request.uplink_b.is_empty(), "uplink B must not be empty");
    ensure!(
        !request.expected_b_stt.trim().is_empty(),
        "expected B STT must not be empty"
    );
    ensure!(
        !request.expected_b_llm.trim().is_empty(),
        "expected B LLM must not be empty"
    );
    for packet in request.uplink_a.iter().chain(&request.uplink_b) {
        decode_canonical_uplink_opus_packet(packet)?;
    }

    let mut connection = request.websocket_url.into_client_request()?;
    let headers = connection.headers_mut();
    headers.insert("Protocol-Version", "1".parse()?);
    headers.insert("Device-Id", request.device_id.parse()?);
    headers.insert("Client-Id", request.client_id.parse()?);
    let (mut socket, _) = connect_async(connection)
        .await
        .context("connect WebSocket")?;
    socket
        .send(Message::Text(
            json!({
                "type": "hello",
                "version": 1,
                "transport": "websocket",
                "audio_params": {
                    "format": "opus", "sample_rate": 16000, "channels": 1, "frame_duration": 60
                },
                "features": {"aec": true}
            })
            .to_string()
            .into(),
        ))
        .await?;
    let hello = match timeout_at(
        Instant::now() + request.config.tts_start_timeout,
        socket.next(),
    )
    .await?
    {
        Some(Ok(Message::Text(text))) => {
            serde_json::from_str::<ServerHello>(&text).context("invalid ServerHello")?
        }
        Some(Ok(_)) => bail!("invalid ServerHello: expected text frame"),
        Some(Err(error)) => return Err(error.into()),
        None => bail!("WebSocket closed before ServerHello"),
    };
    validate_server_hello(&hello)?;
    let session_id = hello.session_id;
    socket
        .send(Message::Text(
            json!({"session_id": session_id, "type": "listen", "state": "start", "mode": request.mode.wire_value()})
                .to_string()
                .into(),
        ))
        .await?;
    for packet in &request.uplink_a {
        socket.send(Message::Binary(packet.clone().into())).await?;
    }

    let started = Instant::now();
    let start_deadline = started + request.config.tts_start_timeout;
    loop {
        let message = timeout_at(start_deadline, socket.next())
            .await
            .map_err(|_| anyhow::anyhow!("barge-in timed out waiting for A tts:start"))?;
        match message {
            Some(Ok(Message::Text(text))) => {
                let Ok(value) = serde_json::from_str::<serde_json::Value>(&text) else {
                    continue;
                };
                if value["type"] == "tts" {
                    validate_tts_session(&value, &session_id)?;
                    if value["state"] == "start" {
                        break;
                    }
                }
            }
            Some(Ok(Message::Binary(_))) => bail!("audio_before_a_tts_start"),
            Some(Ok(Message::Close(_))) => bail!("WebSocket closed before A tts:start"),
            Some(Ok(_)) => {}
            Some(Err(error)) => return Err(error.into()),
            None => bail!("WebSocket closed before A tts:start"),
        }
    }
    for packet in &request.uplink_b {
        socket.send(Message::Binary(packet.clone().into())).await?;
    }

    let turn_deadline = started + request.config.turn_timeout;
    let mut interruption_stops = 0;
    let mut b_started = false;
    let mut b_stt_messages = 0;
    let mut b_llm_messages = 0;
    let mut b_binary_packets = 0;
    loop {
        let message = timeout_at(turn_deadline, socket.next())
            .await
            .map_err(|_| anyhow::anyhow!("barge-in timed out before B tts:stop"))?;
        let message = match message {
            Some(Ok(message)) => message,
            Some(Err(error)) => return Err(error.into()),
            None => bail!("WebSocket closed during barge-in"),
        };
        match message {
            Message::Binary(packet) => {
                if interruption_stops > 0 && !b_started {
                    bail!("stale_audio_after_interruption_stop");
                }
                if b_started {
                    decode_canonical_downlink_opus_packet(&packet)?;
                    b_binary_packets += 1;
                }
            }
            Message::Text(text) => {
                let Ok(value) = serde_json::from_str::<serde_json::Value>(&text) else {
                    continue;
                };
                if value["type"] == "tts" {
                    validate_tts_session(&value, &session_id)?;
                    match value["state"].as_str() {
                        Some("stop") if !b_started => {
                            interruption_stops += 1;
                            if interruption_stops > 1 {
                                bail!("duplicate_interruption_tts_stop");
                            }
                        }
                        Some("start") if !b_started => {
                            if interruption_stops != 1 || b_stt_messages != 1 || b_llm_messages != 1
                            {
                                bail!("B tts:start arrived before the expected B STT and LLM");
                            }
                            b_started = true;
                        }
                        Some("stop") => {
                            if b_binary_packets == 0 {
                                bail!("B tts:stop received without audio");
                            }
                            break;
                        }
                        Some("start") => bail!("duplicate_b_tts_start"),
                        _ => {}
                    }
                } else if value["type"] == "stt" {
                    if interruption_stops != 1 || b_started || b_stt_messages != 0 {
                        bail!("unexpected STT after interruption boundary");
                    }
                    if value["text"] != request.expected_b_stt {
                        bail!("unexpected STT after interruption boundary");
                    }
                    b_stt_messages = 1;
                } else if value["type"] == "llm" {
                    if interruption_stops != 1
                        || b_started
                        || b_stt_messages != 1
                        || b_llm_messages != 0
                    {
                        bail!("unexpected LLM after interruption boundary");
                    }
                    if value["text"] != request.expected_b_llm {
                        bail!("unexpected LLM after interruption boundary");
                    }
                    b_llm_messages = 1;
                }
            }
            Message::Close(_) => bail!("WebSocket closed during barge-in"),
            _ => {}
        }
    }
    ensure!(b_started, "B never reached tts:start");
    ensure!(b_stt_messages > 0, "B never reached STT");
    ensure!(b_llm_messages > 0, "B never reached LLM");

    let quiet_deadline = Instant::now() + request.config.post_stop_quiet_period;
    loop {
        match timeout_at(quiet_deadline, socket.next()).await {
            Err(_) => break,
            Ok(None) => bail!("WebSocket closed during post-stop quiet period"),
            Ok(Some(Err(error))) => return Err(error.into()),
            Ok(Some(Ok(Message::Binary(_)))) => bail!("audio_after_b_tts_stop"),
            Ok(Some(Ok(Message::Close(_)))) => {
                bail!("WebSocket closed during post-stop quiet period")
            }
            Ok(Some(Ok(Message::Text(text)))) => {
                if let Ok(value) = serde_json::from_str::<serde_json::Value>(&text)
                    && matches!(value["type"].as_str(), Some("tts" | "stt" | "llm"))
                {
                    bail!("turn_payload_after_b_tts_stop");
                }
            }
            Ok(Some(Ok(_))) => {}
        }
    }
    Ok(BargeInReport {
        session_id,
        interruption_stops,
        b_binary_packets,
        b_stt_messages,
        b_llm_messages,
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

/// Decodes exactly one canonical uplink Opus packet and returns its PCM samples.
pub fn decode_canonical_uplink_opus_packet(packet: &[u8]) -> anyhow::Result<[i16; 960]> {
    if packet.is_empty() {
        bail!("canonical uplink packet is empty");
    }
    let mut decoder =
        Decoder::new(16_000, Channels::Mono).context("create canonical uplink Opus decoder")?;
    let mut pcm = [0_i16; 960];
    let decoded = decoder
        .decode(packet, &mut pcm, false)
        .context("decode canonical uplink Opus packet")?;
    if decoded != pcm.len() {
        bail!(
            "uplink packet decoded to {decoded} samples; expected one 60 ms 16 kHz frame ({})",
            pcm.len()
        );
    }
    Ok(pcm)
}

#[cfg(test)]
mod phase5_fixture_tests {
    use super::{
        decode_canonical_uplink_opus_packet, resample_24khz_to_16khz, transcript_has_suffix,
    };

    const FIXTURES: [&[u8]; 5] = [
        include_bytes!("../tests/fixtures/phase5-uplink-01-silence.opus"),
        include_bytes!("../tests/fixtures/phase5-uplink-02-speech-a.opus"),
        include_bytes!("../tests/fixtures/phase5-uplink-03-silence.opus"),
        include_bytes!("../tests/fixtures/phase5-uplink-04-speech-b.opus"),
        include_bytes!("../tests/fixtures/phase5-uplink-05-silence.opus"),
    ];

    #[test]
    fn phase5_uplink_fixture_has_two_synthetic_speech_onsets() {
        let peak = FIXTURES.map(|packet| {
            decode_canonical_uplink_opus_packet(packet)
                .unwrap()
                .into_iter()
                .map(i16::unsigned_abs)
                .max()
                .unwrap()
        });

        assert!(peak[0] < 100, "fixture must start with silence");
        assert!(peak[1] > 1_000, "fixture must contain speech A");
        assert!(
            peak[2] < 100,
            "fixture must retain an intervening silence frame"
        );
        assert!(peak[3] > 1_000, "fixture must contain speech B");
        assert!(peak[4] < 100, "fixture must end with silence");
    }

    #[test]
    fn audio_turn_suffix_match_is_case_insensitive_and_ignores_terminal_punctuation() {
        assert!(transcript_has_suffix(
            "Không gian vẫn vô cùng ngột ngạt.",
            "VẪN VÔ CÙNG NGỘT NGẠT"
        ));
        assert!(!transcript_has_suffix(
            "Không gian vẫn vô cùng",
            "vẫn vô cùng ngột ngạt"
        ));
    }

    #[test]
    fn audio_turn_resamples_24khz_to_the_canonical_16khz_count() {
        assert_eq!(
            resample_24khz_to_16khz(&[0, 10, 20, 30, 40, 50]),
            [0, 15, 30, 45]
        );
    }
}
