mod dispatch;
mod ops;
mod transport;
mod types;

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::sync::{Mutex as AsyncMutex, broadcast, oneshot};

use self::dispatch::{PendingStream, handle_decoded_frame};
#[cfg(test)]
pub(crate) use self::transport::MuseVmReceiver;
pub(crate) use self::transport::{MuseVmSender, MuseVmTransport, WsNoiseVmTransport};
pub(crate) use self::types::{RpcResponse, VmError, VmStreamEvent};
use super::proto::{
    encode_application_request, encode_service_frame, encode_service_request,
    split_into_transport_frames,
};

const PING_INTERVAL: Duration = Duration::from_secs(25);

/// Multiplexed RPC & real-time event client over a single encrypted Hatch VM Noise session.
pub(crate) struct MuseVmClient {
    next_stream_id: AtomicU64,
    connected: Arc<AtomicBool>,
    pub(super) policy_written: AtomicBool,
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
}
