use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use base64::Engine as _;
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use tokio::sync::{Mutex as AsyncMutex, broadcast, oneshot};
use tokio_tungstenite::connect_async_with_config;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::HeaderValue;
use tokio_tungstenite::tungstenite::protocol::WebSocketConfig;

use super::noise::{CipherState, NoiseXXInitiator};
use super::pool::UpstreamFailureKind;
use super::proto::{
    DecodedServiceFrame, NoiseFrameDecoder, SERVICE_DAEMON, SERVICE_SENTINEL,
    decode_service_response_frame, encode_application_request, encode_service_frame,
    encode_service_request, split_into_transport_frames,
};
use super::workflow::WorkflowFailure;

const READ_CHUNK_SIZE: usize = 131_072; // 128 KiB per /fs/read call
const WRITE_CHUNK_SIZE: usize = 262_144; // 256 KiB per /fs/write call
const PING_INTERVAL: Duration = Duration::from_secs(25);
const REPLAY_INITIAL_WAIT: Duration = Duration::from_millis(20);
const REPLAY_QUIET_WINDOW: Duration = Duration::from_millis(350);
const REPLAY_MAX_SETTLE: Duration = Duration::from_millis(1500);

pub(super) const ISOLATION_POLICY_MARKDOWN: &str = "\
# Session Isolation Policy (MANDATORY)\n\n\
This environment serves multiple independent API users concurrently. Each chat session belongs to a separate user.\n\n\
## Rules\n\
1. **Never reference other sessions**: Do not read, mention, or use information from any session other than the current one.\n\
2. **Ignore shared workspace artifacts from other tasks**: Only access files explicitly created or referenced within the current conversation turn.\n\
3. **Treat each session as a clean slate**: Only use the conversation history within the current session.\n";

#[derive(Debug, Clone)]
pub(crate) enum VmError {
    AuthInvalid(String),
    QuotaExceeded(String),
    Timeout(String),
    Transport(String),
}

impl std::fmt::Display for VmError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            VmError::AuthInvalid(msg) => write!(f, "vm auth invalid: {msg}"),
            VmError::QuotaExceeded(msg) => write!(f, "vm quota exceeded: {msg}"),
            VmError::Timeout(msg) => write!(f, "vm timeout: {msg}"),
            VmError::Transport(msg) => write!(f, "vm transport error: {msg}"),
        }
    }
}

impl From<VmError> for WorkflowFailure {
    fn from(err: VmError) -> Self {
        match err {
            VmError::AuthInvalid(msg) => {
                WorkflowFailure::Upstream(UpstreamFailureKind::AuthInvalid, msg)
            }
            VmError::QuotaExceeded(msg) => {
                WorkflowFailure::Upstream(UpstreamFailureKind::QuotaExceeded, msg)
            }
            VmError::Timeout(msg) => WorkflowFailure::Timeout(msg),
            VmError::Transport(msg) => {
                WorkflowFailure::Upstream(UpstreamFailureKind::Transient, msg)
            }
        }
    }
}

/// Real-time event emitted on the `/chat/subscribe` stream with schema-aware accessors.
#[derive(Debug, Clone)]
pub(crate) struct VmStreamEvent {
    pub raw: Value,
}

impl VmStreamEvent {
    pub fn event_name(&self) -> &str {
        self.raw
            .get("event")
            .or_else(|| self.raw.get("event_name"))
            .or_else(|| self.raw.get("type"))
            .and_then(Value::as_str)
            .unwrap_or("")
    }

    pub fn payload(&self) -> &Value {
        self.raw.get("payload").unwrap_or(&self.raw)
    }

    pub fn session_id(&self) -> Option<&str> {
        let p = self.payload();
        p.get("session_id")
            .or_else(|| self.raw.get("session_id"))
            .or_else(|| {
                p.get("chat_context")
                    .and_then(|c| c.get("chat_id").or_else(|| c.get("session_id")))
            })
            .or_else(|| {
                self.raw
                    .get("chat_context")
                    .and_then(|c| c.get("chat_id").or_else(|| c.get("session_id")))
            })
            .or_else(|| p.get("chat_id"))
            .or_else(|| self.raw.get("chat_id"))
            .or_else(|| {
                p.get("session")
                    .and_then(|s| s.get("session_id").or_else(|| s.get("id")))
            })
            .and_then(Value::as_str)
    }

