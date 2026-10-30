use std::sync::Arc;
use std::time::Duration;

use base64::Engine as _;
use serde_json::{Value, json};
use tokio::time::Instant;

use super::super::client::VmStreamEvent;
use super::super::pool::{SharedMusePool, UpstreamFailureKind, VmOperationLease};
use super::super::proto::SERVICE_DAEMON;
use super::png::remove_background_to_rgba_png;
use super::{
    MEDIA_DONE_MARKER, MediaKind, WorkflowFailure, classify_assistant_error,
    extract_history_turn_events, extract_turn_assistant_messages, has_active_tool_task,
};
use crate::providers::llm::{LlmOutput, MediaPart, ReferenceImage};

const MEDIA_POLL_INTERVAL: Duration = Duration::from_millis(400);
const MAX_IMAGE_FILE_BYTES: usize = 50 * 1024 * 1024;
const MAX_VIDEO_FILE_BYTES: usize = 150 * 1024 * 1024;

const VIDEO_ATTACH_FOLLOWUP_PROMPT: &str = "You have not attached the final video file(s) to this chat thread yet. Do NOT ask any questions. Generate and attach the final MP4 video file(s) directly into this chat message now, and end your message with [ALL_MEDIA_ATTACHED] once every video is attached.";
const IMAGE_ATTACH_FOLLOWUP_PROMPT: &str = "You have not attached the generated image to this chat thread yet. Generate and attach the image file directly into this chat message now, and end your message with [ALL_MEDIA_ATTACHED].";

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ReadyMediaCandidate {
    pub path: String,
    pub group: usize,
}

fn strip_done_marker(text: &str) -> String {
    text.replace(MEDIA_DONE_MARKER, "").trim().to_owned()
}

pub(super) fn format_clean_assistant_bubbles(bubbles: &[String]) -> Option<String> {
    let joined = bubbles
        .iter()
        .map(|s| strip_done_marker(s))
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("\n\n");
    (!joined.is_empty()).then_some(joined)
}

pub(super) fn select_effective_ready_attachments(
    ready: &[ReadyMediaCandidate],
) -> Vec<ReadyMediaCandidate> {
    let mut unique: Vec<ReadyMediaCandidate> = Vec::with_capacity(ready.len());
    for item in ready {
        let clean = item.path.trim().trim_start_matches("sandbox://");
        if !clean.is_empty()
            && !unique
                .iter()
                .any(|u| u.path.trim().trim_start_matches("sandbox://") == clean)
        {
            unique.push(item.clone());
        }
    }
    if unique.len() <= 1 {
        return unique;
    }
    let max_group = unique.iter().map(|c| c.group).max().unwrap_or(0);
    let min_group = unique.iter().map(|c| c.group).min().unwrap_or(0);
    if max_group > min_group {
        let first_group_count = unique.iter().filter(|c| c.group == min_group).count();
        let last_group: Vec<ReadyMediaCandidate> = unique
            .iter()
            .filter(|c| c.group == max_group)
            .cloned()
            .collect();
        if first_group_count > 1 && last_group.len() == 1 {
            return last_group;
        }
    }
    unique
}

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

            if !active_tools && !ready_candidates.is_empty() && (has_done_marker || explicit_done) {
                break;
            }

            if !active_tools
                && ready_candidates.is_empty()
                && explicit_done
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
                let rgba_png = remove_background_to_rgba_png(&raw_bytes)?;
                (rgba_png, "image/png".to_string())
            } else {
                let mime = sniff_media_mime_type(&raw_bytes, "image/png", MediaKind::Image);
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

fn is_presentation_pending(p: &Value) -> bool {
    let status = p
        .get("status")
        .or_else(|| p.get("data").and_then(|d| d.get("status")))
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_ascii_lowercase();
    matches!(status.as_str(), "loading" | "generating" | "pending")
}

fn collect_presentations_from_payload(payload: &Value, out: &mut Vec<Value>) {
    if let Some(arr) = payload.get("presentations").and_then(Value::as_array) {
        for p in arr {
            if !is_presentation_pending(p) {
                out.push(p.clone());
            }
        }
    }
    if let Some(msgs) = payload
        .get("transcript")
        .and_then(|t| t.get("messages"))
        .and_then(Value::as_array)
    {
        for m in msgs {
            if let Some(arr) = m.get("presentations").and_then(Value::as_array) {
                for p in arr {
                    if !is_presentation_pending(p) {
                        out.push(p.clone());
                    }
                }
            }
        }
    }
}

fn extract_candidates_from_stream_and_history(
    kind: MediaKind,
    stream_presentations: &[Value],
    turn_events: &[Value],
    assistant_bubbles: &[String],
) -> Vec<ReadyMediaCandidate> {
    let mut candidates = Vec::new();
    let mut group_counter = 0usize;

    for p in stream_presentations {
        let before = candidates.len();
        extract_paths_from_presentation(kind, p, group_counter, &mut candidates);
        if candidates.len() > before {
            group_counter += 1;
        }
    }

    for ev in turn_events {
        let p = ev.get("payload").unwrap_or(ev);
        if !is_presentation_pending(p) {
            let before = candidates.len();
            extract_paths_from_presentation(kind, p, group_counter, &mut candidates);
            if candidates.len() > before {
                group_counter += 1;
            }
        }
        if let Some(arr) = p
            .get("presentations")
            .or_else(|| ev.get("presentations"))
            .and_then(Value::as_array)
        {
            for pres in arr {
                if !is_presentation_pending(pres) {
                    let before = candidates.len();
                    extract_paths_from_presentation(kind, pres, group_counter, &mut candidates);
                    if candidates.len() > before {
                        group_counter += 1;
                    }
                }
            }
        }
    }

    for bubble in assistant_bubbles {
        let before = candidates.len();
        extract_sandbox_paths_from_text(kind, bubble, group_counter, &mut candidates);
        if candidates.len() > before {
            group_counter += 1;
        }
    }

    candidates
}

fn extract_paths_from_presentation(
    kind: MediaKind,
    p: &Value,
    group: usize,
    out: &mut Vec<ReadyMediaCandidate>,
) {
    match kind {
        MediaKind::Image => {
            if let Some(path) = p
                .get("image_path")
                .or_else(|| p.get("path"))
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty() && !has_video_ext(s))
            {
                out.push(ReadyMediaCandidate {
                    path: path.to_string(),
                    group,
                });
            }
            if let Some(images) = p
                .get("data")
                .and_then(|d| d.get("images"))
                .or_else(|| p.get("images"))
                .and_then(Value::as_array)
            {
                for img in images {
                    if let Some(path) = img
                        .get("path")
                        .or_else(|| img.get("image_path"))
                        .and_then(Value::as_str)
                        .map(str::trim)
                        .filter(|s| !s.is_empty() && !has_video_ext(s))
                    {
                        out.push(ReadyMediaCandidate {
                            path: path.to_string(),
                            group,
                        });
                    }
                }
            }
        }
        MediaKind::Video => {
            if let Some(path) = p
                .get("video_path")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())
            {
                out.push(ReadyMediaCandidate {
                    path: path.to_string(),
                    group,
                });
            }
            if let Some(videos) = p
                .get("data")
                .and_then(|d| d.get("videos"))
                .or_else(|| p.get("videos"))
                .and_then(Value::as_array)
            {
                for vid in videos {
                    if let Some(path) = vid
                        .get("path")
                        .or_else(|| vid.get("video_path"))
                        .and_then(Value::as_str)
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                    {
                        out.push(ReadyMediaCandidate {
                            path: path.to_string(),
                            group,
                        });
                    }
                }
            }
            if let Some(path) = p
                .get("image_path")
                .or_else(|| p.get("path"))
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|s| has_video_ext(s))
            {
                out.push(ReadyMediaCandidate {
                    path: path.to_string(),
                    group,
                });
            }
        }
    }

    if let Some(fb) = p.get("fallback_text").and_then(Value::as_str) {
        extract_sandbox_paths_from_text(kind, fb, group, out);
    }
}

