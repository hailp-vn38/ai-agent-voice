use std::sync::atomic::Ordering;
use std::time::Duration;

use base64::Engine as _;
use serde_json::{Value, json};

use super::super::proto::{SERVICE_DAEMON, SERVICE_SENTINEL};
use super::MuseVmClient;
use super::types::{ISOLATION_POLICY_MARKDOWN, VmError};

const READ_CHUNK_SIZE: usize = 131_072; // 128 KiB per /fs/read call
const WRITE_CHUNK_SIZE: usize = 262_144; // 256 KiB per /fs/write call
const REPLAY_INITIAL_WAIT: Duration = Duration::from_millis(20);
const REPLAY_QUIET_WINDOW: Duration = Duration::from_millis(350);
const REPLAY_MAX_SETTLE: Duration = Duration::from_millis(1500);

impl MuseVmClient {
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
        let clean_path = path
            .trim()
            .trim_start_matches("sandbox://")
            .trim_start_matches("/home/user/")
            .trim_start_matches('/')
            .to_string();
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
