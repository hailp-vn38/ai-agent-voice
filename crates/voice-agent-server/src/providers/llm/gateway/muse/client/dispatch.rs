use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use serde_json::Value;
use tokio::sync::{broadcast, oneshot};

use super::super::proto::DecodedServiceFrame;
use super::types::{RpcResponse, VmError, VmStreamEvent};

pub(super) struct PendingStream {
    pub status: u32,
    pub body_buf: Vec<u8>,
    pub line_buf: String,
    pub resolve_on_status: bool,
    pub tx: Option<oneshot::Sender<Result<RpcResponse, VmError>>>,
}

pub(super) fn handle_decoded_frame(
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