    pub fn activity_code(&self) -> Option<&str> {
        if self.event_name() != "agent.status" {
            return None;
        }
        self.payload().get("activity_code").and_then(Value::as_str)
    }

    /// Inspects `agent.status` (`payload.activity_code`) per `src/core/client.js:1264-1276`.
    pub fn agent_activity_state(&self) -> Option<bool> {
        match self.activity_code()? {
            "working" | "making_something" | "responding" | "thinking" | "running" => Some(true),
            "online" | "idle" => Some(false),
            _ => None,
        }
    }

    pub fn session_update_status(&self, target_session_id: &str) -> Option<&str> {
        if self.event_name() != "sessions.updated" {
            return None;
        }
        let p = self.payload();
        let change = p.get("change").and_then(Value::as_str);
        let sess = p.get("session")?;
        let id = sess
            .get("session_id")
            .or_else(|| sess.get("id"))
            .and_then(Value::as_str)
            .or_else(|| self.session_id())?;
        if id != target_session_id {
            return None;
        }
        let status = sess.get("status").and_then(Value::as_str)?;
        if status == "completed" && change == Some("created") {
            return None;
        }
        Some(status)
    }

    /// Inspects `sessions.updated` (`payload.change != "created"` && `payload.session.status == "completed"`).
    pub fn is_session_completed_for(&self, target_session_id: &str) -> bool {
        self.session_update_status(target_session_id) == Some("completed")
    }
}

#[derive(Debug, Clone)]
pub(crate) struct RpcResponse {
    pub status: u32,
    pub body: Vec<u8>,
}

impl RpcResponse {
    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.body).into_owned()
    }

    pub fn json(&self) -> Result<Value, VmError> {
        serde_json::from_slice(&self.body)
            .map_err(|e| VmError::Transport(format!("invalid RPC JSON: {e}")))
    }
}

/// Internal seam allowing replacement of the live WebSocket Noise transport with `FakeMuseVmTransport` in tests.
#[async_trait]
pub(crate) trait MuseVmTransport: Send + Sync {
    async fn connect(
        &self,
        wss_url: &str,
    ) -> Result<(Box<dyn MuseVmSender>, Box<dyn MuseVmReceiver>), VmError>;
}

#[async_trait]
pub(crate) trait MuseVmSender: Send {
    /// Encrypts and sends a sequence of fragmented `NoiseTransportFrame` payloads in strict nonce order.
    async fn send_frames(&mut self, plaintext_frames: Vec<Vec<u8>>) -> Result<(), VmError>;
    async fn send_ping(&mut self) -> Result<(), VmError> {
        Ok(())
    }
    async fn close(&mut self);
}

#[async_trait]
pub(crate) trait MuseVmReceiver: Send {
    /// Receives, decrypts, and reassembles a complete `DecodedServiceFrame`.
    async fn recv_frame(&mut self) -> Result<Option<DecodedServiceFrame>, VmError>;
}

/// Production WebSocket + `Noise_XX_25519_AESGCM_SHA256` transport to `wss://hatch.metaaivm.com/v1/noise`.
pub(crate) struct WsNoiseVmTransport;

type WsStream =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

struct WsNoiseVmSender {
    sink: futures_util::stream::SplitSink<WsStream, Message>,
    send_cipher: CipherState,
}

#[async_trait]
impl MuseVmSender for WsNoiseVmSender {
    async fn send_frames(&mut self, plaintext_frames: Vec<Vec<u8>>) -> Result<(), VmError> {
        for frame in plaintext_frames {
            let encrypted = self
                .send_cipher
                .encrypt_with_ad(&[], &frame)
                .map_err(VmError::Transport)?;
            self.sink
                .send(Message::Binary(encrypted.into()))
                .await
                .map_err(|e| VmError::Transport(format!("websocket send failed: {e}")))?;
        }
        Ok(())
    }

    async fn send_ping(&mut self) -> Result<(), VmError> {
        self.sink
            .send(Message::Ping(Vec::new().into()))
            .await
            .map_err(|e| VmError::Transport(format!("websocket ping failed: {e}")))
    }

    async fn close(&mut self) {
        let _ = self.sink.close().await;
    }
}

