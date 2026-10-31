mod candidates;

use std::sync::Arc;
use std::time::Duration;

use base64::Engine as _;
use serde_json::{Value, json};
use tokio::time::Instant;

use self::candidates::{
    ReadyMediaCandidate, collect_presentations_from_payload,
    extract_candidates_from_stream_and_history, format_clean_assistant_bubbles,
    is_presentation_pending, select_effective_ready_attachments, sniff_media_mime_type,
};
use super::super::client::VmStreamEvent;
use super::super::pool::{SharedMusePool, UpstreamFailureKind, VmOperationLease};
use super::super::proto::SERVICE_DAEMON;
use super::png::remove_background_to_rgba_png;
use super::{
    MEDIA_DONE_MARKER, MediaKind, WorkflowFailure, classify_assistant_error,
    extract_history_turn_events, extract_turn_assistant_messages, has_active_tool_task,
    is_last_assistant_completed,
};
use crate::providers::llm::{LlmOutput, MediaPart, ReferenceImage};

const MEDIA_POLL_INTERVAL: Duration = Duration::from_millis(400);
const MAX_IMAGE_FILE_BYTES: usize = 50 * 1024 * 1024;
const MAX_VIDEO_FILE_BYTES: usize = 150 * 1024 * 1024;

const VIDEO_ATTACH_FOLLOWUP_PROMPT: &str = "You have not attached the final video file(s) to this chat thread yet. Do NOT ask any questions. Generate and attach the final MP4 video file(s) directly into this chat message now, and end your message with [ALL_MEDIA_ATTACHED] once every video is attached.";
const IMAGE_ATTACH_FOLLOWUP_PROMPT: &str = "You have not attached the generated image to this chat thread yet. Generate and attach the image file directly into this chat message now, and end your message with [ALL_MEDIA_ATTACHED].";

pub(crate) async fn run_media_turn_once(
    pool: &Arc<SharedMusePool>,
    kind: MediaKind,
    formatted_prompt: &str,
    images: &[ReferenceImage],
    transparent_background: bool,
    timeout_duration: Duration,
) -> Result<LlmOutput, WorkflowFailure> {
    if formatted_prompt.trim().is_empty() {
        return Err(WorkflowFailure::Rejected("empty media prompt".to_string()));
    }
    let mut lease = pool
        .acquire_fresh()
        .await
        .map_err(|(kind, reason)| WorkflowFailure::Upstream(kind, reason))?;

    let res = execute_media_turn_on_lease(
        &mut lease,
        formatted_prompt,
        images,
        kind,
        transparent_background,
        timeout_duration,
    )
    .await;

    if let Err(WorkflowFailure::Upstream(err_kind, _)) = &res {
        lease.fail_account(*err_kind).await;
    }
    res
}

