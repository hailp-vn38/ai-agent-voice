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
use super::super::pool::{
    JobEntry, JobSession, SharedMusePool, UpstreamFailureKind, VmOperationLease,
};
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
/// Follow-up turns (same Muse session) allowed when the delivered video is shorter than requested.
const MAX_DURATION_REPAIRS: usize = 2;
/// A video up to this many seconds short still counts as complete (half of one 10s Muse clip).
const DURATION_SHORTFALL_TOLERANCE_SECS: u32 = 5;

const VIDEO_ATTACH_FOLLOWUP_PROMPT: &str = "You have not attached the final video file(s) to this chat thread yet. Do NOT ask any questions. Generate and attach the final MP4 video file(s) directly into this chat message now, and end your message with [ALL_MEDIA_ATTACHED] once every video is attached.";
const IMAGE_ATTACH_FOLLOWUP_PROMPT: &str = "You have not attached the generated image to this chat thread yet. Generate and attach the image file directly into this chat message now, and end your message with [ALL_MEDIA_ATTACHED].";

/// One chat turn sent to Muse. A long video is split into several batches that run back to back
/// in the same Muse session; each batch is one merged file of at most one turn's worth of clips.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct MediaBatch {
    pub prompt: String,
    /// Duration this batch should deliver (videos only); drives the shortfall repair.
    pub target_secs: Option<u32>,
}

pub(crate) async fn run_media_turn_once(
    pool: &Arc<SharedMusePool>,
    kind: MediaKind,
    batches: &[MediaBatch],
    images: &[ReferenceImage],
    transparent_background: bool,
    job: Option<Arc<JobEntry>>,
    timeout_duration: Duration,
) -> Result<LlmOutput, WorkflowFailure> {
    if batches.is_empty() || batches.iter().any(|b| b.prompt.trim().is_empty()) {
        return Err(WorkflowFailure::Rejected("empty media prompt".to_string()));
    }
    // Calls of one job run one at a time so they never interleave inside the shared session.
    let mut gate = match &job {
        Some(entry) => Some(entry.lock().await),
        None => None,
    };
    let retained = gate
        .as_ref()
        .and_then(|slot| (**slot).clone())
        .filter(|session| {
            pool.gateway_account_key().as_deref() == Some(session.account_key.as_str())
        });
    let mut lease = match &retained {
        Some(session) => {
            pool.acquire_existing_session(session.session_id.clone())
                .await
        }
        None => pool.acquire_fresh().await,
    }
    .map_err(|(kind, reason)| WorkflowFailure::Upstream(kind, reason))?;
    if job.is_some() {
        lease.retain_session();
    }

    let res = execute_media_turn_on_lease(
        &mut lease,
        batches,
        images,
        kind,
        transparent_background,
        timeout_duration,
    )
    .await;

    if let Err(WorkflowFailure::Upstream(err_kind, _)) = &res {
        lease.fail_account(*err_kind).await;
        if let (Some(entry), Some(slot)) = (&job, gate.as_mut()) {
            **slot = None;
            entry.set_account_hint(None);
        }
    } else if let (Some(entry), Some(slot)) = (&job, gate.as_mut())
        && let Some(account_key) = lease.account_key()
    {
        **slot = Some(JobSession::new(
            account_key.to_string(),
            lease.session_id().to_string(),
            Arc::clone(lease.client()),
        ));
        entry.set_account_hint(Some(account_key.to_string()));
    }
    res
}

/// Result of one Muse chat turn: what the poll loop saw when the turn settled (or the deadline hit).
struct CollectedTurn {
    ready_candidates: Vec<ReadyMediaCandidate>,
    assistant_bubbles: Vec<String>,
}