struct WsNoiseVmReceiver {
    stream: futures_util::stream::SplitStream<WsStream>,
    recv_cipher: CipherState,
    decoder: NoiseFrameDecoder,
}

#[async_trait]
impl MuseVmReceiver for WsNoiseVmReceiver {
    async fn recv_frame(&mut self) -> Result<Option<DecodedServiceFrame>, VmError> {
        while let Some(msg_res) = self.stream.next().await {
            let msg = msg_res
                .map_err(|e| VmError::Transport(format!("websocket receive failed: {e}")))?;
            match msg {
                Message::Binary(ciphertext) => {
                    let plaintext = self
                        .recv_cipher
                        .decrypt_with_ad(&[], ciphertext.as_ref())
                        .map_err(VmError::Transport)?;
                    if let Some(assembled) =
                        self.decoder.push(&plaintext).map_err(VmError::Transport)?
                        && let Some(decoded) =
                            decode_service_response_frame(&assembled).map_err(VmError::Transport)?
                    {
                        return Ok(Some(decoded));
                    }
                }
                Message::Close(close_frame) => {
                    if let Some(cf) = close_frame {
                        let code: u16 = cf.code.into();
                        if code == 1008 || code == 4001 || code == 4003 {
                            return Err(VmError::AuthInvalid(format!(
                                "Hatch VM closed WebSocket with auth code {code}: {}",
                                cf.reason
                            )));
                        }
                    }
                    return Ok(None);
                }
                _ => {}
            }
        }
        Ok(None)
    }
}

#[async_trait]
impl MuseVmTransport for WsNoiseVmTransport {
    async fn connect(
        &self,
        wss_url: &str,
    ) -> Result<(Box<dyn MuseVmSender>, Box<dyn MuseVmReceiver>), VmError> {
        let mut req = wss_url
            .into_client_request()
            .map_err(|e| VmError::Transport(format!("invalid wss url: {e}")))?;
        req.headers_mut()
            .insert("Origin", HeaderValue::from_static("https://muse.ai"));
        req.headers_mut().insert(
            "User-Agent",
            HeaderValue::from_static(super::credentials::DEFAULT_USER_AGENT),
        );

        let mut ws_config = WebSocketConfig::default();
        ws_config.max_message_size = Some(64 * 1024 * 1024);
        ws_config.max_frame_size = Some(64 * 1024 * 1024);

        let (mut ws_stream, _) = tokio::time::timeout(
            Duration::from_secs(20),
            connect_async_with_config(req, Some(ws_config), false),
        )
        .await
        .map_err(|_| VmError::Timeout("Hatch VM WebSocket connect timed out".to_string()))?
        .map_err(|e| {
            let msg = e.to_string();
            if msg.contains("401") || msg.contains("403") {
                VmError::AuthInvalid(format!("Hatch VM rejected WebSocket upgrade: {msg}"))
            } else {
                VmError::Transport(format!("Hatch VM WebSocket connect failed: {msg}"))
            }
        })?;

        let mut initiator = NoiseXXInitiator::new(None).map_err(VmError::Transport)?;
        let msg1 = initiator.write_message_1().map_err(VmError::Transport)?;
        ws_stream
            .send(Message::Binary(msg1.into()))
            .await
            .map_err(|e| VmError::Transport(format!("failed sending Noise message 1: {e}")))?;

        let msg2 = tokio::time::timeout(Duration::from_secs(20), async {
            while let Some(frame_res) = ws_stream.next().await {
                match frame_res {
                    Ok(Message::Binary(bin)) => return Ok(bin.to_vec()),
                    Ok(Message::Close(cf)) => {
                        let detail = cf
                            .map(|c| format!("code={}, reason={}", u16::from(c.code), c.reason))
                            .unwrap_or_else(|| "no close frame".to_string());
                        return Err(VmError::AuthInvalid(format!(
                            "WebSocket closed during Noise handshake: {detail}"
                        )));
                    }
                    Ok(_) => continue,
                    Err(e) => {
                        return Err(VmError::Transport(format!(
                            "WebSocket error during Noise handshake: {e}"
                        )));
                    }
                }
            }
            Err(VmError::Transport(
                "WebSocket EOF before Noise message 2".to_string(),
            ))
        })
        .await
        .map_err(|_| VmError::Timeout("Noise XX handshake timed out".to_string()))??;

        initiator
            .read_message_2(&msg2)
            .map_err(VmError::AuthInvalid)?;
        let (msg3, send_cipher, recv_cipher) =
            initiator.write_message_3().map_err(VmError::AuthInvalid)?;

        ws_stream
            .send(Message::Binary(msg3.into()))
            .await
            .map_err(|e| VmError::Transport(format!("failed sending Noise message 3: {e}")))?;

        let (sink, stream) = ws_stream.split();
        Ok((
            Box::new(WsNoiseVmSender { sink, send_cipher }),
            Box::new(WsNoiseVmReceiver {
                stream,
                recv_cipher,
                decoder: NoiseFrameDecoder::default(),
            }),
        ))
    }
}