async fn execute_media_turn_on_lease(
    lease: &mut VmOperationLease,
    prompt: &str,
    images: &[ReferenceImage],
    kind: MediaKind,
    transparent_background: bool,
    timeout_duration: Duration,
) -> Result<LlmOutput, WorkflowFailure> {
    let mut items = Vec::with_capacity(images.len() + 1);
    for img in images {
        items.push(json!({
            "type": "image",
            "data_base64": base64::engine::general_purpose::STANDARD.encode(&img.bytes),
            "mime_type": img.mime_type,
        }));
    }
    items.push(json!({
        "type": "text",
        "text": prompt,
    }));

    let mut event_rx = lease.client().subscribe_events();
    send_media_stream_request(lease, items).await?;

    let deadline = Instant::now() + timeout_duration;
    let mut poll_interval = tokio::time::interval(MEDIA_POLL_INTERVAL);
    poll_interval.tick().await;

    let mut stream_presentations: Vec<Value> = Vec::new();
    let mut streamed_text = String::new();
    let mut assistant_bubbles: Vec<String> = Vec::new();
    let mut ready_candidates: Vec<ReadyMediaCandidate> = Vec::new();
    let mut explicit_done = false;
    let mut should_poll_history = false;
    let mut nudged_attach = false;

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
                        if let Some(chunk) = ev.payload().get("text").and_then(Value::as_str) {
                            streamed_text.push_str(chunk);
                        }
                    } else if ev_name == "delta.presentation" {
                        let p = ev.payload().clone();
                        if !is_presentation_pending(&p) {
                            stream_presentations.push(p);
                            should_poll_history = true;
                        }
                    } else if ev_name.contains("approval") {
                        handle_media_approval(lease, &ev).await;
                    } else if ev_name == "delta.message_done" {
                        collect_presentations_from_payload(ev.payload(), &mut stream_presentations);
                        explicit_done = true;
                        should_poll_history = true;
                    } else if ev.agent_activity_state() == Some(true)
                        || ev.session_update_status(lease.session_id()) == Some("running")
                    {
                        explicit_done = false;
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
                            .unwrap_or("upstream media generation error");
                        if let Some((err_kind, reason)) = classify_assistant_error(msg) {
                            return Err(WorkflowFailure::Upstream(err_kind, reason));
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
            let mut active_tools = false;
            let mut is_assistant_completed = false;

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
                active_tools = has_active_tool_task(&turn_events);
                is_assistant_completed = is_last_assistant_completed(&turn_events);
                let history_bubbles = extract_turn_assistant_messages(&turn_events);
                if !history_bubbles.is_empty() {
                    assistant_bubbles = history_bubbles;
                } else if !streamed_text.trim().is_empty() {
                    assistant_bubbles = vec![streamed_text.trim().to_string()];
                }
                ready_candidates = extract_candidates_from_stream_and_history(
                    kind,
                    &stream_presentations,
                    &turn_events,
                    &assistant_bubbles,
                );
            } else {
                if !streamed_text.trim().is_empty() && assistant_bubbles.is_empty() {
                    assistant_bubbles = vec![streamed_text.trim().to_string()];
                }
                ready_candidates = extract_candidates_from_stream_and_history(
                    kind,
                    &stream_presentations,
                    &[],
                    &assistant_bubbles,
                );
            }

            let joined_raw = assistant_bubbles.join("\n\n");
            if let Some((err_kind, reason)) = classify_assistant_error(&joined_raw) {
                return Err(WorkflowFailure::Upstream(err_kind, reason));
            }

            let has_done_marker = assistant_bubbles
                .iter()
                .any(|b| b.contains(MEDIA_DONE_MARKER))
                || streamed_text.contains(MEDIA_DONE_MARKER);
            let media_turn_done = match kind {
                MediaKind::Image => true,
                MediaKind::Video => has_done_marker || explicit_done || is_assistant_completed,
            };

            if !active_tools && !ready_candidates.is_empty() && media_turn_done {
                break;
            }

            if !active_tools
                && ready_candidates.is_empty()
                && (explicit_done || is_assistant_completed)
                && !nudged_attach
                && !joined_raw.trim().is_empty()
            {
                nudged_attach = true;
                explicit_done = false;
                streamed_text.clear();
                let followup = match kind {
                    MediaKind::Video => VIDEO_ATTACH_FOLLOWUP_PROMPT,
                    MediaKind::Image => IMAGE_ATTACH_FOLLOWUP_PROMPT,
                };
                let followup_items = vec![json!({
                    "type": "text",
                    "text": followup,
                })];
                send_media_stream_request(lease, followup_items).await?;
            }
        }
    }

    let selected = select_effective_ready_attachments(&ready_candidates);
    if selected.is_empty() {
        return Err(WorkflowFailure::Timeout(format!(
            "Muse {kind:?} generation timed out without producing any downloadable file path"
        )));
    }

    let assistant_message = format_clean_assistant_bubbles(&assistant_bubbles);

    match kind {
        MediaKind::Image => {
            let chosen = selected.last().expect("selected is non-empty");
            let raw_bytes = lease
                .client()
                .read_file(&chosen.path, MAX_IMAGE_FILE_BYTES)
                .await?;
            if raw_bytes.is_empty() {
                return Err(WorkflowFailure::Upstream(
                    UpstreamFailureKind::Transient,
                    format!("Muse /fs/read returned empty image for {}", chosen.path),
                ));
            }
            let (final_bytes, final_mime) = if transparent_background {
                let processed = remove_background_to_rgba_png(&raw_bytes)?;
                let mime = sniff_media_mime_type(&processed, "image/png", MediaKind::Image);
                (processed, mime)
            } else {
                let mime = sniff_media_mime_type(&raw_bytes, "image/webp", MediaKind::Image);
                (raw_bytes, mime)
            };
            lease.finish_clean(None);
            LlmOutput::from_media_parts(
                vec![MediaPart {
                    bytes: final_bytes,
                    mime_type: final_mime,
                }],
                assistant_message,
            )
            .ok_or_else(|| {
                WorkflowFailure::Upstream(
                    UpstreamFailureKind::Transient,
                    "empty image media parts".to_string(),
                )
            })
        }
        MediaKind::Video => {
            let mut parts = Vec::with_capacity(selected.len());
            for item in &selected {
                let raw_bytes = lease
                    .client()
                    .read_file(&item.path, MAX_VIDEO_FILE_BYTES)
                    .await?;
                if raw_bytes.is_empty() {
                    return Err(WorkflowFailure::Upstream(
                        UpstreamFailureKind::Transient,
                        format!("Muse /fs/read returned empty video for {}", item.path),
                    ));
                }
                let mime_type = sniff_media_mime_type(&raw_bytes, "video/mp4", MediaKind::Video);
                parts.push(MediaPart {
                    bytes: raw_bytes,
                    mime_type,
                });
            }
            lease.finish_clean(None);
            LlmOutput::from_media_parts(parts, assistant_message).ok_or_else(|| {
                WorkflowFailure::Upstream(
                    UpstreamFailureKind::Transient,
                    "no video parts downloaded".to_string(),
                )
            })
        }
    }
}

async fn send_media_stream_request(
    lease: &mut VmOperationLease,
    items: Vec<Value>,
) -> Result<(), WorkflowFailure> {
    let body = json!({
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
            Some(body.to_string().as_bytes()),
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
    Ok(())
}

async fn handle_media_approval(lease: &VmOperationLease, ev: &VmStreamEvent) {
    super::handle_approval_event(lease, ev, should_allow_media_approval).await;
}

fn should_allow_media_approval(item: &Value) -> bool {
    let raw = item.to_string().to_ascii_lowercase();
    !(raw.contains("bash") || raw.contains("curl") || raw.contains("wget"))
}