async fn execute_media_turn_on_lease(
    lease: &mut VmOperationLease,
    batches: &[MediaBatch],
    images: &[ReferenceImage],
    kind: MediaKind,
    transparent_background: bool,
    timeout_duration: Duration,
) -> Result<LlmOutput, WorkflowFailure> {
    let mut event_rx = lease.client().subscribe_events();
    let deadline = Instant::now() + timeout_duration;

    let mut all_parts: Vec<MediaPart> = Vec::new();
    let mut messages: Vec<String> = Vec::new();
    for (index, batch) in batches.iter().enumerate() {
        let batch_images = if index == 0 { images } else { &[] };
        send_media_stream_request(lease, build_turn_items(batch_images, &batch.prompt)).await?;
        let outcome = run_media_batch(
            lease,
            &mut event_rx,
            kind,
            batch.target_secs,
            transparent_background,
            deadline,
        )
        .await;
        match outcome {
            Ok(BatchOutput::Image(output)) => return Ok(output),
            Ok(BatchOutput::Video(best)) => {
                all_parts.extend(best.parts);
                messages.extend(best.assistant_message);
            }
            // Earlier batches already produced footage: hand it back instead of discarding it.
            Err(WorkflowFailure::Timeout(_)) if !all_parts.is_empty() => break,
            Err(err) => return Err(err),
        }
    }

    lease.finish_clean(None);
    let message = (!messages.is_empty()).then(|| messages.join("\n\n"));
    LlmOutput::from_media_parts(all_parts, message).ok_or_else(|| {
        WorkflowFailure::Upstream(
            UpstreamFailureKind::Transient,
            "no video parts downloaded".to_string(),
        )
    })
}

fn build_turn_items(images: &[ReferenceImage], prompt: &str) -> Vec<Value> {
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
    items
}

enum BatchOutput {
    Image(LlmOutput),
    Video(BestVideo),
}

/// Waits for the batch just sent to settle, downloads its media and, for videos that come back
/// shorter than the batch target, asks Muse in the same session to fill the gap.
async fn run_media_batch(
    lease: &mut VmOperationLease,
    event_rx: &mut tokio::sync::broadcast::Receiver<VmStreamEvent>,
    kind: MediaKind,
    target_duration_secs: Option<u32>,
    transparent_background: bool,
    deadline: Instant,
) -> Result<BatchOutput, WorkflowFailure> {
    let mut repairs_used = 0usize;
    let mut best: Option<BestVideo> = None;
    loop {
        let turn = match collect_media_turn(lease, event_rx, kind, deadline).await {
            Ok(turn) => turn,
            Err(WorkflowFailure::Timeout(_)) if best.is_some() => break,
            Err(err) => return Err(err),
        };
        let selected = select_effective_ready_attachments(&turn.ready_candidates);
        if selected.is_empty() {
            if best.is_some() {
                break;
            }
            return Err(WorkflowFailure::Timeout(format!(
                "Muse {kind:?} generation timed out without producing any downloadable file path"
            )));
        }
        let assistant_message = format_clean_assistant_bubbles(&turn.assistant_bubbles);

        if matches!(kind, MediaKind::Image) {
            let output =
                finish_image(lease, &selected, transparent_background, assistant_message).await?;
            return Ok(BatchOutput::Image(output));
        }

        let parts = download_video_parts(lease, &selected).await?;
        let total_secs = total_duration_secs(&parts);
        let is_better = best
            .as_ref()
            .is_none_or(|b| total_secs.unwrap_or(0) >= b.total_secs.unwrap_or(0));
        if is_better {
            best = Some(BestVideo {
                parts,
                total_secs,
                assistant_message,
            });
        }

        let best_total = best.as_ref().and_then(|b| b.total_secs);
        let shortfall = video_shortfall_secs(target_duration_secs, best_total);
        if shortfall == 0 || repairs_used >= MAX_DURATION_REPAIRS || Instant::now() >= deadline {
            break;
        }
        repairs_used += 1;
        let followup = video_repair_prompt(
            target_duration_secs.unwrap_or_default(),
            best_total.unwrap_or_default(),
            shortfall,
        );
        send_media_stream_request(lease, vec![json!({ "type": "text", "text": followup })]).await?;
    }

    best.map(BatchOutput::Video).ok_or_else(|| {
        WorkflowFailure::Timeout(format!(
            "Muse {kind:?} generation timed out without producing any downloadable file path"
        ))
    })
}

struct BestVideo {
    parts: Vec<MediaPart>,
    total_secs: Option<u32>,
    assistant_message: Option<String>,
}

fn total_duration_secs(parts: &[MediaPart]) -> Option<u32> {
    parts.iter().try_fold(0u32, |acc, part| {
        orchestrator_flow_rs::inspect_mp4_duration_secs(&part.bytes)
            .map(|secs| acc.saturating_add(secs))
    })
}

/// Seconds missing from the delivered video; 0 when no target, unknown duration, or within tolerance.
fn video_shortfall_secs(target: Option<u32>, actual: Option<u32>) -> u32 {
    match (target, actual) {
        (Some(target), Some(actual))
            if actual.saturating_add(DURATION_SHORTFALL_TOLERANCE_SECS) < target =>
        {
            target - actual
        }
        _ => 0,
    }
}

