use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use tokio::sync::{Mutex, mpsc, oneshot};
use tokio_tungstenite::{
    connect_async_with_config,
    tungstenite::{Message, protocol::WebSocketConfig},
};

use super::CdpError;

const CDP_MAX_WS_PAYLOAD_BYTES: usize = 256 * 1024 * 1024;

type PendingResponses = Arc<Mutex<HashMap<u64, oneshot::Sender<Result<Value, CdpError>>>>>;

pub(in crate::providers::llm::gateway::muse) struct CdpSession {
    next_id: AtomicU64,
    tx: mpsc::UnboundedSender<String>,
    pending: PendingResponses,
    closed: Arc<AtomicBool>,
}

impl CdpSession {
    pub async fn connect(ws_url: &str) -> Result<Arc<Self>, CdpError> {
        let mut ws_config = WebSocketConfig::default();
        ws_config.max_message_size = Some(CDP_MAX_WS_PAYLOAD_BYTES);
        ws_config.max_frame_size = Some(CDP_MAX_WS_PAYLOAD_BYTES);
        let (ws_stream, _) = connect_async_with_config(ws_url, Some(ws_config), false)
            .await
            .map_err(|err| {
                CdpError::BrowserUnavailable(format!("websocket connect {ws_url}: {err}"))
            })?;
        let (mut sink, mut stream) = ws_stream.split();
        let (tx, mut rx) = mpsc::unbounded_channel::<String>();
        let pending: PendingResponses = Arc::new(Mutex::new(HashMap::new()));
        let closed = Arc::new(AtomicBool::new(false));

        let closed_writer = Arc::clone(&closed);
        tokio::spawn(async move {
            while let Some(msg) = rx.recv().await {
                if sink.send(Message::Text(msg.into())).await.is_err() {
                    closed_writer.store(true, Ordering::SeqCst);
                    break;
                }
            }
        });

        let pending_reader = Arc::clone(&pending);
        let closed_reader = Arc::clone(&closed);
        tokio::spawn(async move {
            let mut close_reason = "cdp websocket closed".to_string();
            while let Some(frame) = stream.next().await {
                match frame {
                    Ok(Message::Text(text)) => {
                        if let Ok(val) = serde_json::from_str::<Value>(&text)
                            && let Some(id) = val.get("id").and_then(Value::as_u64)
                        {
                            let sender = pending_reader.lock().await.remove(&id);
                            if let Some(sender) = sender {
                                if let Some(err) = val.get("error") {
                                    let msg = err
                                        .get("message")
                                        .and_then(Value::as_str)
                                        .unwrap_or("cdp error")
                                        .to_string();
                                    let _ = sender.send(Err(CdpError::Protocol(msg)));
                                } else {
                                    let res = val.get("result").cloned().unwrap_or(Value::Null);
                                    let _ = sender.send(Ok(res));
                                }
                            }
                        }
                    }
                    Ok(Message::Close(_)) => break,
                    Err(err) => {
                        close_reason = format!("cdp websocket closed: {err}");
                        break;
                    }
                    _ => {}
                }
            }
            closed_reader.store(true, Ordering::SeqCst);
            let mut map = pending_reader.lock().await;
            for (_, sender) in map.drain() {
                let _ = sender.send(Err(CdpError::Protocol(close_reason.clone())));
            }
        });

        Ok(Arc::new(Self {
            next_id: AtomicU64::new(1),
            tx,
            pending,
            closed,
        }))
    }

    pub fn is_closed(&self) -> bool {
        self.closed.load(Ordering::SeqCst)
    }

    pub async fn send(
        &self,
        method: &str,
        params: Value,
        timeout: Duration,
    ) -> Result<Value, CdpError> {
        if self.is_closed() {
            return Err(CdpError::BrowserUnavailable(
                "cdp session is closed".to_string(),
            ));
        }
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = oneshot::channel();
        self.pending.lock().await.insert(id, tx);
        let payload = json!({
            "id": id,
            "method": method,
            "params": params,
        });
        if self.tx.send(payload.to_string()).is_err() {
            self.pending.lock().await.remove(&id);
            self.closed.store(true, Ordering::SeqCst);
            return Err(CdpError::BrowserUnavailable(
                "cdp writer channel closed".to_string(),
            ));
        }
        match tokio::time::timeout(timeout, rx).await {
            Ok(Ok(res)) => res,
            Ok(Err(_)) => Err(CdpError::Protocol(
                "cdp response channel dropped".to_string(),
            )),
            Err(_) => {
                self.pending.lock().await.remove(&id);
                Err(CdpError::Timeout)
            }
        }
    }
}