struct PendingStream {
    status: u32,
    body_buf: Vec<u8>,
    line_buf: String,
    resolve_on_status: bool,
    tx: Option<oneshot::Sender<Result<RpcResponse, VmError>>>,
}

/// Multiplexed RPC & real-time event client over a single encrypted Hatch VM Noise session.
pub(crate) struct MuseVmClient {
    next_stream_id: AtomicU64,
    connected: Arc<AtomicBool>,
    policy_written: AtomicBool,
    sender: Arc<AsyncMutex<Box<dyn MuseVmSender>>>,
    pending_streams: Arc<Mutex<HashMap<u64, PendingStream>>>,
    event_tx: broadcast::Sender<VmStreamEvent>,
}

impl MuseVmClient {
    pub async fn connect(
        transport: &dyn MuseVmTransport,
        wss_url: &str,
    ) -> Result<Arc<Self>, VmError> {
        let (sender_box, mut receiver_box) = transport.connect(wss_url).await?;
        let connected = Arc::new(AtomicBool::new(true));
        let sender = Arc::new(AsyncMutex::new(sender_box));
        let pending_streams: Arc<Mutex<HashMap<u64, PendingStream>>> =
            Arc::new(Mutex::new(HashMap::new()));
        let (event_tx, _) = broadcast::channel(512);

        let client = Arc::new(Self {
            next_stream_id: AtomicU64::new(1),
            connected: Arc::clone(&connected),
            policy_written: AtomicBool::new(false),
            sender: Arc::clone(&sender),
            pending_streams: Arc::clone(&pending_streams),
            event_tx: event_tx.clone(),
        });

        let connected_ping = Arc::clone(&connected);
        let sender_ping = Arc::clone(&sender);
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(PING_INTERVAL);
            interval.tick().await;
            loop {
                interval.tick().await;
                if !connected_ping.load(Ordering::SeqCst) {
                    break;
                }
                let mut guard = sender_ping.lock().await;
                if guard.send_ping().await.is_err() {
                    connected_ping.store(false, Ordering::SeqCst);
                    break;
                }
            }
        });

        let connected_reader = Arc::clone(&connected);
        let pending_reader = Arc::clone(&pending_streams);
        let event_tx_reader = event_tx;
        tokio::spawn(async move {
            let mut global_line_buf = String::new();
            let mut close_err = VmError::Transport("Hatch VM connection closed".to_string());

            loop {
                match receiver_box.recv_frame().await {
                    Ok(Some(frame)) => {
                        handle_decoded_frame(
                            frame,
                            &pending_reader,
                            &event_tx_reader,
                            &mut global_line_buf,
                        );
                    }
                    Ok(None) => break,
                    Err(err) => {
                        close_err = err;
                        break;
                    }
                }
            }

            connected_reader.store(false, Ordering::SeqCst);
            let mut map = pending_reader.lock().expect("pending_streams mutex");
            for (_, mut entry) in map.drain() {
                if let Some(tx) = entry.tx.take() {
                    let _ = tx.send(Err(close_err.clone()));
                }
            }
        });

        Ok(client)
    }

    pub fn is_connected(&self) -> bool {
        self.connected.load(Ordering::SeqCst)
    }

    pub fn subscribe_events(&self) -> broadcast::Receiver<VmStreamEvent> {
        self.event_tx.subscribe()
    }

    pub async fn close(&self) {
        self.connected.store(false, Ordering::SeqCst);
        let mut guard = self.sender.lock().await;
        guard.close().await;
    }

    pub async fn send_request(
        &self,
        verb: &str,
        path: &str,
        body: Option<&[u8]>,
        service_type: u64,
        timeout: Duration,
        resolve_on_status: bool,
    ) -> Result<RpcResponse, VmError> {
        if !self.is_connected() {
            return Err(VmError::Transport(
                "Hatch VM socket is not connected".to_string(),
            ));
        }

        let stream_id = self.next_stream_id.fetch_add(1, Ordering::SeqCst);
        let headers: &[(&str, &str)] = if body.is_some() {
            &[
                ("accept", "application/json"),
                ("content-type", "application/json"),
            ]
        } else {
            &[("accept", "application/json")]
        };

        let app_req = encode_application_request(verb, path, headers, body, true);
        let svc_frame = encode_service_frame(stream_id, &app_req);
        let svc_req = encode_service_request(service_type, &svc_frame);
        let transport_frames = split_into_transport_frames(stream_id, &svc_req);

        let (tx, rx) = oneshot::channel();
        {
            let mut map = self.pending_streams.lock().expect("pending_streams mutex");
            map.insert(
                stream_id,
                PendingStream {
                    status: 200,
                    body_buf: Vec::new(),
                    line_buf: String::new(),
                    resolve_on_status,
                    tx: Some(tx),
                },
            );
        }

        {
            let mut sender_guard = self.sender.lock().await;
            if let Err(err) = sender_guard.send_frames(transport_frames).await {
                self.connected.store(false, Ordering::SeqCst);
                let mut map = self.pending_streams.lock().expect("pending_streams mutex");
                map.remove(&stream_id);
                return Err(err);
            }
        }

        let resp = match tokio::time::timeout(timeout, rx).await {
            Ok(Ok(res)) => res?,
            Ok(Err(_)) => {
                return Err(VmError::Transport(format!(
                    "RPC stream {stream_id} ({verb} {path}) cancelled"
                )));
            }
            Err(_) => {
                let mut map = self.pending_streams.lock().expect("pending_streams mutex");
                map.remove(&stream_id);
                return Err(VmError::Timeout(format!(
                    "RPC request ({verb} {path}) timed out after {}s",
                    timeout.as_secs()
                )));
            }
        };

        if resp.status == 401 || resp.status == 403 {
            return Err(VmError::AuthInvalid(format!(
                "RPC {verb} {path} unauthorized (HTTP {})",
                resp.status
            )));
        }
        if resp.status == 429 {
            return Err(VmError::QuotaExceeded(format!(
                "RPC {verb} {path} rate limited (HTTP 429)"
            )));
        }
        Ok(resp)
    }

    pub async fn subscribe(&self) -> Result<(), VmError> {
        let mut event_rx = self.subscribe_events();
        let payload = json!({
            "after_stream_seq": 0,
            "after_chat_event_seq": 0,
            "capabilities": [
                "chat_cancel",
                "delta_stream",
                "custom_reactions",
                "custom_reactions_facebook_thumbs_up_v1"
            ]
        });
        let resp = self
            .send_request(
                "POST",
                "/chat/subscribe",
                Some(payload.to_string().as_bytes()),
                SERVICE_DAEMON,
                Duration::from_secs(15),
                true,
            )
            .await?;
        if resp.status >= 400 {
            return Err(VmError::Transport(format!(
                "/chat/subscribe failed with status {}",
                resp.status
            )));
        }

        // Drain any immediate replay burst from the VM before starting new turns.
        tokio::task::yield_now().await;
        if tokio::time::timeout(REPLAY_INITIAL_WAIT, event_rx.recv())
            .await
            .is_ok()
        {
            let _ = tokio::time::timeout(REPLAY_MAX_SETTLE, async {
                while tokio::time::timeout(REPLAY_QUIET_WINDOW, event_rx.recv())
                    .await
                    .is_ok()
                {}
            })
            .await;
        }
        Ok(())
    }

    pub async fn ensure_isolation_policy(&self) {
        if self
            .policy_written
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_ok()
        {
            let _ = self
                .write_file("AGENTS.md", ISOLATION_POLICY_MARKDOWN.as_bytes())
                .await;
        }
    }

    pub async fn read_file(&self, path: &str, max_bytes: usize) -> Result<Vec<u8>, VmError> {
        let stripped = path.trim().trim_start_matches("sandbox://");
        let clean_path = if stripped.starts_with('/') {
            stripped.to_string()
        } else {
            format!("/home/user/{stripped}")
        };
        let mut offset = 0usize;
        let mut out = Vec::new();

        while offset <= max_bytes {
            let req_len = READ_CHUNK_SIZE.min(max_bytes.saturating_sub(offset).saturating_add(1));
            let payload = json!({
                "path": clean_path,
                "offset": offset,
                "len": req_len,
            });
            let resp = self
                .send_request(
                    "POST",
                    "/fs/read",
                    Some(payload.to_string().as_bytes()),
                    SERVICE_DAEMON,
                    Duration::from_secs(30),
                    false,
                )
                .await?;
            if resp.status != 200 {
                return Err(VmError::Transport(format!(
                    "/fs/read {clean_path} failed: HTTP {} {}",
                    resp.status,
                    resp.text()
                )));
            }
            let val = resp.json()?;
            let result = val.get("result").unwrap_or(&val);
            if val.get("ok").and_then(Value::as_bool) == Some(false) {
                return Err(VmError::Transport(format!(
                    "/fs/read {clean_path} returned error: {}",
                    resp.text()
                )));
            }
            let b64 = result
                .get("data_base64")
                .and_then(Value::as_str)
                .unwrap_or("");
            let eof = result.get("eof").and_then(Value::as_bool).unwrap_or(true);
            if !b64.is_empty() {
                let chunk = base64::engine::general_purpose::STANDARD
                    .decode(b64.trim())
                    .map_err(|e| {
                        VmError::Transport(format!("invalid base64 from /fs/read: {e}"))
                    })?;
                if offset + chunk.len() > max_bytes {
                    return Err(VmError::Transport(format!(
                        "file {clean_path} exceeds max_bytes limit ({max_bytes})"
                    )));
                }
                offset += chunk.len();
                out.extend_from_slice(&chunk);
                if eof || chunk.is_empty() {
                    break;
                }
            } else {
                break;
            }
        }
        Ok(out)
    }

    pub async fn write_file(&self, path: &str, data: &[u8]) -> Result<(), VmError> {
        let clean_path = path
            .trim()
            .trim_start_matches('/')
            .trim_start_matches("sandbox://");
        if data.is_empty() {
            let payload = json!({
                "path": clean_path,
                "data_base64": "",
                "offset": 0,
                "overwrite": true,
                "append": false,
                "create_parent": true,
            });
            let resp = self
                .send_request(
                    "POST",
                    "/fs/write",
                    Some(payload.to_string().as_bytes()),
                    SERVICE_DAEMON,
                    Duration::from_secs(30),
                    false,
                )
                .await?;
            if resp.status != 200 {
                return Err(VmError::Transport(format!(
                    "/fs/write {clean_path} failed: HTTP {}",
                    resp.status
                )));
            }
            return Ok(());
        }

        let mut offset = 0usize;
        let mut idx = 0usize;
        while offset < data.len() {
            let end = (offset + WRITE_CHUNK_SIZE).min(data.len());
            let b64 = base64::engine::general_purpose::STANDARD.encode(&data[offset..end]);
            let payload = json!({
                "path": clean_path,
                "data_base64": b64,
                "offset": offset,
                "overwrite": idx == 0,
                "append": idx > 0,
                "create_parent": true,
            });
            let resp = self
                .send_request(
                    "POST",
                    "/fs/write",
                    Some(payload.to_string().as_bytes()),
                    SERVICE_DAEMON,
                    Duration::from_secs(30),
                    false,
                )
                .await?;
            if resp.status != 200 {
                return Err(VmError::Transport(format!(
                    "/fs/write {clean_path} at offset {offset} failed: HTTP {}",
                    resp.status
                )));
            }
            offset = end;
            idx += 1;
        }
        Ok(())
    }

    pub async fn decide_approval(&self, approval_id: &str, allow: bool) -> Result<(), VmError> {
        let clean_id = approval_id.trim();
        if clean_id.is_empty() {
            return Ok(());
        }
        let path = format!("/approvals/{clean_id}/decide");
        let payload = json!({
            "decision": if allow { "allow_once" } else { "deny" }
        });
        let body_bytes = payload.to_string().into_bytes();
        if self
            .send_request(
                "POST",
                &path,
                Some(&body_bytes),
                SERVICE_SENTINEL,
                Duration::from_secs(10),
                false,
            )
            .await
            .is_ok()
        {
            return Ok(());
        }
        let _ = self
            .send_request(
                "POST",
                &path,
                Some(&body_bytes),
                SERVICE_DAEMON,
                Duration::from_secs(10),
                false,
            )
            .await?;
        Ok(())
    }

    pub async fn delete_session(&self, session_id: &str) -> Result<(), VmError> {
        let clean_id = session_id.trim();
        if clean_id.is_empty() {
            return Ok(());
        }
        let payload = json!({
            "method": "/api/session/delete",
            "session_id": clean_id,
        });
        let _ = self
            .send_request(
                "POST",
                "/session/delete",
                Some(payload.to_string().as_bytes()),
                SERVICE_DAEMON,
                Duration::from_secs(10),
                false,
            )
            .await?;
        Ok(())
    }
}

