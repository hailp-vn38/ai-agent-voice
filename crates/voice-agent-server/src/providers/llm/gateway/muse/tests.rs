use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use base64::Engine as _;
use futures_util::StreamExt;
use serde_json::{Value, json};
use tokio::sync::mpsc;

use super::client::{MuseVmReceiver, MuseVmSender, MuseVmTransport, VmError};
use super::config::{has_required_cookies, parse_accounts};
use super::credentials::{
    ParsedMuseCredential, build_wss_url, get_wss_expiry_unix_secs, merge_set_cookies,
};
use super::noise::{LOW_ORDER_POINTS, NoiseXXInitiator, NoiseXXResponder, x25519_dh};
use super::proto::{
    DecodedApplicationResponse, DecodedBodyChunk, DecodedServiceFrame, NoiseFrameDecoder,
    SERVICE_DAEMON, decode_service_request_frame, encode_application_request, encode_service_frame,
    encode_service_request, split_into_transport_frames,
};
use super::workflow::{
    decode_uncompressed_or_simple_png, encode_rgba_png, flatten_messages, format_image_prompt,
    format_video_prompt, remove_background_to_rgba_png,
};
use crate::config::DatabaseConfig;
use crate::database::Database;
use crate::database::credentials::CredentialCipher;
use crate::providers::llm::gateway::{LlmGatewayConfig, LlmGatewayProvider};
use crate::providers::llm::{
    ChatMessage, LlmEvent, LlmOperation, LlmOutput, LlmProvider, LlmRequest, MediaPart,
    ReferenceImage,
};
use crate::services::llm_gateway::{
    FakeOAuthVendorDriver, GatewayBinding, GatewayVendor, LlmGatewayHub, OAuthCallbackSubmission,
    OAuthVendorDriver, StartOAuthLoginRequest,
};

pub(in crate::providers::llm::gateway) const SINGLE_COOKIE: &str = "hatch_sess=sess_token_1";
pub(in crate::providers::llm::gateway) const SECOND_COOKIE: &str = "hatch_sess=sess_token_2";

pub(in crate::providers::llm::gateway) const VALID_PNG_BYTES: &[u8] = &[
    0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48, 0x44, 0x52,
    0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00, 0x00, 0x1F, 0x15, 0xC4,
    0x89, 0x00, 0x00, 0x00, 0x0A, 0x49, 0x44, 0x41, 0x54, 0x78, 0x9C, 0x63, 0x00, 0x01, 0x00, 0x00,
    0x05, 0x00, 0x01, 0x0D, 0x0A, 0x2D, 0xB4, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4E, 0x44, 0xAE,
    0x42, 0x60, 0x82,
];

pub(in crate::providers::llm::gateway) const VALID_MP4_BYTES: &[u8] = &[
    0x00, 0x00, 0x00, 0x18, b'f', b't', b'y', b'p', b'm', b'p', b'4', b'2', 0x00, 0x00, 0x00, 0x00,
    b'm', b'p', b'4', b'2', b'i', b's', b'o', b'm',
];

#[derive(Clone)]
pub(in crate::providers::llm::gateway) struct FakeStreamTurn {
    pub status: u32,
    pub assigned_session_id: Option<String>,
    pub events: Vec<Value>,
    pub history_polls: VecDeque<Vec<Value>>,
}

impl FakeStreamTurn {
    pub fn chat_reply(deltas: Vec<&str>, final_text: &str) -> Self {
        let mut events: Vec<Value> = deltas
            .into_iter()
            .map(|d| {
                json!({
                    "event": "delta.text_append",
                    "payload": { "text": d }
                })
            })
            .collect();
        events.push(json!({
            "event": "delta.message_done",
            "payload": {
                "transcript": {
                    "messages": [
                        { "role": "assistant", "content": final_text }
                    ]
                }
            }
        }));
        let history = vec![
            json!({ "role": "user", "display_text": "prompt" }),
            json!({ "role": "assistant", "status": "completed", "display_text": final_text }),
        ];
        Self {
            status: 200,
            assigned_session_id: None,
            events,
            history_polls: VecDeque::from([history]),
        }
    }

    pub fn media_reply(
        presentations: Vec<Value>,
        history_polls: Vec<Vec<Value>>,
        extra_events: Vec<Value>,
    ) -> Self {
        let mut events = extra_events;
        for p in presentations {
            events.push(json!({
                "event": "delta.presentation",
                "payload": p,
            }));
        }
        events.push(json!({
            "event": "agent.status",
            "payload": { "activity_code": "idle" }
        }));
        Self {
            status: 200,
            assigned_session_id: None,
            events,
            history_polls: VecDeque::from(history_polls),
        }
    }
}

pub(in crate::providers::llm::gateway) struct FakeVmAccount {
    turns: Mutex<VecDeque<FakeStreamTurn>>,
    files: Mutex<HashMap<String, Vec<u8>>>,
    written_files: Mutex<Vec<(String, Vec<u8>)>>,
    stream_requests: Mutex<Vec<Value>>,
    decided_approvals: Mutex<Vec<(String, String)>>,
    deleted_sessions: Mutex<Vec<String>>,
    active_history: Mutex<VecDeque<Vec<Value>>>,
}

impl FakeVmAccount {
    pub fn new(turns: Vec<FakeStreamTurn>) -> Arc<Self> {
        Arc::new(Self {
            turns: Mutex::new(VecDeque::from(turns)),
            files: Mutex::new(HashMap::new()),
            written_files: Mutex::new(Vec::new()),
            stream_requests: Mutex::new(Vec::new()),
            decided_approvals: Mutex::new(Vec::new()),
            deleted_sessions: Mutex::new(Vec::new()),
            active_history: Mutex::new(VecDeque::new()),
        })
    }

    pub fn with_file(self: &Arc<Self>, path: &str, bytes: Vec<u8>) -> Arc<Self> {
        let clean = path.trim().trim_start_matches("sandbox://").to_string();
        self.files.lock().unwrap().insert(clean, bytes);
        Arc::clone(self)
    }

