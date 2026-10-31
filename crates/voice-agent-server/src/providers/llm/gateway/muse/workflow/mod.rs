mod chat;
mod media;
mod png;
mod prompt;

use serde_json::Value;

use super::client::VmStreamEvent;
use super::pool::{UpstreamFailureKind, VmOperationLease};

#[derive(Debug, Clone)]
pub(crate) enum WorkflowFailure {
    Rejected(String),
    Upstream(UpstreamFailureKind, String),
    Timeout(String),
}

impl WorkflowFailure {
    pub fn into_llm_error(self) -> crate::providers::llm::LlmError {
        match self {
            WorkflowFailure::Rejected(reason) => {
                tracing::debug!(reason = %reason, "Muse workflow rejected request");
                crate::providers::llm::LlmError::Rejected
            }
            WorkflowFailure::Upstream(kind, reason) => {
                tracing::debug!(?kind, reason = %reason, "Muse workflow upstream failure");
                crate::providers::llm::LlmError::Failed
            }
            WorkflowFailure::Timeout(reason) => {
                tracing::debug!(reason = %reason, "Muse workflow timed out");
                crate::providers::llm::LlmError::Failed
            }
        }
    }
}

pub(crate) const MEDIA_DONE_MARKER: &str = "[ALL_MEDIA_ATTACHED]";
pub(crate) const MUSE_SINGLE_CLIP_LIMIT_SECS: u32 = 10;
pub(crate) const MUSE_MAX_CLIPS_PER_TURN: u32 = 6;
pub(crate) const CUTOUT_HINT: &str = "\n\nBackground requirement: Render the subject isolated on a solid, uniform, flat pure-white (#FFFFFF) studio background with clean, crisp edges and no background shadows or gradients so it can be cleanly cut out.";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MediaKind {
    Image,
    Video,
}

pub(crate) const QUOTA_HINTS: &[&str] = &[
    "rate limit",
    "quota exceeded",
    "too many requests",
    "reached your limit",
    "daily limit",
    "usage limit",
    "try again tomorrow",
];

pub(crate) const GLITCH_HINTS: &[&str] = &[
    "hit a glitch",
    "try again in a moment",
    "something went wrong",
    "temporarily unavailable",
];

pub(crate) fn is_glitch_reply(text: &str) -> bool {
    let lower = text.trim().to_ascii_lowercase();
    lower.len() <= 240 && GLITCH_HINTS.iter().any(|h| lower.contains(h))
}

pub(crate) fn is_quota_reply(text: &str) -> bool {
    let lower = text.trim().to_ascii_lowercase();
    lower.len() <= 320 && QUOTA_HINTS.iter().any(|h| lower.contains(h))
}

pub(crate) fn classify_assistant_error(text: &str) -> Option<(UpstreamFailureKind, String)> {
    if is_quota_reply(text) {
        return Some((
            UpstreamFailureKind::QuotaExceeded,
            format!("Muse quota exceeded: {}", text.trim()),
        ));
    }
    if is_glitch_reply(text) {
        return Some((
            UpstreamFailureKind::Transient,
            format!("Muse upstream glitch reply: {}", text.trim()),
        ));
    }
    None
}

pub(crate) fn extract_history_turn_events(history_json: &Value) -> Vec<Value> {
    let evs = history_json
        .get("result")
        .and_then(|r| r.get("chat_events"))
        .or_else(|| history_json.get("chat_events"))
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();

    let last_user_idx = evs.iter().rposition(|e| {
        let p = e.get("payload").unwrap_or(e);
        e.get("role").and_then(Value::as_str) == Some("user")
            || p.get("role").and_then(Value::as_str) == Some("user")
            || e.get("event_name").and_then(Value::as_str) == Some("message.user")
            || e.get("event").and_then(Value::as_str) == Some("message.user")
    });

    match last_user_idx {
        Some(idx) if idx < evs.len() => evs[idx + 1..].to_vec(),
        _ => Vec::new(),
    }
}

pub(crate) fn extract_value_text(val: &Value) -> Option<String> {
    if let Some(s) = val.as_str() {
        let trimmed = s.trim();
        if !trimmed.is_empty() {
            return Some(trimmed.to_string());
        }
        return None;
    }
    if let Some(arr) = val.as_array() {
        let mut combined = String::new();
        for item in arr {
            if let Some(s) = item.as_str() {
                combined.push_str(s);
            } else if let Some(s) = item
                .get("text")
                .or_else(|| item.get("content"))
                .and_then(Value::as_str)
            {
                combined.push_str(s);
            }
        }
        let trimmed = combined.trim();
        if !trimmed.is_empty() {
            return Some(trimmed.to_string());
        }
    }
    None
}

pub(crate) fn has_active_tool_task(turn_events: &[Value]) -> bool {
    turn_events.iter().any(|ev| {
        let p = ev.get("payload").unwrap_or(ev);
        let d = p.get("data").or_else(|| ev.get("data"));
        let s = d
            .and_then(|x| x.get("status"))
            .or_else(|| p.get("status"))
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_ascii_lowercase();
        let ds = d
            .and_then(|x| x.get("display_state").or_else(|| x.get("display_status")))
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_ascii_lowercase();
        matches!(s.as_str(), "running" | "queued" | "in_progress" | "pending")
            || matches!(ds.as_str(), "working" | "starting")
    })
}