fn video_repair_prompt(target: u32, actual: u32, missing: u32) -> String {
    format!(
        "MISSING_DURATION: the video you attached is only {actual}s but {target}s was requested ({missing}s missing). Do NOT ask questions and do NOT start over. Reuse the clips already generated in this chat, generate ONLY the missing beats, merge everything in order into ONE continuous video of ~{target}s, attach only that merged file, and end your message with {MEDIA_DONE_MARKER}."
    )
}

async fn download_video_parts(
    lease: &mut VmOperationLease,
    selected: &[ReadyMediaCandidate],
) -> Result<Vec<MediaPart>, WorkflowFailure> {
    let mut parts = Vec::with_capacity(selected.len());
    for item in selected {
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
    Ok(parts)
}

async fn finish_image(
    lease: &mut VmOperationLease,
    selected: &[ReadyMediaCandidate],
    transparent_background: bool,
    assistant_message: Option<String>,
) -> Result<LlmOutput, WorkflowFailure> {
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

/// Drives one chat turn to completion (or the deadline) and reports what Muse produced.
async fn collect_media_turn(
    lease: &mut VmOperationLease,
    event_rx: &mut tokio::sync::broadcast::Receiver<VmStreamEvent>,
    kind: MediaKind,
    deadline: Instant,
) -> Result<CollectedTurn, WorkflowFailure> {
    let mut poll_interval = tokio::time::interval(MEDIA_POLL_INTERVAL);
    poll_interval.tick().await;

    let mut stream_presentations: Vec<Value> = Vec::new();
    let mut streamed_text = String::new();
    let mut assistant_bubbles: Vec<String> = Vec::new();
    let mut ready_candidates: Vec<ReadyMediaCandidate> = Vec::new();
    let mut explicit_done = false;
    let mut latest_activity_code = String::new();
    let mut empty_completed_ticks: usize = 0;
    let mut candidate_ready_ticks: usize = 0;
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
                    } else if ev_name == "agent.status" {
                        if let Some(code) = ev.activity_code() {
                            latest_activity_code = code.to_string();
                        }
                        match ev.agent_activity_state() {
                            Some(true) => {
                                explicit_done = false;
                                empty_completed_ticks = 0;
                                candidate_ready_ticks = 0;
                            }
                            Some(false) => {
                                explicit_done = true;
                                should_poll_history = true;
                            }
                            None => {}
                        }
                    } else if ev.session_update_status(lease.session_id()) == Some("running") {
                        explicit_done = false;
                        empty_completed_ticks = 0;
                    } else if ev.is_session_completed_for(lease.session_id()) {
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

            let is_agent_working = active_tools
                || matches!(
                    latest_activity_code.as_str(),
                    "working" | "making_something" | "responding" | "thinking" | "running"
                );

            if !active_tools && !is_agent_working && !ready_candidates.is_empty() {
                candidate_ready_ticks += 1;
            } else {
                candidate_ready_ticks = 0;
            }

            let has_done_marker = assistant_bubbles
                .iter()
                .any(|b| b.contains(MEDIA_DONE_MARKER))
                || streamed_text.contains(MEDIA_DONE_MARKER);
            let media_turn_done = match kind {
                MediaKind::Image => true,
                MediaKind::Video => {
                    has_done_marker
                        || explicit_done
                        || is_assistant_completed
                        || (!is_agent_working && candidate_ready_ticks >= 6)
                }
            };

            if !active_tools && !ready_candidates.is_empty() && media_turn_done {
                break;
            }

            if !active_tools
                && !is_agent_working
                && ready_candidates.is_empty()
                && (explicit_done || is_assistant_completed)
                && !joined_raw.trim().is_empty()
            {
                empty_completed_ticks += 1;
            } else {
                empty_completed_ticks = 0;
            }

            // Mirror `muse-api-server-main/src/core/client.js:1448` (`requiredTicks = 12` = 4.8s):
            // Muse records `message.assistant` (status="completed") 0.7s-1.5s before `delta.presentation`
            // in `/chat/history`. Waiting 12 ticks before sending a follow-up nudge prevents appending a
            // new `message.user` during that gap and losing the presentation event.
            if empty_completed_ticks >= 12 && !nudged_attach {
                nudged_attach = true;
                explicit_done = false;
                empty_completed_ticks = 0;
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

    Ok(CollectedTurn {
        ready_candidates,
        assistant_bubbles,
    })
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
