use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use base64::Engine as _;
use serde_json::{Value, json};
use tokio::sync::mpsc;

use super::super::client::{MuseVmReceiver, MuseVmSender, MuseVmTransport, VmError};
use super::super::proto::{
    DecodedApplicationResponse, DecodedBodyChunk, DecodedServiceFrame, NoiseFrameDecoder,
    decode_service_request_frame,
};
pub(in crate::providers::llm::gateway) use super::fixtures::{
    FakeStreamTurn, SECOND_COOKIE, SINGLE_COOKIE, VALID_MP4_BYTES, VALID_PNG_BYTES,
};
pub(super) use super::fixtures::make_test_rgba_png;
use crate::config::DatabaseConfig;
use crate::database::Database;
use crate::database::credentials::CredentialCipher;
use crate::providers::llm::gateway::{LlmGatewayConfig, LlmGatewayProvider};
use crate::services::llm_gateway::{
    FakeOAuthVendorDriver, GatewayBinding, GatewayVendor, LlmGatewayHub, OAuthCallbackSubmission,
    OAuthVendorDriver, StartOAuthLoginRequest,
};

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
        let clean = path
            .trim()
            .trim_start_matches("sandbox://")
            .trim_start_matches("/home/user/")
            .trim_start_matches('/')
            .to_string();
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
                let raw_path = body_json
                    .get("path")
                    .and_then(Value::as_str)
                    .unwrap_or("");
                if raw_path.starts_with('/') {
                    send_json_response(
                        &self.tx,
                        stream_id,
                        400,
                        &json!({
                            "ok": false,
                            "error": {
                                "code": "invalid_params",
                                "message": "absolute paths are not allowed"
                            }
                        }),
                    );
                    continue;
                }
                let file_path = raw_path.trim_start_matches("sandbox://");
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

pub(super) async fn build_gateway_muse_provider(
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