    pub fn stream_requests(&self) -> Vec<Value> {
        self.stream_requests.lock().unwrap().clone()
    }

    pub fn decided_approvals(&self) -> Vec<(String, String)> {
        self.decided_approvals.lock().unwrap().clone()
    }

    pub fn deleted_sessions(&self) -> Vec<String> {
        self.deleted_sessions.lock().unwrap().clone()
    }

    pub fn written_files(&self) -> Vec<(String, Vec<u8>)> {
        self.written_files.lock().unwrap().clone()
    }
}

pub(in crate::providers::llm::gateway) struct FakeMuseVmTransport {
    accounts: Mutex<VecDeque<Arc<FakeVmAccount>>>,
}

impl FakeMuseVmTransport {
    pub fn new(accounts: Vec<Arc<FakeVmAccount>>) -> Arc<Self> {
        Arc::new(Self {
            accounts: Mutex::new(VecDeque::from(accounts)),
        })
    }
}

struct FakeVmSender {
    account: Arc<FakeVmAccount>,
    req_decoder: NoiseFrameDecoder,
    subscribe_stream_id: Option<u64>,
    tx: mpsc::UnboundedSender<Result<DecodedServiceFrame, VmError>>,
}

struct FakeVmReceiver {
    rx: mpsc::UnboundedReceiver<Result<DecodedServiceFrame, VmError>>,
}

#[async_trait]
impl MuseVmSender for FakeVmSender {
    async fn send_frames(&mut self, plaintext_frames: Vec<Vec<u8>>) -> Result<(), VmError> {
        for raw in plaintext_frames {
            let Some(assembled) = self.req_decoder.push(&raw).map_err(VmError::Transport)? else {
                continue;
            };
            let req = decode_service_request_frame(&assembled).map_err(VmError::Transport)?;
            let stream_id = req.stream_id;
            let path = req.path.as_str();

            if path == "/chat/subscribe" {
                self.subscribe_stream_id = Some(stream_id);
                let _ = self.tx.send(Ok(DecodedServiceFrame {
                    stream_id,
                    response: Some(DecodedApplicationResponse {
                        status: 200,
                        body: Vec::new(),
                        end_body: false,
                    }),
                    body_chunk: None,
                }));
            } else if path == "/fs/write" {
                let body_json: Value =
                    serde_json::from_slice(&req.body).unwrap_or_else(|_| json!({}));
                let file_path = body_json
                    .get("path")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                let b64 = body_json
                    .get("data_base64")
                    .and_then(Value::as_str)
                    .unwrap_or("");
                let bytes = base64::engine::general_purpose::STANDARD
                    .decode(b64)
                    .unwrap_or_default();
                self.account
                    .written_files
                    .lock()
                    .unwrap()
                    .push((file_path, bytes));
                send_json_response(&self.tx, stream_id, 200, &json!({ "ok": true }));
            } else if path == "/fs/read" {
                let body_json: Value =
                    serde_json::from_slice(&req.body).unwrap_or_else(|_| json!({}));
                let file_path = body_json
                    .get("path")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .trim_start_matches("sandbox://");
                let offset = body_json
                    .get("offset")
                    .and_then(Value::as_u64)
                    .unwrap_or(0) as usize;
                let len = body_json
                    .get("len")
                    .and_then(Value::as_u64)
                    .unwrap_or(131_072) as usize;
                let files = self.account.files.lock().unwrap();
                if let Some(full) = files.get(file_path) {
                    let end = (offset + len).min(full.len());
                    let slice = if offset < full.len() {
                        &full[offset..end]
                    } else {
                        &[]
                    };
                    let b64 = base64::engine::general_purpose::STANDARD.encode(slice);
                    let eof = end >= full.len();
                    send_json_response(
                        &self.tx,
                        stream_id,
                        200,
                        &json!({
                            "ok": true,
                            "result": {
                                "data_base64": b64,
                                "len": slice.len(),
                                "eof": eof,
                            }
                        }),
                    );
                } else {
                    send_json_response(
                        &self.tx,
                        stream_id,
                        404,
                        &json!({ "ok": false, "error": "file not found" }),
                    );
                }
            } else if path.starts_with("/approvals/") && path.ends_with("/decide") {
                let app_id = path
                    .trim_start_matches("/approvals/")
                    .trim_end_matches("/decide")
                    .to_string();
                let body_json: Value =
                    serde_json::from_slice(&req.body).unwrap_or_else(|_| json!({}));
                let decision = body_json
                    .get("decision")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                self.account
                    .decided_approvals
                    .lock()
                    .unwrap()
                    .push((app_id, decision));
                send_json_response(&self.tx, stream_id, 200, &json!({ "ok": true }));
            } else if path == "/session/delete" {
                let body_json: Value =
                    serde_json::from_slice(&req.body).unwrap_or_else(|_| json!({}));
                if let Some(sid) = body_json.get("session_id").and_then(Value::as_str) {
                    self.account
                        .deleted_sessions
                        .lock()
                        .unwrap()
                        .push(sid.to_string());
                }
                send_json_response(&self.tx, stream_id, 200, &json!({ "ok": true }));
            } else if path == "/chat/stream" {
                let body_json: Value =
                    serde_json::from_slice(&req.body).unwrap_or_else(|_| json!({}));
                self.account
                    .stream_requests
                    .lock()
                    .unwrap()
                    .push(body_json.clone());
                let req_sid = body_json
                    .get("session_id")
                    .and_then(Value::as_str)
                    .unwrap_or("default-session")
                    .to_string();

                let turn = self
                    .account
                    .turns
                    .lock()
                    .unwrap()
                    .pop_front()
                    .unwrap_or_else(|| FakeStreamTurn::chat_reply(vec!["ok"], "ok"));

                if turn.status >= 400 {
                    send_json_response(
                        &self.tx,
                        stream_id,
                        turn.status,
                        &json!({ "error": "unauthorized" }),
                    );
                    continue;
                }

                let effective_sid = turn.assigned_session_id.unwrap_or(req_sid);
                *self.account.active_history.lock().unwrap() = turn.history_polls;

                let _ = self.tx.send(Ok(DecodedServiceFrame {
                    stream_id,
                    response: Some(DecodedApplicationResponse {
                        status: 200,
                        body: json!({ "session_id": effective_sid })
                            .to_string()
                            .into_bytes(),
                        end_body: false,
                    }),
                    body_chunk: None,
                }));

                let sub_id = self.subscribe_stream_id.unwrap_or(stream_id);
                for mut ev in turn.events {
                    if let Some(obj) = ev.as_object_mut() {
                        obj.entry("session_id")
                            .or_insert_with(|| Value::String(effective_sid.clone()));
                    }
                    let line = format!("{ev}\n").into_bytes();
                    let _ = self.tx.send(Ok(DecodedServiceFrame {
                        stream_id: sub_id,
                        response: None,
                        body_chunk: Some(DecodedBodyChunk {
                            data: line,
                            end_body: false,
                        }),
                    }));
                }
            } else if path.starts_with("/chat/history") {
                let mut q = self.account.active_history.lock().unwrap();
                let events = if q.len() > 1 {
                    q.pop_front().unwrap_or_default()
                } else {
                    q.front().cloned().unwrap_or_default()
                };
                send_json_response(
                    &self.tx,
                    stream_id,
                    200,
                    &json!({
                        "ok": true,
                        "result": { "chat_events": events }
                    }),
                );
            } else {
                send_json_response(&self.tx, stream_id, 200, &json!({ "ok": true }));
            }
        }
        Ok(())
    }