fn extract_sandbox_paths_from_text(
    kind: MediaKind,
    text: &str,
    group: usize,
    out: &mut Vec<ReadyMediaCandidate>,
) {
    let mut rest = text;
    while let Some(idx) = rest.find("sandbox://") {
        let after = &rest[idx..];
        let token: String = after
            .chars()
            .take_while(|c| {
                !c.is_whitespace() && !matches!(c, ')' | ']' | '"' | '\'' | '>' | ',' | ';')
            })
            .collect();
        let matches_kind = match kind {
            MediaKind::Image => has_image_ext(&token),
            MediaKind::Video => has_video_ext(&token),
        };
        if matches_kind {
            out.push(ReadyMediaCandidate { path: token, group });
        }
        rest = &after["sandbox://".len()..];
    }
}

fn has_image_ext(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    lower.ends_with(".png")
        || lower.ends_with(".jpg")
        || lower.ends_with(".jpeg")
        || lower.ends_with(".webp")
}

fn has_video_ext(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    lower.ends_with(".mp4") || lower.ends_with(".webm") || lower.ends_with(".mov")
}

async fn handle_media_approval(lease: &VmOperationLease, ev: &VmStreamEvent) {
    super::handle_approval_event(lease, ev, should_allow_media_approval).await;
}

fn should_allow_media_approval(item: &Value) -> bool {
    let raw = item.to_string().to_ascii_lowercase();
    !(raw.contains("bash") || raw.contains("curl") || raw.contains("wget"))
}

pub(super) fn sniff_media_mime_type(bytes: &[u8], hinted: &str, kind: MediaKind) -> String {
    if bytes.starts_with(&[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]) {
        return "image/png".to_string();
    }
    if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        return "image/jpeg".to_string();
    }
    if bytes.len() >= 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        return "image/webp".to_string();
    }
    if bytes.len() >= 12 && &bytes[4..8] == b"ftyp" {
        return "video/mp4".to_string();
    }
    if bytes.starts_with(&[0x1A, 0x45, 0xDF, 0xA3]) {
        return "video/webm".to_string();
    }
    let clean = hinted
        .split(';')
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    if clean.starts_with("image/") || clean.starts_with("video/") {
        return clean;
    }
    match kind {
        MediaKind::Image => "image/png".to_string(),
        MediaKind::Video => "video/mp4".to_string(),
    }
}