fn handle_decoded_frame(
    frame: DecodedServiceFrame,
    pending_streams: &Arc<Mutex<HashMap<u64, PendingStream>>>,
    event_tx: &broadcast::Sender<VmStreamEvent>,
    global_line_buf: &mut String,
) {
    let mut map = pending_streams.lock().expect("pending_streams mutex");
    let stream_id = frame.stream_id;

    if let Some(resp) = frame.response {
        if let Some(entry) = map.get_mut(&stream_id) {
            entry.status = resp.status;
            if entry.resolve_on_status {
                if !resp.body.is_empty() {
                    let text = String::from_utf8_lossy(&resp.body);
                    drain_ndjson_events(&mut entry.line_buf, &text, event_tx);
                }
                if let Some(tx) = entry.tx.take() {
                    let _ = tx.send(Ok(RpcResponse {
                        status: resp.status,
                        body: resp.body.clone(),
                    }));
                }
            } else {
                entry.body_buf.extend_from_slice(&resp.body);
                if resp.end_body {
                    let completed = map.remove(&stream_id).expect("stream entry exists");
                    if let Some(tx) = completed.tx {
                        let _ = tx.send(Ok(RpcResponse {
                            status: completed.status,
                            body: completed.body_buf,
                        }));
                    }
                    return;
                }
            }
        } else if !resp.body.is_empty() {
            let text = String::from_utf8_lossy(&resp.body);
            drain_ndjson_events(global_line_buf, &text, event_tx);
        }
    }

    if let Some(chunk) = frame.body_chunk {
        if let Some(entry) = map.get_mut(&stream_id) {
            if entry.resolve_on_status {
                if !chunk.data.is_empty() {
                    let text = String::from_utf8_lossy(&chunk.data);
                    drain_ndjson_events(&mut entry.line_buf, &text, event_tx);
                }
                if chunk.end_body {
                    map.remove(&stream_id);
                }
            } else {
                entry.body_buf.extend_from_slice(&chunk.data);
                if chunk.end_body {
                    let completed = map.remove(&stream_id).expect("stream entry exists");
                    if let Some(tx) = completed.tx {
                        let _ = tx.send(Ok(RpcResponse {
                            status: completed.status,
                            body: completed.body_buf,
                        }));
                    }
                }
            }
        } else if !chunk.data.is_empty() {
            let text = String::from_utf8_lossy(&chunk.data);
            drain_ndjson_events(global_line_buf, &text, event_tx);
        }
    }
}

fn drain_ndjson_events(
    line_buf: &mut String,
    incoming: &str,
    event_tx: &broadcast::Sender<VmStreamEvent>,
) {
    line_buf.push_str(incoming);
    while let Some(newline_idx) = line_buf.find('\n') {
        let line = line_buf[..newline_idx].trim().to_string();
        line_buf.drain(..=newline_idx);
        if line.is_empty() {
            continue;
        }
        if let Ok(val) = serde_json::from_str::<Value>(&line) {
            let _ = event_tx.send(VmStreamEvent { raw: val });
        }
    }
    let rem = line_buf.trim();
    if rem.starts_with('{')
        && rem.ends_with('}')
        && let Ok(val) = serde_json::from_str::<Value>(rem)
        && (val.get("event").is_some()
            || val.get("event_name").is_some()
            || val.get("type").is_some())
    {
        let _ = event_tx.send(VmStreamEvent { raw: val });
        line_buf.clear();
    }
}