pub(crate) fn extract_turn_assistant_messages(turn_events: &[Value]) -> Vec<String> {
    let mut blocks = Vec::new();
    for ev in turn_events {
        let ev_name = ev
            .get("event_name")
            .or_else(|| ev.get("event"))
            .and_then(Value::as_str)
            .unwrap_or("");
        if ev_name == "delta.presentation" {
            continue;
        }
        let p = ev.get("payload").unwrap_or(ev);
        let d = p.get("data").or_else(|| ev.get("data"));
        if ev.get("kind").and_then(Value::as_str) == Some("browser_task")
            || p.get("kind").and_then(Value::as_str) == Some("browser_task")
            || d.and_then(|x| x.get("browser_task_id")).is_some()
        {
            continue;
        }
        let is_assistant = ev.get("role").and_then(Value::as_str) == Some("assistant")
            || p.get("role").and_then(Value::as_str) == Some("assistant")
            || ev_name == "message.assistant";
        if !is_assistant {
            continue;
        }
        let text = ev
            .get("display_text")
            .and_then(extract_value_text)
            .or_else(|| p.get("display_text").and_then(extract_value_text))
            .or_else(|| p.get("content").and_then(extract_value_text))
            .or_else(|| ev.get("content").and_then(extract_value_text))
            .or_else(|| ev.get("text").and_then(extract_value_text))
            .or_else(|| p.get("text").and_then(extract_value_text))
            .unwrap_or_default();
        if !text.is_empty() && !blocks.iter().any(|b| b == &text) {
            blocks.push(text);
        }
    }
    blocks
}

fn is_commentary_assistant_event(ev: &Value) -> bool {
    let p = ev.get("payload").unwrap_or(ev);
    let meta = p.get("metadata").or_else(|| ev.get("metadata"));
    meta.and_then(|m| m.get("message_kind"))
        .and_then(Value::as_str)
        == Some("commentary")
        || meta
            .and_then(|m| m.get("recipient"))
            .and_then(Value::as_str)
            == Some("commentary")
}

pub(crate) fn is_last_assistant_completed(turn_events: &[Value]) -> bool {
    let assistant_events: Vec<&Value> = turn_events
        .iter()
        .filter(|ev| {
            let ev_name = ev
                .get("event_name")
                .or_else(|| ev.get("event"))
                .and_then(Value::as_str)
                .unwrap_or("");
            if ev_name == "delta.presentation" {
                return false;
            }
            let p = ev.get("payload").unwrap_or(ev);
            let d = p.get("data").or_else(|| ev.get("data"));
            if ev.get("kind").and_then(Value::as_str) == Some("browser_task")
                || p.get("kind").and_then(Value::as_str) == Some("browser_task")
                || d.and_then(|x| x.get("browser_task_id")).is_some()
            {
                return false;
            }
            ev.get("role").and_then(Value::as_str) == Some("assistant")
                || p.get("role").and_then(Value::as_str) == Some("assistant")
                || ev_name == "message.assistant"
        })
        .collect();

    if assistant_events.is_empty() {
        return false;
    }

    let any_in_progress = assistant_events.iter().any(|ev| {
        let p = ev.get("payload").unwrap_or(ev);
        let status = ev
            .get("status")
            .or_else(|| p.get("status"))
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_ascii_lowercase();
        matches!(
            status.as_str(),
            "in_progress" | "running" | "pending" | "streaming"
        )
    });
    if any_in_progress {
        return false;
    }

    assistant_events.last().is_some_and(|ev| {
        if is_commentary_assistant_event(ev) {
            return false;
        }
        let p = ev.get("payload").unwrap_or(ev);
        ev.get("status").and_then(Value::as_str) == Some("completed")
            || p.get("status").and_then(Value::as_str) == Some("completed")
    })
}

pub(crate) async fn handle_approval_event<F>(
    lease: &VmOperationLease,
    ev: &VmStreamEvent,
    mut should_approve: F,
) where
    F: FnMut(&Value) -> bool,
{
    let p = ev.payload();
    if let Some(app_id) = p
        .get("approval_id")
        .or_else(|| p.get("approvalId"))
        .and_then(Value::as_str)
    {
        let allow = should_approve(p);
        let _ = lease.client().decide_approval(app_id, allow).await;
    }
    if let Some(arr) = p
        .get("pending_approvals")
        .or_else(|| p.get("pending"))
        .and_then(Value::as_array)
    {
        for item in arr {
            if let Some(app_id) = item
                .get("approval_id")
                .or_else(|| item.get("approvalId"))
                .and_then(Value::as_str)
            {
                let allow = should_approve(item);
                let _ = lease.client().decide_approval(app_id, allow).await;
            }
        }
    }
}

pub(crate) use self::chat::run_chat_turn_once;
pub(crate) use self::media::run_media_turn_once;
#[cfg(test)]
pub(crate) use self::png::{
    decode_uncompressed_or_simple_png, encode_rgba_png, remove_background_to_rgba_png,
};
#[cfg(test)]
pub(crate) use self::prompt::flatten_messages;
pub(crate) use self::prompt::{format_image_prompt, format_video_prompt};