    async fn close(&mut self) {}
}

#[async_trait]
impl MuseVmReceiver for FakeVmReceiver {
    async fn recv_frame(&mut self) -> Result<Option<DecodedServiceFrame>, VmError> {
        match self.rx.recv().await {
            Some(Ok(frame)) => Ok(Some(frame)),
            Some(Err(err)) => Err(err),
            None => Ok(None),
        }
    }
}

#[async_trait]
impl MuseVmTransport for FakeMuseVmTransport {
    async fn connect(
        &self,
        _wss_url: &str,
    ) -> Result<(Box<dyn MuseVmSender>, Box<dyn MuseVmReceiver>), VmError> {
        let account = {
            let mut q = self.accounts.lock().unwrap();
            if q.len() > 1 {
                q.pop_front().expect("account available")
            } else {
                q.front().cloned().ok_or_else(|| {
                    VmError::Transport("no fake VM accounts configured".to_string())
                })?
            }
        };
        let (tx, rx) = mpsc::unbounded_channel();
        Ok((
            Box::new(FakeVmSender {
                account,
                req_decoder: NoiseFrameDecoder::default(),
                subscribe_stream_id: None,
                tx,
            }),
            Box::new(FakeVmReceiver { rx }),
        ))
    }
}

fn send_json_response(
    tx: &mpsc::UnboundedSender<Result<DecodedServiceFrame, VmError>>,
    stream_id: u64,
    status: u32,
    body: &Value,
) {
    let _ = tx.send(Ok(DecodedServiceFrame {
        stream_id,
        response: Some(DecodedApplicationResponse {
            status,
            body: body.to_string().into_bytes(),
            end_body: true,
        }),
        body_chunk: None,
    }));
}

async fn build_gateway_muse_provider(
    account_cookies: &[&str],
    transport: Arc<FakeMuseVmTransport>,
) -> LlmGatewayProvider {
    let db = Database::connect(&DatabaseConfig {
        url: "sqlite::memory:".into(),
        max_connections: 1,
        ..Default::default()
    })
    .await
    .expect("in-memory db");
    let cipher = Arc::new(CredentialCipher::new(1, &[0x42; 32]).expect("cipher"));
    let mut drivers: std::collections::BTreeMap<GatewayVendor, Arc<dyn OAuthVendorDriver>> =
        std::collections::BTreeMap::new();
    for vendor in GatewayVendor::ALL {
        drivers.insert(vendor, Arc::new(FakeOAuthVendorDriver::new(vendor)));
    }
    let hub = Arc::new(LlmGatewayHub::with_drivers(
        Some(db.pool().clone()),
        cipher,
        drivers,
    ));

    for (idx, cookie) in account_cookies.iter().enumerate() {
        let ch = hub
            .start_oauth_login(StartOAuthLoginRequest {
                vendor: GatewayVendor::Muse,
                label: format!("Muse Test #{idx}"),
                redirect_uri: None,
            })
            .await
            .expect("start oauth");
        let acc = hub
            .complete_oauth_login(
                &ch.session_id,
                OAuthCallbackSubmission {
                    callback_url_or_code: (*cookie).to_string(),
                    state: None,
                },
            )
            .await
            .expect("complete oauth");
        let priority = 100i64.saturating_sub(idx as i64 * 10);
        hub.update_account(
            &acc.account_key,
            crate::services::llm_gateway::UpdateGatewayAccountPatch {
                label: None,
                enabled: None,
                priority: Some(priority),
            },
        )
        .await
        .expect("set priority");
    }

    let cfg = LlmGatewayConfig {
        gateway_binding: GatewayBinding::Pool(GatewayVendor::Muse),
        model: "muse-chat".to_string(),
        base_url: None,
        timeout_ms: 5_000,
        max_tokens: None,
        context_limit_tokens: None,
        reasoning_effort: None,
        temperature_percent: None,
        tags: None,
        account_concurrency: Some(2),
        cdp_port: Some(19222),
        headless: Some(true),
    };
    let transport_dyn: Arc<dyn MuseVmTransport> = transport;
    LlmGatewayProvider::with_hub_and_muse_transport(&cfg, hub, transport_dyn)
        .expect("gateway provider")
}

// ============================================================================
// 1. Noise_XX_25519_AESGCM_SHA256 Handshake & Transport Cryptography Tests
// ============================================================================

