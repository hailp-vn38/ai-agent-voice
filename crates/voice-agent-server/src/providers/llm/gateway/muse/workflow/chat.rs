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
    extract_turn_assistant_messages, extract_value_text, handle_approval_event,
    has_active_tool_task,
};
use crate::providers::llm::{LlmRequest, ReferenceImage};

const CHAT_POLL_INTERVAL: Duration = Duration::from_millis(400);
const ONLINE_IDLE_DEBOUNCE: Duration = Duration::from_millis(3500);

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
    let mut poll_interval = tokio::time::interval_at(
        Instant::now() + Duration::from_millis(300),
        CHAT_POLL_INTERVAL,
    );

    let mut emitted_text = String::new();
    let mut last_polled_text = String::new();
    let mut latest_activity_code = String::from("unknown");
    let mut session_running_seen = false;
    let mut session_completed = false;
    let mut message_done_seen = false;
    let mut unchanged_ticks = 0usize;
    let mut should_poll_history = false;
    let mut debounce_deadline: Option<Instant> = None;
    let mut finish_after_poll = false;
    let expected_user_turns = lease
        .matched_hot()
        .map(|hot| hot.turns.len() + 1)
        .unwrap_or(1);

    while Instant::now() < deadline {
        let debounce_sleep = debounce_deadline.unwrap_or(deadline);
        tokio::select! {
            recv_res = event_rx.recv() => {
                if let Ok(ev) = recv_res {
                    let ev_name = ev.event_name();
                    let ev_sid = ev.session_id();
                    let sid_matches = ev_sid == Some(lease.session_id());

                    if ev_sid.is_none_or(|sid| sid == lease.session_id())
                        && (ev_name == "approvals.snapshot"
                            || ev_name.starts_with("task.")
                            || ev_name.contains("approval"))
                    {
                        unchanged_ticks = 0;
                        debounce_deadline = None;
                        if ev_name == "approvals.snapshot" || ev_name.contains("approval") {
                            handle_chat_approval(lease, &ev).await;
                        }
                    }

                    if let Some(status) = ev.session_update_status(lease.session_id()) {
                        if status == "completed" {
                            session_completed = true;
                            should_poll_history = true;
                        } else if status == "running" {
                            session_running_seen = true;
                            session_completed = false;
                            message_done_seen = false;
                            debounce_deadline = None;
                            unchanged_ticks = 0;
                            should_poll_history = true;
                        }
                        continue;
                    }

                    if !sid_matches {
                        continue;
                    }

                    if ev_name == "delta.text_append" {
                        if let Some(chunk) = ev.payload().get("text").and_then(Value::as_str)
                            && !chunk.is_empty()
                        {
                            if let Some((kind, reason)) = classify_assistant_error(chunk) {
                                return Err(WorkflowFailure::Upstream(kind, reason));
                            }
                            emitted_text.push_str(chunk);
                            last_polled_text = emitted_text.clone();
                            unchanged_ticks = 0;
                            debounce_deadline = None;
                            on_delta(chunk.to_string());
                        }
                    } else if ev_name == "delta.presentation" {
                        let p = ev.payload();
                        if p.get("kind").and_then(Value::as_str) != Some("browser_task")
                            && p.get("data").and_then(|d| d.get("browser_task_id")).is_none()
                        {
                            unchanged_ticks = 0;
                        }
                    } else if ev_name == "agent.status" {
                        if let Some(code) = ev.activity_code() {
                            latest_activity_code = code.to_string();
                            if matches!(
                                code,
                                "working" | "making_something" | "responding" | "thinking" | "running"
                            ) {
                                unchanged_ticks = 0;
                                message_done_seen = false;
                                debounce_deadline = None;
                            } else if matches!(code, "online" | "idle")
                                && session_completed
                                && !emitted_text.trim().is_empty()
                            {
                                debounce_deadline = Some(Instant::now() + ONLINE_IDLE_DEBOUNCE);
                            }
                        }
                    } else if ev_name == "delta.message_done" {
                        let done_text = extract_message_done_text(ev.payload());
                        if let Some((kind, reason)) = classify_assistant_error(&done_text) {
                            return Err(WorkflowFailure::Upstream(kind, reason));
                        }
                        if reconcile_and_emit_history_text(&mut emitted_text, &done_text, on_delta) {
                            last_polled_text = emitted_text.clone();
                            unchanged_ticks = 0;
                        }
                        message_done_seen = true;
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
            _ = tokio::time::sleep_until(debounce_sleep), if debounce_deadline.is_some() => {
                debounce_deadline = None;
                should_poll_history = true;
                finish_after_poll = true;
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
                if is_stale_followup_history(
                    &history_json,
                    expected_user_turns,
                    &effective_prompt,
                ) {
                    finish_after_poll = false;
                    continue;
                }
                let turn_events = extract_history_turn_events(&history_json);
                let assistant_blocks = extract_turn_assistant_messages(&turn_events);
                let history_text = assistant_blocks.join("\n\n");
                if !history_text.is_empty() {
                    if let Some((kind, reason)) = classify_assistant_error(&history_text) {
                        return Err(WorkflowFailure::Upstream(kind, reason));
                    }
                    if reconcile_and_emit_history_text(&mut emitted_text, &history_text, on_delta) {
                        unchanged_ticks = 0;
                        last_polled_text = history_text.clone();
                    }
                }

                let active_tools = has_active_tool_task(&turn_events);
                let is_assistant_completed = is_last_assistant_completed(&turn_events);
                let is_all_completed =
                    (session_completed || is_assistant_completed) && !active_tools;
                let is_agent_working = active_tools
                    || (!turn_events.is_empty() && !is_all_completed)
                    || matches!(
                        latest_activity_code.as_str(),
                        "working" | "making_something" | "responding" | "thinking" | "running"
                    );

                if !emitted_text.is_empty() && history_text == last_polled_text {
                    if !is_agent_working {
                        unchanged_ticks += 1;
                    } else {
                        unchanged_ticks = 0;
                        debounce_deadline = None;
                    }
                }

                if finish_after_poll {
                    finish_after_poll = false;
                    if !emitted_text.trim().is_empty() && !active_tools && !is_agent_working {
                        break;
                    }
                }

                if !emitted_text.trim().is_empty() && !active_tools && !is_agent_working {
                    if session_completed && is_assistant_completed {
                        break;
                    }
                    if !session_running_seen
                        && is_assistant_completed
                        && (message_done_seen
                            || matches!(latest_activity_code.as_str(), "idle" | "online"))
                    {
                        break;
                    }
                    let required_ticks = if is_all_completed {
                        if session_running_seen { 12 } else { 2 }
                    } else {
                        25
                    };
                    if unchanged_ticks >= required_ticks {
                        break;
                    }
                }
            } else if (session_completed || message_done_seen) && !emitted_text.trim().is_empty() {
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

fn is_stale_followup_history(
    history_json: &Value,
    expected_user_turns: usize,
    effective_prompt: &str,
) -> bool {
    if expected_user_turns <= 1 {
        return false;
    }
    let evs = history_json
        .get("result")
        .and_then(|r| r.get("chat_events"))
        .or_else(|| history_json.get("chat_events"))
        .and_then(Value::as_array);
    let Some(evs) = evs else {
        return false;
    };
    let user_evs: Vec<&Value> = evs
        .iter()
        .filter(|e| {
            let p = e.get("payload").unwrap_or(e);
            e.get("role").and_then(Value::as_str) == Some("user")
                || p.get("role").and_then(Value::as_str) == Some("user")
                || e.get("event_name").and_then(Value::as_str) == Some("message.user")
                || e.get("event").and_then(Value::as_str) == Some("message.user")
        })
        .collect();
    if user_evs.len() >= expected_user_turns {
        return false;
    }
    let Some(&ev) = user_evs.last() else {
        return true;
    };
    let p = ev.get("payload").unwrap_or(ev);
    let last_u_text = ev
        .get("display_text")
        .and_then(extract_value_text)
        .or_else(|| p.get("display_text").and_then(extract_value_text))
        .or_else(|| p.get("content").and_then(extract_value_text))
        .or_else(|| ev.get("content").and_then(extract_value_text))
        .unwrap_or_default();
    last_u_text.trim() != effective_prompt.trim()
}

fn reconcile_and_emit_history_text<F>(
    emitted_text: &mut String,
    candidate_full: &str,
    on_delta: &mut F,
) -> bool
where
    F: FnMut(String) + Send,
{
    if candidate_full.is_empty() || candidate_full.len() <= emitted_text.len() {
        return false;
    }
    if emitted_text.is_empty() {
        *emitted_text = candidate_full.to_string();
        on_delta(candidate_full.to_string());
        return true;
    }
    if candidate_full.starts_with(&*emitted_text) {
        let delta = candidate_full[emitted_text.len()..].to_string();
        *emitted_text = candidate_full.to_string();
        if !delta.is_empty() {
            on_delta(delta);
            return true;
        }
        return false;
    }
    if let Some(idx) = candidate_full.find(&*emitted_text) {
        let delta = candidate_full[idx + emitted_text.len()..].to_string();
        *emitted_text = candidate_full.to_string();
        if !delta.is_empty() {
            on_delta(delta);
            return true;
        }
        return false;
    }

    // Suffix / prefix overlap matching (`muse-api-server-main/src/core/client.js:1397-1410`)
    let max_possible = emitted_text.len().min(candidate_full.len());
    let mut max_overlap = 0usize;
    for i in (1..=max_possible).rev() {
        if candidate_full.is_char_boundary(i) && emitted_text.ends_with(&candidate_full[..i]) {
            max_overlap = i;
            break;
        }
    }
    let delta = candidate_full[max_overlap..].to_string();
    if !delta.is_empty() {
        emitted_text.push_str(&delta);
        on_delta(delta);
        return true;
    }
    false
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
                    .and_then(extract_value_text)
                    .or_else(|| m.get("text").and_then(extract_value_text))
                    .or_else(|| m.get("display_text").and_then(extract_value_text))
            {
                if !t.is_empty() {
                    blocks.push(t);
                }
            }
        }
        if !blocks.is_empty() {
            return blocks.join("\n\n");
        }
    }
    payload
        .get("content")
        .and_then(extract_value_text)
        .or_else(|| payload.get("text").and_then(extract_value_text))
        .or_else(|| payload.get("display_text").and_then(extract_value_text))
        .unwrap_or_default()
}

fn is_last_assistant_completed(turn_events: &[Value]) -> bool {
    turn_events
        .iter()
        .rev()
        .find(|ev| {
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
        .is_some_and(|ev| {
            let p = ev.get("payload").unwrap_or(ev);
            ev.get("status").and_then(Value::as_str) == Some("completed")
                || p.get("status").and_then(Value::as_str) == Some("completed")
        })
}

async fn handle_chat_approval(lease: &VmOperationLease, ev: &VmStreamEvent) {
    handle_approval_event(lease, ev, |_| false).await;
}
