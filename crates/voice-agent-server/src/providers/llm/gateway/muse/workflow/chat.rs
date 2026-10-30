use std::sync::Arc;
use std::time::Duration;

use base64::Engine as _;
use serde_json::{Value, json};
use tokio::time::Instant;

use super::super::client::VmStreamEvent;
use super::super::pool::{
    ConversationTurn, HotConversation, SharedMusePool, UpstreamFailureKind, VmOperationLease,
};
use super::super::proto::SERVICE_DAEMON;
use super::prompt::{extract_prior_turns_and_last_user, flatten_messages, followup_text};
use super::{
    WorkflowFailure, classify_assistant_error, extract_history_turn_events,
    extract_turn_assistant_messages, handle_approval_event, has_active_tool_task,
};
use crate::providers::llm::{LlmRequest, ReferenceImage};

const CHAT_POLL_INTERVAL: Duration = Duration::from_millis(350);

pub(crate) async fn run_chat_turn_once<F>(
    pool: &Arc<SharedMusePool>,
    request: &LlmRequest,
    images: &[ReferenceImage],
    timeout_duration: Duration,
    mut on_delta: F,
) -> Result<String, WorkflowFailure>
where
    F: FnMut(String) + Send,
{
    let flattened = flatten_messages(request);
    if flattened.trim().is_empty() {
        return Err(WorkflowFailure::Rejected("empty chat prompt".to_string()));
    }
    let (prior_turns, last_user) = match extract_prior_turns_and_last_user(request) {
        Some((turns, last)) => (turns, Some(last)),
        None => (Vec::new(), None),
    };

    let mut lease = pool
        .acquire_for_chat(&prior_turns)
        .await
        .map_err(|(kind, reason)| WorkflowFailure::Upstream(kind, reason))?;

    let res = execute_chat_turn_on_lease(
        &mut lease,
        &flattened,
        images,
        &prior_turns,
        last_user.as_deref(),
        &mut on_delta,
        timeout_duration,
    )
    .await;

    if let Err(WorkflowFailure::Upstream(kind, _)) = &res {
        lease.fail_account(*kind).await;
    }
    res
}

async fn execute_chat_turn_on_lease<F>(
    lease: &mut VmOperationLease,
    prompt: &str,
    images: &[ReferenceImage],
    prior_turns: &[ConversationTurn],
    last_user: Option<&str>,
    on_delta: &mut F,
    timeout_duration: Duration,
) -> Result<String, WorkflowFailure>
where
    F: FnMut(String) + Send,
{
    let (effective_prompt, images_to_send) = if let Some(hot) = lease.matched_hot()
        && let Some(lu) = last_user
        && !lu.is_empty()
        && prior_turns.len() >= hot.turns.len()
        && images.len() >= hot.image_count
    {
        (
            followup_text(&prior_turns[hot.turns.len()..], lu),
            &images[hot.image_count..],
        )
    } else {
        (prompt.to_string(), images)
    };

    let mut items = Vec::with_capacity(images_to_send.len() + 1);
    for img in images_to_send {
        items.push(json!({
            "type": "image",
            "data_base64": base64::engine::general_purpose::STANDARD.encode(&img.bytes),
            "mime_type": img.mime_type,
        }));
    }
    items.push(json!({
        "type": "text",
        "text": effective_prompt,
    }));

    let mut event_rx = lease.client().subscribe_events();
    let stream_body = json!({
        "items": items,
        "node_id": "root",
        "session_id": lease.session_id(),
        "capabilities": ["chat_cancel", "delta_stream"],
    });
    let init_resp = lease
        .client()
        .send_request(
            "POST",
            "/chat/stream",
            Some(stream_body.to_string().as_bytes()),
            SERVICE_DAEMON,
            Duration::from_secs(30),
            true,
        )
        .await?;
    if let Ok(init_json) = init_resp.json()
        && let Some(assigned_id) = init_json
            .get("session_id")
            .or_else(|| init_json.get("result").and_then(|r| r.get("session_id")))
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
    {
        lease.set_session_id(assigned_id.to_string());
    }

    let deadline = Instant::now() + timeout_duration;
    let mut poll_interval = tokio::time::interval(CHAT_POLL_INTERVAL);
    poll_interval.tick().await;

    let mut emitted_text = String::new();
    let mut explicit_done = false;
    let mut should_poll_history = false;

    while Instant::now() < deadline {
        tokio::select! {
            recv_res = event_rx.recv() => {
                if let Ok(ev) = recv_res {
                    let sid_matches = ev
                        .session_id()
                        .is_none_or(|sid| sid == lease.session_id());
                    if !sid_matches {
                        continue;
                    }
                    let ev_name = ev.event_name();
                    if ev_name == "delta.text_append" {
                        if let Some(chunk) = ev.payload().get("text").and_then(Value::as_str)
                            && !chunk.is_empty()
                        {
                            if let Some((kind, reason)) = classify_assistant_error(chunk) {
                                return Err(WorkflowFailure::Upstream(kind, reason));
                            }
                            emitted_text.push_str(chunk);
                            on_delta(chunk.to_string());
                        }
                    } else if ev_name.contains("approval") {
                        handle_chat_approval(lease, &ev).await;
                    } else if ev_name == "delta.message_done" {
                        let done_text = extract_message_done_text(ev.payload());
                        if let Some((kind, reason)) = classify_assistant_error(&done_text) {
                            return Err(WorkflowFailure::Upstream(kind, reason));
                        }
                        append_and_emit_missing_suffix(&mut emitted_text, &done_text, on_delta);
                        explicit_done = true;
                        should_poll_history = true;
                    } else if ev.agent_activity_state() == Some(false)
                        || ev.is_session_completed_for(lease.session_id())
                    {
                        explicit_done = true;
                        should_poll_history = true;
                    } else if ev_name == "error" || ev_name == "chat.error" {
                        let msg = ev
                            .payload()
                            .get("message")
                            .or_else(|| ev.payload().get("error"))
                            .and_then(Value::as_str)
                            .unwrap_or("upstream chat error");
                        if let Some((kind, reason)) = classify_assistant_error(msg) {
                            return Err(WorkflowFailure::Upstream(kind, reason));
                        }
                        return Err(WorkflowFailure::Upstream(
                            UpstreamFailureKind::Transient,
                            msg.to_string(),
                        ));
                    }
                }
            }
            _ = poll_interval.tick() => {
                should_poll_history = true;
            }
        }

        if should_poll_history {
            should_poll_history = false;
            let history_path = format!("/chat/history?session_id={}", lease.session_id());
            if let Ok(resp) = lease
                .client()
                .send_request(
                    "GET",
                    &history_path,
                    None,
                    SERVICE_DAEMON,
                    Duration::from_secs(15),
                    false,
                )
                .await
                && resp.status == 200
                && let Ok(history_json) = resp.json()
            {
                let turn_events = extract_history_turn_events(&history_json);
                let assistant_blocks = extract_turn_assistant_messages(&turn_events);
                let history_text = assistant_blocks.join("\n\n");
                if !history_text.is_empty() {
                    if let Some((kind, reason)) = classify_assistant_error(&history_text) {
                        return Err(WorkflowFailure::Upstream(kind, reason));
                    }
                    append_and_emit_missing_suffix(&mut emitted_text, &history_text, on_delta);
                }
                let active_tools = has_active_tool_task(&turn_events);
                if !active_tools
                    && !emitted_text.trim().is_empty()
                    && (explicit_done || is_last_assistant_completed(&turn_events))
                {
                    break;
                }
            } else if explicit_done && !emitted_text.trim().is_empty() {
                break;
            }
        }
    }

    let final_text = emitted_text.trim().to_string();
    if final_text.is_empty() {
        return Err(WorkflowFailure::Timeout(
            "Muse chat stream timed out before producing assistant text".to_string(),
        ));
    }

    if let Some((kind, reason)) = classify_assistant_error(&final_text) {
        return Err(WorkflowFailure::Upstream(kind, reason));
    }

    let updated_hot = if let Some(lu) = last_user
        && !lu.is_empty()
    {
        let mut next_turns = prior_turns.to_vec();
        next_turns.push(ConversationTurn {
            user: lu.to_string(),
            assistant: final_text.clone(),
        });
        Some(HotConversation {
            turns: next_turns,
            image_count: images.len(),
            session_id: lease.session_id().to_string(),
        })
    } else {
        None
    };
    lease.finish_clean(updated_hot);
    Ok(final_text)
}

