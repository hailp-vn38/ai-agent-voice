use super::super::super::client::VmStreamEvent;
use super::super::{
    extract_history_turn_events, extract_turn_assistant_messages, has_active_tool_task,
    is_last_assistant_completed,
};
use serde_json::Value;
use std::time::Duration;
use tokio::time::Instant;

#[derive(Debug, PartialEq, Eq)]
pub(super) enum TurnStatus {
    Working,
    Done,
}

pub(super) struct TurnTracker {
    expected: u32,
    last_signal: Instant,
    fingerprint: String,
    stable_since: Option<Instant>,
    done: bool,
    partial: Option<String>,
    session_completed: bool,
}

pub(super) fn user_turns(history: &Value) -> Option<u32> {
    let events = history
        .get("result")
        .unwrap_or(history)
        .get("chat_events")?
        .as_array()?;
    Some(
        events
            .iter()
            .filter(|event| {
                let payload = event.get("payload").unwrap_or(event);
                event
                    .get("role")
                    .or_else(|| payload.get("role"))
                    .and_then(Value::as_str)
                    == Some("user")
                    || event
                        .get("event_name")
                        .or_else(|| event.get("event"))
                        .and_then(Value::as_str)
                        == Some("message.user")
            })
            .count() as u32,
    )
}

impl TurnTracker {
    pub fn new(expected_user_turns: u32, started_at: Instant) -> Self {
        Self {
            expected: expected_user_turns,
            last_signal: started_at,
            fingerprint: String::new(),
            stable_since: None,
            done: false,
            partial: None,
            session_completed: false,
        }
    }
    pub fn on_event(&mut self, event: &VmStreamEvent, now: Instant) {
        if event.event_name().starts_with("delta.")
            || event.event_name() == "agent.status"
            || event.event_name().starts_with("task.")
        {
            self.last_signal = now;
        }
        if event.event_name() == "sessions.updated" {
            self.session_completed = event
                .payload()
                .get("session")
                .and_then(|s| s.get("status"))
                .and_then(Value::as_str)
                == Some("completed");
        }
    }
    pub fn on_history(&mut self, history: &Value, now: Instant) {
        let fingerprint = history.to_string();
        if fingerprint != self.fingerprint {
            self.last_signal = now;
            self.fingerprint = fingerprint;
            self.stable_since = None;
        }
        self.done = false;
        if user_turns(history) != Some(self.expected) {
            self.stable_since = None;
            return;
        }
        let events = extract_history_turn_events(history);
        let messages = extract_turn_assistant_messages(
            &events
                .iter()
                .filter(|event| {
                    let payload = event.get("payload").unwrap_or(event);
                    let metadata = payload.get("metadata").or_else(|| event.get("metadata"));
                    !["message_kind", "recipient"].iter().any(|key| {
                        metadata.and_then(|m| m.get(key)).and_then(Value::as_str)
                            == Some("commentary")
                    })
                })
                .cloned()
                .collect::<Vec<_>>(),
        );
        let text = messages.join("\n\n");
        self.partial = (!text.is_empty()).then_some(text.clone());
        if has_active_tool_task(&events) || !is_last_assistant_completed(&events) {
            self.stable_since = None;
            return;
        }
        if text.contains("[MUSE_TASK_DONE]") {
            self.done = true;
            return;
        }
        let completed = self.session_completed
            || history
                .get("result")
                .unwrap_or(history)
                .get("status")
                .and_then(Value::as_str)
                == Some("completed");
        if completed {
            self.stable_since.get_or_insert(now);
        } else {
            self.stable_since = None;
        }
    }
    pub fn status(&self, now: Instant) -> TurnStatus {
        if self.done
            || self
                .stable_since
                .is_some_and(|since| now.saturating_duration_since(since) >= Duration::from_secs(5))
        {
            TurnStatus::Done
        } else {
            TurnStatus::Working
        }
    }
    pub fn partial_text(&self) -> Option<String> {
        self.partial
            .as_ref()
            .map(|s| s.replace("[MUSE_TASK_DONE]", "").trim().to_owned())
    }
    pub fn last_signal_at(&self) -> Instant {
        self.last_signal
    }
}

#[cfg(test)]
#[path = "tracker_tests.rs"]
mod tests;