#[test]
fn noise_xx_handshake_and_transport_roundtrip() {
    let mut initiator = NoiseXXInitiator::new(None).expect("initiator");
    let mut responder = NoiseXXResponder::new(None).expect("responder");

    let msg1 = initiator.write_message_1().expect("write message 1");
    assert_eq!(msg1.len(), 66);

    let _client_nonce = responder
        .read_message_1(&msg1)
        .expect("responder reads message 1");
    let msg2 = responder
        .write_message_2(b"server-cert-payload")
        .expect("responder writes message 2");
    let cert_payload = initiator
        .read_message_2(&msg2)
        .expect("initiator reads message 2");
    assert_eq!(cert_payload, b"server-cert-payload");
    let (msg3, mut client_send, mut client_recv) = initiator
        .write_message_3()
        .expect("initiator writes message 3");
    let (_msg3_payload, mut server_send, mut server_recv) = responder
        .read_message_3(&msg3)
        .expect("responder reads message 3");

    for i in 0..5 {
        let client_plain = format!("hello from client frame #{i}").into_bytes();
        let ct = client_send
            .encrypt_with_ad(&[], &client_plain)
            .expect("client encrypt");
        let dec = server_recv
            .decrypt_with_ad(&[], &ct)
            .expect("server decrypt");
        assert_eq!(dec, client_plain);

        let server_plain = format!("reply from server frame #{i}").into_bytes();
        let s_ct = server_send
            .encrypt_with_ad(&[], &server_plain)
            .expect("server encrypt");
        let s_dec = client_recv
            .decrypt_with_ad(&[], &s_ct)
            .expect("client decrypt");
        assert_eq!(s_dec, server_plain);
    }
}

#[test]
fn noise_xx_rejects_low_order_public_keys() {
    let scalar = [7u8; 32];
    for point in &LOW_ORDER_POINTS {
        let res = x25519_dh(&scalar, point);
        assert!(res.is_err(), "must reject low-order X25519 point");
    }
}

// ============================================================================
// 2. Protobuf Wire Framing & 32 KB Multi-Chunk Reassembly Tests
// ============================================================================

#[test]
fn protobuf_request_and_multi_chunk_reassembly_roundtrip() {
    let large_body: Vec<u8> = (0..100_000).map(|i| (i % 251) as u8).collect();
    let headers: &[(&str, &str)] = &[
        ("accept", "application/json"),
        ("content-type", "application/json"),
    ];
    let app_req =
        encode_application_request("POST", "/fs/write", headers, Some(&large_body), true);
    let svc_frame = encode_service_frame(42, &app_req);
    let svc_req = encode_service_request(SERVICE_DAEMON, &svc_frame);

    let mut chunks = split_into_transport_frames(42, &svc_req);
    assert!(
        chunks.len() >= 4,
        "100 KB payload must split into >= 4 chunks"
    );
    // Reverse chunk arrival order to verify out-of-order reassembly
    chunks.reverse();

    let mut decoder = NoiseFrameDecoder::default();
    let mut assembled = None;
    for (idx, chunk) in chunks.iter().enumerate() {
        let res = decoder.push(chunk).expect("valid chunk");
        if idx + 1 < chunks.len() {
            assert!(res.is_none(), "must wait until all chunks arrive");
        } else {
            assembled = res;
        }
    }

    let assembled = assembled.expect("reassembled payload");
    assert_eq!(assembled, svc_req);

    let decoded_req = decode_service_request_frame(&assembled).expect("decode service request");
    assert_eq!(decoded_req.service_type, SERVICE_DAEMON);
    assert_eq!(decoded_req.stream_id, 42);
    assert_eq!(decoded_req.verb, "POST");
    assert_eq!(decoded_req.path, "/fs/write");
    assert_eq!(decoded_req.body, large_body);
}

// ============================================================================
// 3. Credential Normalization, Redaction & Token Expiry Tests
// ============================================================================

#[test]
fn credential_parser_accepts_single_hatch_sess_wss_hybrid_and_init_json() {
    // 1. Single hatch_sess cookie
    let c1 = ParsedMuseCredential::parse("hatch_sess=secret_cookie_val").expect("single cookie");
    assert_eq!(
        c1.cookie_header.as_deref(),
        Some("hatch_sess=secret_cookie_val")
    );
    assert!(c1.wss_url.is_none());
    assert!(!format!("{c1:?}").contains("secret_cookie_val"));
    assert!(format!("{c1:?}").contains("[REDACTED]"));

    // 2. Direct wss:// URL
    let wss = build_wss_url("vm-123", "s0:eyJ.eyJ.sig", "notary-xyz");
    let c2 = ParsedMuseCredential::parse(&wss).expect("direct wss");
    assert_eq!(c2.wss_url.as_deref(), Some(wss.as_str()));
    assert!(c2.cookie_header.is_none());
    assert!(!format!("{c2:?}").contains("notary-xyz"));

    // 3. Hybrid ("wss://...\nhatch_sess=...")
    let hybrid = format!("{wss}\nhatch_sess=hybrid_secret; theme=dark");
    let c3 = ParsedMuseCredential::parse(&hybrid).expect("hybrid");
    assert_eq!(c3.wss_url.as_deref(), Some(wss.as_str()));
    assert_eq!(
        c3.cookie_header.as_deref(),
        Some("hatch_sess=hybrid_secret")
    );
    assert_eq!(
        c3.to_hybrid_string(),
        format!("{wss}\nhatch_sess=hybrid_secret")
    );

    // 4. __HATCH_GATEWAY_INIT__ JSON
    let init_json = r#"{"vmName":"vm-init-9","authToken":"tok-abc","noiseNotaryToken":"notary-9"}"#;
    let c4 = ParsedMuseCredential::parse(init_json).expect("init json");
    let built_wss = c4.wss_url.as_deref().expect("wss built from init json");
    assert!(built_wss.starts_with("wss://hatch.metaaivm.com/v1/noise?"));
    assert!(built_wss.contains("vm_id=vm-init-9"));
    assert!(built_wss.contains("auth_token=tok-abc"));

    // Verify MuseAccountCredential Debug redaction as well
    let accs = parse_accounts(&hybrid);
    assert_eq!(accs.len(), 1);
    assert_eq!(accs[0].vm_id.as_deref(), Some("vm-123"));
    let acc_dbg = format!("{:?}", accs[0]);
    assert!(!acc_dbg.contains("hybrid_secret"));
    assert!(!acc_dbg.contains("notary-xyz"));
}