fn append_and_emit_missing_suffix<F>(
    emitted_text: &mut String,
    candidate_full: &str,
    on_delta: &mut F,
) where
    F: FnMut(String) + Send,
{
    if candidate_full.is_empty() {
        return;
    }
    let delta = if emitted_text.is_empty() {
        candidate_full.to_string()
    } else if candidate_full.len() > emitted_text.len() && candidate_full.starts_with(&*emitted_text)
    {
        candidate_full[emitted_text.len()..].to_string()
    } else {
        String::new()
    };
    if !delta.is_empty() {
        emitted_text.push_str(&delta);
        on_delta(delta);
    }
}

fn extract_message_done_text(payload: &Value) -> String {
    if let Some(msgs) = payload
        .get("transcript")
        .and_then(|t| t.get("messages"))
        .and_then(Value::as_array)
    {
        let mut blocks = Vec::new();
        for m in msgs {
            if m.get("role").and_then(Value::as_str) == Some("assistant")
                && let Some(t) = m
                    .get("content")
                    .or_else(|| m.get("text"))
                    .or_else(|| m.get("display_text"))
                    .and_then(Value::as_str)
            {
                let trimmed = t.trim();
                if !trimmed.is_empty() {
                    blocks.push(trimmed.to_string());
                }
            }
        }
        if !blocks.is_empty() {
            return blocks.join("\n\n");
        }
    }
    payload
        .get("content")
        .or_else(|| payload.get("text"))
        .or_else(|| payload.get("display_text"))
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string()
}

fn is_last_assistant_completed(turn_events: &[Value]) -> bool {
    turn_events
        .iter()
        .rev()
        .find(|ev| {
            let p = ev.get("payload").unwrap_or(ev);
            ev.get("role").and_then(Value::as_str) == Some("assistant")
                || p.get("role").and_then(Value::as_str) == Some("assistant")
                || ev.get("event_name").and_then(Value::as_str) == Some("message.assistant")
        })
        .is_some_and(|ev| {
            let p = ev.get("payload").unwrap_or(ev);
            ev.get("status").and_then(Value::as_str) == Some("completed")
                || p.get("status").and_then(Value::as_str) == Some("completed")
                || ev.get("event_name").and_then(Value::as_str) == Some("message.assistant")
        })
}

async fn handle_chat_approval(lease: &VmOperationLease, ev: &VmStreamEvent) {
    handle_approval_event(lease, ev, |_| false).await;
}
