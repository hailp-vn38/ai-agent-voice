use std::{
    sync::{Arc, Mutex},
    time::Duration,
};

use serde_json::{Value, json};
use tokio::{sync::mpsc, time::Instant};

use crate::{
    providers::llm::{LlmError, LlmEvent},
    services::llm_gateway::GatewayLease,
};

pub(crate) type ErrorDetailSlot = Arc<Mutex<Option<String>>>;

pub(crate) struct ActivityThrottle {
    last_emitted: Option<Instant>,
}

impl ActivityThrottle {
    pub(crate) fn new() -> Self {
        Self { last_emitted: None }
    }

    pub(crate) fn maybe_emit(&mut self, tx: &mpsc::UnboundedSender<Result<LlmEvent, LlmError>>) {
        let now = Instant::now();
        let should_emit = match self.last_emitted {
            None => true,
            Some(prev) => now.duration_since(prev) >= Duration::from_secs(1),
        };
        if should_emit {
            self.last_emitted = Some(now);
            let _ = tx.send(Ok(LlmEvent::Activity));
        }
    }
}

pub(crate) fn new_error_detail_slot() -> ErrorDetailSlot {
    Arc::new(Mutex::new(None))
}

pub(crate) fn record_wire_detail(slot: &ErrorDetailSlot, detail: impl Into<String>) {
    let text = detail.into();
    let trimmed = text.trim();
    if !trimmed.is_empty() {
        *slot.lock().unwrap_or_else(|p| p.into_inner()) = Some(trimmed.to_owned());
    }
}

/// Drains a single `\n`-terminated SSE line from a raw byte buffer and decodes it as UTF-8.
/// Buffering raw bytes until `b'\n'` (`0x0A`) ensures multi-byte UTF-8 characters (such as
/// Vietnamese diacritics or `Ö`) split across HTTP/2 DATA frames never fail UTF-8 decoding.
pub(crate) fn drain_next_sse_line(raw_buf: &mut Vec<u8>) -> Option<String> {
    let newline_pos = raw_buf.iter().position(|&b| b == b'\n')?;
    let line_bytes: Vec<u8> = raw_buf.drain(..=newline_pos).collect();
    Some(String::from_utf8_lossy(&line_bytes).trim().to_owned())
}

pub(crate) fn summarize_error_body(raw: &str) -> String {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return "empty error body".to_owned();
    }
    if let Ok(val) = serde_json::from_str::<Value>(trimmed)
        && let Some(extracted) = extract_sse_error_detail(&val)
    {
        return extracted;
    }
    let single_line: String = trimmed.split_whitespace().collect::<Vec<_>>().join(" ");
    if single_line.chars().count() > 200 {
        let head: String = single_line.chars().take(200).collect();
        format!("{head}...")
    } else {
        single_line
    }
}

pub(crate) fn extract_sse_error_detail(val: &Value) -> Option<String> {
    if let Some(err_obj) = val
        .get("error")
        .or_else(|| val.get("response").and_then(|r| r.get("error")))
    {
        if let Some(s) = err_obj.as_str()
            && !s.trim().is_empty()
        {
            return Some(s.trim().to_owned());
        }
        if err_obj.is_object() {
            let code = err_obj
                .get("code")
                .and_then(Value::as_str)
                .or_else(|| err_obj.get("type").and_then(Value::as_str))
                .unwrap_or("error");
            let msg = err_obj
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("upstream reported error");
            return Some(format!("{code}: {msg}"));
        }
    }
    if let Some(inc) = val.get("incomplete_details").or_else(|| {
        val.get("response")
            .and_then(|r| r.get("incomplete_details"))
    }) && let Some(reason) = inc.get("reason").and_then(Value::as_str)
        && !reason.trim().is_empty()
    {
        return Some(format!("incomplete response (reason={})", reason.trim()));
    }
    if let Some(msg) = val.get("message").and_then(Value::as_str)
        && !msg.trim().is_empty()
    {
        let code = val.get("code").and_then(Value::as_str).unwrap_or("error");
        return Some(format!("{code}: {}", msg.trim()));
    }
    None
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum NoLeaseKind {
    CoolingDown,
    NeedsReauth,
    Unavailable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WireError {
    NoLease(NoLeaseKind),
    RateLimited { retry_after_secs: Option<u64> },
    Unauthorized,
    HttpRejected,
    HttpUnavailable,
    Other,
}

impl WireError {
    pub(crate) fn into_llm_error(self) -> LlmError {
        match self {
            Self::RateLimited { retry_after_secs } => LlmError::RateLimited { retry_after_secs },
            Self::NoLease(NoLeaseKind::CoolingDown) => LlmError::RateLimited {
                retry_after_secs: None,
            },
            Self::NoLease(NoLeaseKind::NeedsReauth) | Self::Unauthorized => LlmError::Unauthorized,
            Self::NoLease(NoLeaseKind::Unavailable) | Self::HttpRejected => LlmError::Rejected,
            Self::HttpUnavailable => LlmError::Transient,
            Self::Other => LlmError::Failed,
        }
    }
}

pub(crate) fn apply_lease_headers(
    mut builder: reqwest::RequestBuilder,
    lease: &GatewayLease,
) -> reqwest::RequestBuilder {
    builder = builder.bearer_auth(lease.access_token.expose());
    for (k, v) in &lease.extra_headers {
        builder = builder.header(k, v);
    }
    builder
}

pub(crate) fn check_wire_status(resp: reqwest::Response) -> Result<reqwest::Response, WireError> {
    let status = resp.status();
    if status.as_u16() == 429 {
        let retry_after = resp
            .headers()
            .get("retry-after")
            .and_then(|h| h.to_str().ok())
            .and_then(|s| s.trim().parse::<u64>().ok());
        return Err(WireError::RateLimited {
            retry_after_secs: retry_after,
        });
    }
    if matches!(status.as_u16(), 401 | 403) {
        return Err(WireError::Unauthorized);
    }
    if status.as_u16() == 408 {
        return Err(WireError::Other);
    }
    if status.is_server_error() {
        return Err(WireError::HttpUnavailable);
    }
    if !status.is_success() {
        return Err(WireError::HttpRejected);
    }
    Ok(resp)
}

pub(crate) fn parse_tool_arguments(raw: &str) -> Result<Value, LlmError> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Ok(json!({}));
    }
    serde_json::from_str(trimmed).map_err(|_| LlmError::Failed)
}