#[test]
fn wss_expiry_extracts_notary_timeout_and_s0_prefixed_jwt_exp() {
    let jwt_payload = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .encode(r#"{"exp":1893457000,"iat":1893400000}"#);
    let auth_token = format!("s0:eyJhbGciOiJIUzI1NiJ9.{jwt_payload}.sig");

    // Raw timeout:<unix> in notary_token
    let wss_raw = build_wss_url("vm-1", &auth_token, "v1.timeout:1893456000.sig");
    assert_eq!(get_wss_expiry_unix_secs(&wss_raw), Some(1_893_456_000));

    // Base64-encoded timeout:<unix> in notary_token where JWT exp is smaller
    let b64_notary =
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode("meta:timeout:1893499000");
    let wss_b64 = build_wss_url("vm-1", &auth_token, &format!("hdr.{b64_notary}.sig"));
    assert_eq!(get_wss_expiry_unix_secs(&wss_b64), Some(1_893_457_000));

    // Set-Cookie merge ignores Max-Age=0 deletion headers
    let mut headers = reqwest::header::HeaderMap::new();
    headers.append(
        reqwest::header::SET_COOKIE,
        "hatch_sess=deleted; Max-Age=0; Path=/".parse().unwrap(),
    );
    headers.append(
        reqwest::header::SET_COOKIE,
        "hatch_gw=new_gw; Path=/; HttpOnly".parse().unwrap(),
    );
    let merged = merge_set_cookies("hatch_sess=keep_me", &headers);
    assert_eq!(merged, "hatch_gw=new_gw; hatch_sess=keep_me");
    assert!(has_required_cookies(&super::config::parse_cookie_header(
        &merged
    )));
}

#[test]
fn flatten_messages_and_media_prompts_preserve_structure() {
    let req = LlmRequest {
        messages: vec![
            ChatMessage::System {
                content: "Be concise.".to_string(),
            },
            ChatMessage::User {
                content: "Hello!".to_string(),
            },
        ],
        tools: Vec::new(),
    };
    let flat = flatten_messages(&req);
    assert!(flat.contains("[System]\nBe concise."));
    assert!(flat.contains("[User]\nHello!"));

    let img_prompt = format_image_prompt("A neon city", 1920, 1080, true);
    assert!(img_prompt.contains("aspect ratio 16:9"));
    assert!(img_prompt.contains("solid, uniform, flat pure-white (#FFFFFF) studio background"));

    let vid_prompt = format_video_prompt("A rocket launch", 1280, 720, 28);
    assert!(vid_prompt.contains("3 sequential ~10-second beats"));
    assert!(vid_prompt.contains("[ALL_MEDIA_ATTACHED]"));
}

fn make_test_rgba_png(width: u32, height: u32, fg_range: std::ops::Range<u32>) -> Vec<u8> {
    let mut rgba = vec![255u8; (width * height * 4) as usize];
    for y in fg_range.clone() {
        for x in fg_range.clone() {
            let idx = ((y * width + x) * 4) as usize;
            rgba[idx] = 220;
            rgba[idx + 1] = 30;
            rgba[idx + 2] = 40;
            rgba[idx + 3] = 255;
        }
    }
    encode_rgba_png(width, height, &rgba)
}

#[test]
fn remove_background_to_rgba_png_clears_border_background() {
    let encoded = make_test_rgba_png(16, 16, 4..12);
    let out_bytes = remove_background_to_rgba_png(&encoded).expect("remove background");
    let (w, _h, rgba) =
        decode_uncompressed_or_simple_png(&out_bytes).expect("decode output png");
    assert_eq!(
        rgba[3],
        0,
        "corner background must be transparent"
    );
    let center_idx = ((8 * w + 8) * 4 + 3) as usize;
    assert_eq!(
        rgba[center_idx],
        255,
        "center foreground must remain opaque"
    );
}

// ============================================================================
// 4. End-to-End Integration Tests via LlmProvider Seam + FakeMuseVmTransport
// ============================================================================

#[tokio::test]
async fn chat_stream_emits_realtime_deltas_and_reuses_session_id_on_followup() {
    let mut turn1 =
        FakeStreamTurn::chat_reply(vec!["Xin ", "chào ", "bạn!"], "Xin chào bạn!");
    turn1.assigned_session_id = Some("sess-hot-1".to_string());
    let turn2 = FakeStreamTurn::chat_reply(vec!["Hôm nay ", "trời đẹp."], "Hôm nay trời đẹp.");

    let acc = FakeVmAccount::new(vec![turn1, turn2]);
    let transport = FakeMuseVmTransport::new(vec![Arc::clone(&acc)]);
    let provider = build_gateway_muse_provider(&[SINGLE_COOKIE], transport).await;

    // Turn 1: stream deltas
    let req1 = LlmRequest::text_turn("Chào Muse".to_string());
    let mut stream = provider.stream(req1).await.expect("open stream");
    let mut deltas = Vec::new();
    while let Some(item) = stream.next().await {
        match item.expect("stream event") {
            LlmEvent::TextDelta(d) => deltas.push(d),
            LlmEvent::Finished => break,
            _ => {}
        }
    }
    assert_eq!(deltas.concat(), "Xin chào bạn!");
    assert!(
        acc.written_files()
            .iter()
            .any(|(path, _)| path == "AGENTS.md"),
        "must write isolation policy AGENTS.md on connect"
    );

    // Turn 2: follow-up in the same conversation must reuse `sess-hot-1` and only send the latest user prompt
    let req2 = LlmRequest {
        messages: vec![
            ChatMessage::User {
                content: "Chào Muse".to_string(),
            },
            ChatMessage::AssistantText {
                content: "Xin chào bạn!".to_string(),
            },
            ChatMessage::User {
                content: "Thời tiết thế nào?".to_string(),
            },
        ],
        tools: Vec::new(),
    };
    let reply2 = provider
        .invoke(LlmOperation::Chat {
            request: req2,
            reasoning: false,
        })
        .await
        .expect("turn 2 chat");
    assert_eq!(reply2, LlmOutput::Text("Hôm nay trời đẹp.".to_string()));

    let reqs = acc.stream_requests();
    assert_eq!(reqs.len(), 2);
    assert_eq!(
        reqs[1].get("session_id").and_then(Value::as_str),
        Some("sess-hot-1"),
        "follow-up turn must reuse hot session_id"
    );
    assert_eq!(
        reqs[1]["items"][0]["text"].as_str(),
        Some("Thời tiết thế nào?"),
        "follow-up turn must send only the new user message"
    );
}

#[tokio::test]
async fn image_generation_attaches_reference_images_reads_fs_and_removes_background() {
    let raw_png = make_test_rgba_png(12, 12, 3..9);

    let turn = FakeStreamTurn::media_reply(
        vec![json!({
            "id": "pres-img-1",
            "kind": "image",
            "image_path": "sandbox:///home/user/generated_subject.png",
        })],
        vec![vec![
            json!({ "role": "user", "display_text": "prompt" }),
            json!({
                "role": "assistant",
                "status": "completed",
                "display_text": "Here is your sticker [ALL_MEDIA_ATTACHED]"
            }),
        ]],
        vec![],
    );

    let acc = FakeVmAccount::new(vec![turn])
        .with_file("/home/user/generated_subject.png", raw_png.clone());
    let transport = FakeMuseVmTransport::new(vec![Arc::clone(&acc)]);
    let provider = build_gateway_muse_provider(&[SINGLE_COOKIE], transport).await;

    let output = provider
        .invoke(LlmOperation::ImageGeneration {
            prompt: "A green crystal badge".to_string(),
            width: 1024,
            height: 1024,
            reference_images: vec![
                ReferenceImage {
                    bytes: VALID_PNG_BYTES.to_vec(),
                    mime_type: "image/png".to_string(),
                },
                ReferenceImage {
                    bytes: VALID_PNG_BYTES.to_vec(),
                    mime_type: "image/jpeg".to_string(),
                },
            ],
            transparent_background: true,
        })
        .await
        .expect("image generation succeeds");

    let reqs = acc.stream_requests();
    assert_eq!(reqs.len(), 1);
    let items = reqs[0]["items"].as_array().expect("items array");
    assert_eq!(items.len(), 3, "2 reference images + 1 text item");
    assert_eq!(items[0]["type"].as_str(), Some("image"));
    assert_eq!(items[1]["type"].as_str(), Some("image"));
    assert_eq!(items[2]["type"].as_str(), Some("text"));

    let LlmOutput::Media {
        bytes,
        mime_type,
        assistant_message,
        ..
    } = output
    else {
        panic!("expected LlmOutput::Media");
    };
    assert_eq!(assistant_message.as_deref(), Some("Here is your sticker"));
    assert_eq!(mime_type, "image/png");
    let (_w, _h, rgba) =
        decode_uncompressed_or_simple_png(&bytes).expect("decode output rgba png");
    assert_eq!(rgba[3], 0, "background removed to alpha 0");

    // Allow spawned ephemeral session cleanup task to run
    tokio::task::yield_now().await;
    assert_eq!(
        acc.deleted_sessions().len(),
        1,
        "ephemeral media session must be deleted via /session/delete"
    );
}

#[tokio::test]
async fn video_generation_harvests_all_multi_clip_paths_via_fs_read_and_assistant_message() {
    let clip1_bytes = [VALID_MP4_BYTES, &[1u8; 4]].concat();
    let clip2_bytes = [VALID_MP4_BYTES, &[2u8; 4]].concat();
    let clip3_bytes = [VALID_MP4_BYTES, &[3u8; 4]].concat();

    // Poll 1: tool_group is still running ("in_progress") with only 2 clips -> workflow must wait!
    let poll_running = vec![
        json!({ "role": "user", "display_text": "prompt" }),
        json!({ "role": "assistant", "display_text": "Intro: 3 scenes (30s total)" }),
        json!({
            "event_name": "tool_group",
            "payload": { "status": "in_progress", "data": { "display_state": "working" } }
        }),
        json!({
            "event_name": "delta.presentation",
            "payload": {
                "kind": "video",
                "data": {
                    "videos": [
                        { "path": "sandbox:///home/user/clip_1.mp4" },
                        { "path": "sandbox:///home/user/clip_2.mp4" }
                    ]
                }
            }
        }),
    ];

    // Poll 2: tool_group completed, all 3 clips attached, and [ALL_MEDIA_ATTACHED] marker present
    let poll_completed = vec![
        json!({ "role": "user", "display_text": "prompt" }),
        json!({ "role": "assistant", "display_text": "Intro: 3 scenes (30s total)" }),
        json!({ "role": "assistant", "display_text": "Scene 1: Persian Gulf overview (10s)" }),
        json!({ "role": "assistant", "display_text": "Scene 2: Naval formation (10s)" }),
        json!({
            "role": "assistant",
            "status": "completed",
            "display_text": "Scene 3: Diplomatic summary (10s) [ALL_MEDIA_ATTACHED]"
        }),
        json!({
            "event_name": "delta.presentation",
            "payload": {
                "kind": "video",
                "data": {
                    "videos": [
                        { "path": "sandbox:///home/user/clip_1.mp4" },
                        { "path": "sandbox:///home/user/clip_2.mp4" },
                        { "path": "sandbox:///home/user/clip_3.mp4" }
                    ]
                }
            }
        }),
    ];

    let turn = FakeStreamTurn::media_reply(
        vec![],
        vec![poll_running, poll_completed],
        vec![json!({
            "event": "approval.requested",
            "payload": {
                "approval_id": "app-bash-1",
                "command": "curl https://example.com/upload"
            }
        })],
    );

    let acc = FakeVmAccount::new(vec![turn])
        .with_file("/home/user/clip_1.mp4", clip1_bytes.clone())
        .with_file("/home/user/clip_2.mp4", clip2_bytes.clone())
        .with_file("/home/user/clip_3.mp4", clip3_bytes.clone());
    let transport = FakeMuseVmTransport::new(vec![Arc::clone(&acc)]);
    let provider = build_gateway_muse_provider(&[SINGLE_COOKIE], transport).await;

    let output = provider
        .invoke(LlmOperation::VideoGeneration {
            prompt: "Iran war overview in 3 scenes".to_string(),
            width: 1280,
            height: 720,
            duration_secs: 30,
            first_frame: None,
        })
        .await
        .expect("multi-clip video generation succeeds");

    assert_eq!(
        acc.decided_approvals(),
        vec![("app-bash-1".to_string(), "deny".to_string())],
        "external curl/bash approval must be auto-denied"
    );

    assert_eq!(
        output,
        LlmOutput::from_media_parts(
            vec![
                MediaPart {
                    bytes: clip1_bytes,
                    mime_type: "video/mp4".to_string(),
                },
                MediaPart {
                    bytes: clip2_bytes,
                    mime_type: "video/mp4".to_string(),
                },
                MediaPart {
                    bytes: clip3_bytes,
                    mime_type: "video/mp4".to_string(),
                },
            ],
            Some(
                "Intro: 3 scenes (30s total)\n\nScene 1: Persian Gulf overview (10s)\n\nScene 2: Naval formation (10s)\n\nScene 3: Diplomatic summary (10s)"
                    .to_string()
            ),
        )
        .unwrap()
    );
}

#[tokio::test]
async fn video_generation_selects_final_merged_group_when_raw_clips_precede_merged_output() {
    let raw1 = [VALID_MP4_BYTES, &[0xA1; 4]].concat();
    let raw2 = [VALID_MP4_BYTES, &[0xA2; 4]].concat();
    let merged = [VALID_MP4_BYTES, &[0xFF; 8]].concat();

    let history = vec![
        json!({ "role": "user", "display_text": "prompt" }),
        json!({
            "event_name": "delta.presentation",
            "payload": {
                "kind": "video",
                "data": {
                    "videos": [
                        { "path": "/home/user/raw_1.mp4" },
                        { "path": "/home/user/raw_2.mp4" }
                    ]
                }
            }
        }),
        json!({
            "event_name": "delta.presentation",
            "payload": {
                "kind": "video",
                "video_path": "/home/user/final_merged.mp4"
            }
        }),
        json!({
            "role": "assistant",
            "status": "completed",
            "display_text": "Merged 20s video ready [ALL_MEDIA_ATTACHED]"
        }),
    ];

    let turn = FakeStreamTurn::media_reply(vec![], vec![history], vec![]);
    let acc = FakeVmAccount::new(vec![turn])
        .with_file("/home/user/raw_1.mp4", raw1)
        .with_file("/home/user/raw_2.mp4", raw2)
        .with_file("/home/user/final_merged.mp4", merged.clone());
    let transport = FakeMuseVmTransport::new(vec![acc]);
    let provider = build_gateway_muse_provider(&[SINGLE_COOKIE], transport).await;

    let output = provider
        .invoke(LlmOperation::VideoGeneration {
            prompt: "20s cinematic trailer".to_string(),
            width: 1280,
            height: 720,
            duration_secs: 20,
            first_frame: None,
        })
        .await
        .expect("merged video generation succeeds");

    assert_eq!(
        output,
        LlmOutput::from_media_parts(
            vec![MediaPart {
                bytes: merged,
                mime_type: "video/mp4".to_string(),
            }],
            Some("Merged 20s video ready".to_string()),
        )
        .unwrap()
    );
}

#[tokio::test]
async fn pool_rotates_to_second_account_on_quota_or_auth_failure() {
    let acc1 = FakeVmAccount::new(vec![FakeStreamTurn::chat_reply(
        vec!["You have reached your daily limit. Please try again tomorrow."],
        "You have reached your daily limit. Please try again tomorrow.",
    )]);
    let acc2 = FakeVmAccount::new(vec![FakeStreamTurn::chat_reply(
        vec!["Phản hồi từ tài khoản #2"],
        "Phản hồi từ tài khoản #2",
    )]);

    let transport = FakeMuseVmTransport::new(vec![acc1, acc2]);
    let provider =
        build_gateway_muse_provider(&[SINGLE_COOKIE, SECOND_COOKIE], transport).await;

    let req = LlmRequest::text_turn("Kiểm tra failover".to_string());
    let reply = provider
        .invoke(LlmOperation::Chat {
            request: req,
            reasoning: false,
        })
        .await
        .expect("pool rotates to second account on quota limit");
    assert_eq!(reply, LlmOutput::Text("Phản hồi từ tài khoản #2".to_string()));
}

#[tokio::test]
async fn chat_stream_waits_for_in_progress_assistant_and_reconciles_full_history_text() {
    let full_turn1_part1 = "# Mesut Özil — Tổng quan sự nghiệp & hoạt động sau giải nghệ\n\n**Mesut Özil** (sinh 15/10/1988 tại Gelsenkirchen) là cựu tiền vệ tấn công người Đức gốc Thổ Nhĩ Kỳ.";
    let full_turn1_part2 =
        "1. **Sự nghiệp**: Schalke 04, Werder Bremen, Real Madrid, Arsenal, Fenerbahçe, İstanbul Başakşehir.\n2. **Danh hiệu**: Vô địch World Cup 2014 cùng đội tuyển Đức.";
    let expected_turn1 = format!("{full_turn1_part1}\n\n{full_turn1_part2}");

    // Turn 1 reproduces all 3 failure modes from job `cccd1051-945f-4d2b-a609-1ffb8f28addb`:
    // 1. `/chat/subscribe` frames wrap events as `{"type": "event", "event": "sessions.updated" | "agent.status", ...}`.
    // 2. `/chat/history` writes the initial few chars (`"# Mesut Öz"`, without `status: "completed"`) and returns
    //    that same partial snapshot across consecutive polls while the VM generates the rest of the report.
    //    The workflow must treat an uncompleted `message.assistant` in `turn_events` as still working and wait
    //    until `status: "completed"` arrives with the full text.
    // 3. Turn 1 includes a `ChatMessage::System` prompt so `/chat/history` records the flattened
    //    `[System]\n...\n\n[User]\n...` text in `message.user`.
    let flattened_turn1_user =
        "[System]\nTrả lời chi tiết kèm nguồn.\n\n[User]\nHãy tìm thông tin về Mesut Özil";
    let in_progress_poll = vec![
        json!({
            "event_name": "message.user",
            "payload": { "display_text": flattened_turn1_user }
        }),
        json!({
            "event_name": "message.assistant",
            "payload": { "display_text": "# Mesut Öz" }
        }),
    ];

    let turn1 = FakeStreamTurn {
        status: 200,
        assigned_session_id: Some("sess-ozil-1".to_string()),
        events: vec![
            json!({
                "type": "event",
                "event": "sessions.updated",
                "payload": {
                    "change": "updated",
                    "session": {
                        "session_id": "sess-ozil-1",
                        "status": "running"
                    }
                }
            }),
            json!({
                "type": "event",
                "event": "agent.status",
                "payload": {
                    "chat_context": { "chat_id": "sess-ozil-1" },
                    "status": "online",
                    "activity_code": "working"
                }
            }),
        ],
        history_polls: VecDeque::from(vec![
            in_progress_poll.clone(),
            in_progress_poll.clone(),
            in_progress_poll,
            vec![
                json!({
                    "event_name": "message.user",
                    "payload": { "display_text": flattened_turn1_user }
                }),
                json!({
                    "event_name": "message.assistant",
                    "payload": {
                        "status": "completed",
                        "display_text": full_turn1_part1
                    }
                }),
                json!({
                    "event_name": "tool_group",
                    "payload": {
                        "status": "completed",
                        "data": { "display_state": "done" }
                    }
                }),
                json!({
                    "event_name": "message.assistant",
                    "payload": {
                        "status": "completed",
                        "content": [{ "type": "text", "text": full_turn1_part2 }]
                    }
                }),
            ],
        ]),
    };

    // Turn 2 tests follow-up stale history protection (`is_stale_followup_history`) when Turn 1's user
    // message in `/chat/history` was flattened with `[System]\n...`:
    // - Poll 1 still returns Turn 1's completed history (only 1 `message.user` event < `expected_user_turns == 2`)
    // - Poll 2 returns Turn 2's `message.user` and completed `message.assistant`
    let turn2_reply = "Dưới đây là bản tổng hợp chi tiết về Mesut Özil kèm nguồn tham khảo.";
    let turn2 = FakeStreamTurn {
        status: 200,
        assigned_session_id: Some("sess-ozil-1".to_string()),
        events: vec![json!({
            "type": "event",
            "event": "sessions.updated",
            "payload": {
                "change": "updated",
                "session": {
                    "session_id": "sess-ozil-1",
                    "status": "completed"
                }
            }
        })],
        history_polls: VecDeque::from(vec![
            vec![
                json!({
                    "event_name": "message.user",
                    "payload": { "display_text": flattened_turn1_user }
                }),
                json!({
                    "event_name": "message.assistant",
                    "payload": {
                        "status": "completed",
                        "display_text": full_turn1_part1
                    }
                }),
            ],
            vec![
                json!({
                    "event_name": "message.user",
                    "payload": { "display_text": flattened_turn1_user }
                }),
                json!({
                    "event_name": "message.assistant",
                    "payload": {
                        "status": "completed",
                        "display_text": full_turn1_part1
                    }
                }),
                json!({
                    "event_name": "message.user",
                    "payload": { "display_text": "Kết quả trước không hiển thị. Hãy thực hiện lại." }
                }),
                json!({
                    "event_name": "message.assistant",
                    "payload": {
                        "status": "completed",
                        "display_text": turn2_reply
                    }
                }),
            ],
        ]),
    };

    let acc = FakeVmAccount::new(vec![turn1, turn2]);
    let transport = FakeMuseVmTransport::new(vec![Arc::clone(&acc)]);
    let provider = build_gateway_muse_provider(&[SINGLE_COOKIE], transport).await;

    // Verify Turn 1 streams both the initial "# Mesut Öz" and the reconciled suffix from `/chat/history`
    let req1 = LlmRequest {
        messages: vec![
            ChatMessage::System {
                content: "Trả lời chi tiết kèm nguồn.".to_string(),
            },
            ChatMessage::User {
                content: "Hãy tìm thông tin về Mesut Özil".to_string(),
            },
        ],
        tools: Vec::new(),
    };
    let mut stream = provider.stream(req1).await.expect("open turn 1 stream");
    let mut deltas = Vec::new();
    while let Some(item) = stream.next().await {
        match item.expect("turn 1 stream event") {
            LlmEvent::TextDelta(d) => deltas.push(d),
            LlmEvent::Finished => break,
            _ => {}
        }
    }
    assert_eq!(deltas.concat(), expected_turn1);

    // Verify Turn 2 skips stale Turn 1 history (even though Turn 1 was flattened with `[System]`)
    // and returns Turn 2's full response
    let req2 = LlmRequest {
        messages: vec![
            ChatMessage::System {
                content: "Trả lời chi tiết kèm nguồn.".to_string(),
            },
            ChatMessage::User {
                content: "Hãy tìm thông tin về Mesut Özil".to_string(),
            },
            ChatMessage::AssistantText {
                content: expected_turn1,
            },
            ChatMessage::User {
                content: "Kết quả trước không hiển thị. Hãy thực hiện lại.".to_string(),
            },
        ],
        tools: Vec::new(),
    };
    let reply2 = provider
        .invoke(LlmOperation::Chat {
            request: req2,
            reasoning: false,
        })
        .await
        .expect("turn 2 invoke");
    assert_eq!(reply2, LlmOutput::Text(turn2_reply.to_string()));
}
